//! Issue #475 follow-up — verify the startup-load optimization in
//! `rmqtt-session-storage`: expired offline sessions are pre-filtered
//! (skipped and removed) during `load_offline_session_infos` instead of
//! being fully loaded and only cleaned up by the rebuild pass.
//!
//! Scenario: one short-expiry persistent session (expires while the broker
//! is down) plus one long-expiry persistent session. After a broker restart
//! the load log must show only the live session (`stored_session_infos
//! len: 1`), and reconnecting must yield `session_present = 0` for the
//! expired session and `session_present = 1` for the live one.
//!
//! The broker is self-managed (like the cluster tests) because the harness
//! broker's stdout/stderr are piped and not persisted, while this test must
//! read the broker's startup log to assert the load result.
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
//! (the test is registered in the `chaos` suite; it needs no harness broker)

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::tests::functional::cluster_session_restart::{rmqttd_binary, ClusterNode};

/// Broker MQTT port for this test (avoids harness 1883 / cluster 1886-1890).
const TEST_ADDR: &str = "127.0.0.1:1884";
/// Broker gRPC port (avoids harness 5363 / cluster 5366-5370).
const TEST_RPC: &str = "0.0.0.0:5364";
const NODE_START_TIMEOUT: Duration = Duration::from_secs(20);
const CLIENT_IO_TIMEOUT: Duration = Duration::from_secs(10);
/// Time given to the broker to persist the disconnected sessions before kill.
const PERSIST_WAIT: Duration = Duration::from_millis(1500);
/// Expiry (seconds) of the session that must be skipped after restart.
const SHORT_EXPIRY_SECS: u32 = 2;
/// Expiry (seconds) of the session that must survive the restart.
const LONG_EXPIRY_SECS: u32 = 3600;
/// How long the broker stays down so the short-expiry session lapses.
const DOWNTIME: Duration = Duration::from_secs(4);

/// Write a throwaway self-contained config (rmqtt.toml + plugins dir) under
/// `<workspace>/target/session-expired-cleanup/` with a fresh sled path and
/// ports that cannot clash with the harness (1883/5363) or cluster tests
/// (1886-1890/5366-5370). Returns the path to `rmqtt.toml`.
fn write_test_config() -> Result<PathBuf, anyhow::Error> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = root.join("target").join("session-expired-cleanup");
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

/// Extract the session count logged by `load_offline_session_infos`
/// (`stored_session_infos len: N`) from the broker log file.
fn load_logged_session_count(log_file: &PathBuf) -> Result<usize, anyhow::Error> {
    let content = std::fs::read_to_string(log_file)?;
    for line in content.lines() {
        if let Some(idx) = line.find("stored_session_infos len: ") {
            let rest = &line[idx + "stored_session_infos len: ".len()..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                return Ok(digits.parse()?);
            }
        }
    }
    Err(anyhow::anyhow!("stored_session_infos len not found in broker log"))
}

/// Create one persistent (clean_start = false) session with the given
/// session-expiry (seconds), subscribe it to a unique topic and disconnect.
async fn create_session(addr: &str, cid: &str, expiry_secs: u32) -> Result<(), anyhow::Error> {
    let mut c = crate::mqtt::v5::MqttV5Client::connect_with_options(
        addr,
        cid,
        CLIENT_IO_TIMEOUT,
        false, // clean_start = false -> persistent session
        60,
        None,
        None,
        None,
        Some(expiry_secs),
        None,
        None,
    )
    .await?;
    c.subscribe(&format!("cleanup/{cid}"), QoS::AtLeastOnce).await?;
    c.disconnect().await?;
    Ok(())
}

async fn run_expired_cleanup() -> Result<(), anyhow::Error> {
    let binary = rmqttd_binary();
    if !binary.exists() {
        return Err(anyhow::anyhow!(
            "rmqttd binary not found at {:?}; build it first (cargo build -p rmqttd)",
            binary
        ));
    }
    let config = write_test_config()?;
    let log_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target");
    let log_file = log_dir.join("session-expired-cleanup-node.log");
    let mut node = ClusterNode::new(config, TEST_ADDR, log_file.clone());

    // ---- bring up the broker
    node.spawn(&binary)?;
    if !node.wait_healthy(NODE_START_TIMEOUT) {
        return Err(anyhow::anyhow!("broker did not become healthy (first start)"));
    }

    // ---- create one short-expiry and one long-expiry persistent session
    create_session(TEST_ADDR, "expired-cleanup-short", SHORT_EXPIRY_SECS).await?;
    create_session(TEST_ADDR, "expired-cleanup-live", LONG_EXPIRY_SECS).await?;
    // Give the sled a moment to persist both sessions (disconnect handling is
    // asynchronous). Must stay well below SHORT_EXPIRY_SECS so the short
    // session is still present when the broker is killed.
    tokio::time::sleep(PERSIST_WAIT).await;

    // ---- kill the broker (TerminateProcess; sled is crash-safe) and let the
    // short-expiry session lapse while the broker is down
    node.kill();
    tokio::time::sleep(DOWNTIME).await;

    // ---- restart: the expired session must be pre-filtered at load time
    node.spawn(&binary)?;
    if !node.wait_healthy(NODE_START_TIMEOUT) {
        return Err(anyhow::anyhow!("broker did not become healthy (restart)"));
    }

    // The load pass must have skipped the expired session: only the live one
    // should remain in `stored_session_infos` right after startup.
    let loaded = load_logged_session_count(&log_file)?;
    if loaded != 1 {
        return Err(anyhow::anyhow!(
            "expected 1 stored session after restart (expired session pre-filtered), got {loaded}"
        ));
    }

    // ---- the live session must be restored (session_present = 1)
    let live = crate::mqtt::v5::MqttV5Client::connect_with_options(
        TEST_ADDR,
        "expired-cleanup-live",
        CLIENT_IO_TIMEOUT,
        false,
        60,
        None,
        None,
        None,
        Some(LONG_EXPIRY_SECS),
        None,
        None,
    )
    .await?;
    if !live.connack().session_present {
        let _ = live.disconnect().await;
        return Err(anyhow::anyhow!("live session was NOT restored (session_present = 0)"));
    }
    let _ = live.disconnect().await;

    // ---- the expired session must NOT be restored (session_present = 0)
    let expired = crate::mqtt::v5::MqttV5Client::connect_with_options(
        TEST_ADDR,
        "expired-cleanup-short",
        CLIENT_IO_TIMEOUT,
        false,
        60,
        None,
        None,
        None,
        Some(SHORT_EXPIRY_SECS),
        None,
        None,
    )
    .await?;
    if expired.connack().session_present {
        let _ = expired.disconnect().await;
        return Err(anyhow::anyhow!(
            "expired session was restored (session_present = 1) — pre-filter did not remove it"
        ));
    }
    let _ = expired.disconnect().await;

    Ok(())
}

/// Verify the session-storage startup-load optimization: expired offline
/// sessions are skipped (and removed) during load, live sessions survive.
pub struct SessionStorageExpiredCleanupTest;

impl TestCase for SessionStorageExpiredCleanupTest {
    fn name(&self) -> &str {
        "session_storage_expired_cleanup"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(run_expired_cleanup());
        match result {
            Ok(()) => TestResult::passed(self.name(), "chaos", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "chaos", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(120)
    }
}
