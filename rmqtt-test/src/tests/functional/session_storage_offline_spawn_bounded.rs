//! GitHub issue #495 / PR #499 reproduction — unbounded offline-message
//! persistence spawns in `rmqtt-session-storage`.
//!
//! The plugin's `OfflineMessageHandler` used to dispatch each offline
//! persistence write with a raw `tokio::spawn`: one detached task per routed
//! message per offline session, each owning a cloned payload. `push_limit`
//! only bounds what is *stored* (once the task runs), it does not bound the
//! backlog of tasks waiting to run — so when writes are spawned faster than
//! the storage backend drains them, the pending futures accumulate on the
//! heap and the broker is OOM-killed (measured in #495: 93 → 231 372 live
//! tokio tasks in 21 s, OOM at a 1 GiB limit after 48 s).
//!
//! Scenario (self-managed broker, sled-backed session storage):
//!
//!   1. 10 persistent sessions (`clean_start = false`, long expiry) all
//!      subscribe to the same flood topic and disconnect — each publish now
//!      fans out to 10 offline sessions → 10 persistence tasks per message.
//!   2. Sample the broker's `RssAnon` (baseline).
//!   3. A single publisher floods 20 000 QoS 0 messages of 1 KiB as fast as
//!      possible (~200 000 persistence spawns at ~3 KiB per pending task).
//!   4. Sample `RssAnon` again (peak of 3 samples 1 s apart).
//!   5. The anonymous-memory growth must stay below 100 MiB, and the broker
//!      must still serve a live pub/sub round trip afterwards.
//!
//! Pre-fix the backlog is unbounded: hundreds of MiB of anonymous memory →
//! the test FAILS, reproducing #495. Post-fix (PR #499) the writes go through
//! a bounded `TaskExecQueue` (10 000 pending ≈ 30 MiB) and excess writes are
//! discarded → the test PASSES as a regression guard.
//!
//! Cross-platform memory sampling (the closest equivalent of the #495
//! "anonymous memory" metric, which accounted for 99.9 % of the growth and is
//! independent of sled page cache):
//!
//! - Linux:   `RssAnon` from `/proc/<pid>/status`
//! - Windows: `PrivateMemorySize64` via `Get-Process` (private committed
//!   bytes ≈ anonymous memory)
//!
//! Other platforms are skipped. The 100 MiB threshold separates fixed
//! (~30 MiB) from unfixed (hundreds of MiB) with margin on both platforms.
//!
//! # How to run
//!
//! ```bash
//! cargo build -p rmqttd && cargo build -p rmqtt-test
//! ./target/debug/mqtt_harness --binary target/debug/rmqttd \
//!   --config rmqtt-test/configs/default/rmqtt.toml \
//!   --workspace . --suites chaos --workers 1
//! ```
//!
//! (registered in the `chaos` suite; it needs no harness broker)

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::broker::healthcheck::health_check_sync;
use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::tests::functional::cluster_session_restart::{rmqttd_binary, ClusterNode};

/// Broker MQTT port for this test (avoids harness 1883, expired-cleanup 1884,
/// cluster 1886-1890).
const TEST_ADDR: &str = "127.0.0.1:1885";
/// Broker gRPC port (avoids harness 5363, expired-cleanup 5364, cluster
/// 5366-5370).
const TEST_RPC: &str = "0.0.0.0:5365";
const NODE_START_TIMEOUT: Duration = Duration::from_secs(20);
const CLIENT_IO_TIMEOUT: Duration = Duration::from_secs(10);
/// Time given to the broker to persist the disconnected sessions before the
/// flood starts.
const PERSIST_WAIT: Duration = Duration::from_millis(1500);
/// Offline persistent sessions, all subscribed to the flood topic: every
/// routed publish fans out to all of them (one persistence task each).
const OFFLINE_SESSIONS: usize = 10;
/// QoS 0 messages published as fast as possible (~20 MiB total).
const FLOOD_MESSAGES: usize = 20_000;
/// Per-message payload size (with task overhead ≈ 3 KiB per pending
/// persistence task, matching the #495 measurements).
const PAYLOAD_BYTES: usize = 1024;
/// Topic every offline session subscribes to.
const FLOOD_TOPIC: &str = "flood/oom-repro";
/// Acceptable anonymous-memory growth. Post-fix the bounded exec queue caps
/// the backlog at 10 000 pending tasks (~30 MiB); pre-fix the same flood
/// leaves 150 000+ pending tasks (hundreds of MiB). 100 MiB separates the
/// two with margin on both sides.
const MAX_GROWTH_BYTES: u64 = 100 * 1024 * 1024;
/// Samples of anonymous/private memory taken 1 s apart after the flood; the
/// peak counts.
const PEAK_SAMPLES: usize = 3;

/// Write a throwaway self-contained config (rmqtt.toml + plugins dir) under
/// `<workspace>/target/session-offline-spawn-bounded/` with a fresh sled path
/// and ports that cannot clash with the harness (1883/5363), the
/// expired-cleanup test (1884/5364) or cluster tests (1886-1890/5366-5370).
fn write_test_config() -> Result<PathBuf, anyhow::Error> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = root.join("target").join("session-offline-spawn-bounded");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("plugins"))?;

    // TOML strings on Windows need `\` escaped; `/` works for dirs, and the
    // sled path keeps `{node}` as the storage plugin's node placeholder.
    let plugins_dir = dir.join("plugins").to_string_lossy().replace('\\', "/");
    let sled_path = dir.join("sled").join("session").join("{node}");
    let sled_toml = sled_path.to_string_lossy().replace('\\', "\\\\");

    let mut body = String::new();
    body.push_str("node.id = 1\n");
    body.push_str("\n## RPC\n");
    body.push_str(&format!("rpc.server_addr = \"{TEST_RPC}\"\n"));
    body.push_str("rpc.server_workers = 4\n");
    body.push_str("rpc.batch_size = 128\n");
    body.push_str("rpc.client_concurrency_limit = 128\n");
    body.push_str("rpc.client_timeout = \"5s\"\n");
    body.push_str("\n## Log\n");
    body.push_str("log.to = \"console\"\n");
    body.push_str("log.level = \"info\"\n");
    body.push_str("log.dir = \".\"\n");
    body.push_str("log.file = \"rmqtt.log\"\n");
    body.push_str("\n## Plugins\n");
    body.push_str(&format!("plugins.dir = \"{plugins_dir}\"\n"));
    body.push_str("plugins.default_startups = [\"rmqtt-session-storage\"]\n");
    body.push_str(
        "plugins.disabled_default_startups = [\"rmqtt-acl\", \"rmqtt-counter\", \"rmqtt-http-api\"]\n",
    );
    body.push_str("\n## MQTT listener (external TCP only; everything else disabled)\n");
    body.push_str(&format!("listener.tcp.external.addr = \"{TEST_ADDR}\"\n"));
    body.push_str("listener.tcp.external.workers = 8\n");
    body.push_str("listener.tcp.external.max_connections = 102400\n");
    body.push_str("listener.tcp.external.max_handshaking_limit = 500\n");
    body.push_str("listener.tcp.external.handshake_timeout = \"30s\"\n");
    body.push_str("listener.tcp.external.max_packet_size = \"1m\"\n");
    body.push_str("listener.tcp.external.backlog = 1024\n");
    body.push_str("listener.tcp.external.allow_anonymous = true\n");
    body.push_str("listener.tcp.external.min_keepalive = 0\n");
    body.push_str("listener.tcp.external.keepalive_backoff = 0.75\n");
    body.push_str("listener.tcp.external.max_inflight = 16\n");
    body.push_str("listener.tcp.external.max_mqueue_len = 1000\n");
    body.push_str("listener.tcp.external.mqueue_rate_limit = \"1000,1s\"\n");
    body.push_str("listener.tcp.external.max_clientid_len = 65535\n");
    body.push_str("listener.tcp.external.max_qos_allowed = 2\n");
    body.push_str("listener.tcp.external.max_topic_levels = 0\n");
    body.push_str("listener.tcp.external.session_expiry_interval = \"5m\"\n");
    body.push_str("listener.tcp.external.message_retry_interval = \"5s\"\n");
    body.push_str("listener.tcp.external.message_expiry_interval = \"5m\"\n");
    body.push_str("listener.tcp.external.max_subscriptions = 0\n");
    body.push_str("listener.tcp.external.shared_subscription = true\n");
    body.push_str("\n## Other listeners — all disabled (would clash with other suites)\n");
    body.push_str("listener.tcp.internal.enable = false\n");
    body.push_str("listener.tls.external.enable = false\n");
    body.push_str("listener.ws.external.enable = false\n");
    body.push_str("listener.wss.external.enable = false\n");
    body.push_str("listener.quic.external.enable = false\n");
    std::fs::write(dir.join("rmqtt.toml"), body)?;

    let plugin = format!(
        "storage.type = \"sled\"\nstorage.sled.path = \"{sled_toml}\"\nstorage.sled.cache_capacity = \"3G\"\n"
    );
    std::fs::write(dir.join("plugins").join("rmqtt-session-storage.toml"), plugin)?;

    Ok(dir.join("rmqtt.toml"))
}

/// Whether this platform has a memory-sampling implementation (used to skip
/// cleanly on unsupported platforms instead of failing).
fn memory_sampling_supported() -> bool {
    cfg!(any(target_os = "linux", target_os = "windows"))
}

/// Sample the broker process's anonymous/private memory — the closest
/// platform-equivalent of the #495 "anonymous memory" metric. Returns `None`
/// when sampling fails or the process is gone.
///
/// - Linux: `RssAnon` from `/proc/<pid>/status` (excludes sled page cache)
/// - Windows: `PrivateMemorySize64` via PowerShell `Get-Process` (private
///   committed bytes ≈ anonymous memory)
#[cfg(target_os = "linux")]
fn sample_anon_memory_bytes(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("RssAnon:") {
            let kb: u64 = rest.trim().trim_end_matches("kB").trim().parse().ok()?;
            return Some(kb * 1024);
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn sample_anon_memory_bytes(pid: u32) -> Option<u64> {
    // `-NoProfile -NonInteractive` keeps startup fast and side-effect free;
    // `PrivateMemorySize64` is private committed memory (≈ anonymous), the
    // closest analogue of Linux `RssAnon`. Fails when the process is gone.
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-NoLogo",
            "-Command",
            &format!("(Get-Process -Id {pid}).PrivateMemorySize64"),
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse::<u64>().ok()
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn sample_anon_memory_bytes(_pid: u32) -> Option<u64> {
    None
}

/// Create one persistent (`clean_start = false`) session subscribed to the
/// flood topic, then disconnect it — it becomes an offline session whose
/// every future routed message triggers an offline-persistence write.
async fn create_offline_session(addr: &str, cid: &str) -> Result<(), anyhow::Error> {
    let mut c = crate::mqtt::v5::MqttV5Client::connect_with_options(
        addr,
        cid,
        CLIENT_IO_TIMEOUT,
        false, // clean_start = false -> persistent session
        60,
        None,
        None,
        None,
        Some(3600), // session expiry far beyond the test duration
        None,
        None,
    )
    .await?;
    c.subscribe(FLOOD_TOPIC, QoS::AtLeastOnce).await?;
    c.disconnect().await?;
    Ok(())
}

/// Prove the broker is still serving after the flood: fresh clean client,
/// subscribe + publish + receive on a unique topic.
async fn live_round_trip() -> Result<(), anyhow::Error> {
    let uid = uuid::Uuid::new_v4().simple();
    let topic = format!("probe/{uid}");
    let mut c = crate::mqtt::v5::MqttV5Client::connect(
        TEST_ADDR,
        &format!("offline-flood-probe-{uid}"),
        CLIENT_IO_TIMEOUT,
    )
    .await?;
    c.subscribe(&topic, QoS::AtMostOnce).await?;
    c.publish(&topic, b"probe", QoS::AtMostOnce, false).await?;
    match c.recv_message_timeout(Duration::from_secs(5)).await {
        Some(_) => {
            let _ = c.disconnect().await;
            Ok(())
        }
        None => {
            let _ = c.disconnect().await;
            Err(anyhow::anyhow!("live round trip after flood: probe message not delivered"))
        }
    }
}

async fn run_offline_spawn_bounded() -> Result<(), anyhow::Error> {
    let binary = rmqttd_binary();
    if !binary.exists() {
        return Err(anyhow::anyhow!(
            "rmqttd binary not found at {:?}; build it first (cargo build -p rmqttd)",
            binary
        ));
    }
    let config = write_test_config()?;
    let log_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target");
    let log_file = log_dir.join("session-offline-spawn-bounded-node.log");
    let mut node = ClusterNode::new(config, TEST_ADDR, log_file.clone());

    // ---- bring up the broker (sled-backed session storage)
    node.spawn(&binary)?;
    if !node.wait_healthy(NODE_START_TIMEOUT) {
        return Err(anyhow::anyhow!("broker did not become healthy"));
    }

    // ---- create the offline sessions (all fanned out on the flood topic)
    for i in 0..OFFLINE_SESSIONS {
        create_offline_session(TEST_ADDR, &format!("offline-flood-{i:02}")).await?;
    }
    tokio::time::sleep(PERSIST_WAIT).await;

    // ---- baseline memory
    let pid = node.pid().ok_or_else(|| anyhow::anyhow!("broker pid unavailable"))?;
    let baseline = sample_anon_memory_bytes(pid)
        .ok_or_else(|| anyhow::anyhow!("cannot sample broker memory for pid {pid}"))?;

    // ---- flood: every message routes to OFFLINE_SESSIONS offline sessions,
    // spawning OFFLINE_SESSIONS persistence tasks (unbounded pre-fix)
    let pubc =
        crate::mqtt::v5::MqttV5Client::connect(TEST_ADDR, "offline-flood-publisher", CLIENT_IO_TIMEOUT)
            .await?;
    let payload = vec![b'x'; PAYLOAD_BYTES];
    for _ in 0..FLOOD_MESSAGES {
        pubc.publish(FLOOD_TOPIC, &payload, QoS::AtMostOnce, false).await?;
    }
    let _ = pubc.disconnect().await;

    // ---- settle briefly, then take the peak of a few samples 1 s apart
    let mut peak = baseline;
    for _ in 0..PEAK_SAMPLES {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if let Some(v) = sample_anon_memory_bytes(pid) {
            peak = peak.max(v);
        }
    }
    let growth_mib = peak.saturating_sub(baseline) as f64 / (1024.0 * 1024.0);

    // ---- the broker must have survived the flood and keep serving
    if !health_check_sync(TEST_ADDR, Duration::from_secs(5)) {
        return Err(anyhow::anyhow!(
            "broker not healthy after the flood (OOM-killed?) — issue #495 reproduced fatally"
        ));
    }
    live_round_trip().await?;

    // ---- the assertion: pending persistence tasks must stay bounded
    if peak.saturating_sub(baseline) > MAX_GROWTH_BYTES {
        return Err(anyhow::anyhow!(
            "broker anonymous/private memory grew by {growth_mib:.0} MiB during the offline flood \
             (baseline {} MiB, peak {} MiB) — persistence backlog is unbounded (#495); \
             expected < 100 MiB",
            baseline / (1024 * 1024),
            peak / (1024 * 1024)
        ));
    }
    Ok(())
}

/// Reproduce issue #495 (fixed by PR #499): offline-message persistence in
/// `rmqtt-session-storage` must be bounded — a flood of messages to offline
/// sessions must not grow the broker's anonymous memory unboundedly.
pub struct SessionStorageOfflineSpawnBoundedTest;

impl TestCase for SessionStorageOfflineSpawnBoundedTest {
    fn name(&self) -> &str {
        "session_storage_offline_spawn_bounded"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        if !memory_sampling_supported() {
            return TestResult::skipped(
                self.name(),
                "chaos",
                start.elapsed(),
                "no memory-sampling implementation on this platform \
                 (Linux /proc RssAnon, Windows Get-Process PrivateMemorySize64)",
            );
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(run_offline_spawn_bounded());
        match result {
            Ok(()) => TestResult::passed(self.name(), "chaos", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "chaos", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(180)
    }
}
