//! Issue #475 follow-up — edge semantics of the startup-load pre-filter in
//! `rmqtt-session-storage` (`offline_session_is_expired`), complementing
//! `session_storage_expired_cleanup`:
//!
//! 1. **Default expiry = 0** — a V5 CONNECT without the session-expiry
//!    property decodes to 0 (codec `unwrap_or(0)`), so the session is
//!    *immediately* expired and must be dropped at load, never restored.
//! 2. **DISCONNECT property extends expiry** (connect 3s → disconnect 3600s):
//!    the pre-check takes `max(connect, disconnect)` as a loose upper bound
//!    and must NOT remove the still-live session.
//! 3. **DISCONNECT property shortens expiry** (connect 3600s → disconnect
//!    3s): the pre-check's `max` bound does not trip, but the rebuild pass
//!    (which honours the DISCONNECT value) must clean the session up — i.e.
//!    the pre-check may safely defer, never outlive the rebuild decision.
//!
//! One self-managed broker lifecycle covers all cases: five persistent
//! sessions are created, the broker is killed while the short ones are still
//! unexpired, then restarted after the downtime so they lapse. The startup
//! log must show only the two live sessions (`stored_session_infos len: 2`),
//! and reconnecting must yield the expected `session_present` per client.
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
//! (registered in the `chaos` suite; needs no harness broker)

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::tests::functional::cluster_session_restart::{rmqttd_binary, ClusterNode};

/// Broker MQTT port for this test (cleanup uses 1884, cluster 1886-1890,
/// auth-denied 1892, auth-jwt-denied 1893 → 1894 is free).
const TEST_ADDR: &str = "127.0.0.1:1894";
/// Broker gRPC port (cleanup uses 5364, cluster 5366-5370, others 5371-5372
/// → 5374 is free).
const TEST_RPC: &str = "0.0.0.0:5374";
const NODE_START_TIMEOUT: Duration = Duration::from_secs(20);
const CLIENT_IO_TIMEOUT: Duration = Duration::from_secs(10);
/// Time given to the broker to persist the disconnected sessions before kill.
///
/// Must stay well below SHORT_EXPIRY_SECS so the short sessions are still
/// present when the broker is killed. Also must exceed sled's flush
/// interval: the storage opens in `Mode::HighThroughput` (default ~500ms
/// periodic flush), and `ClusterNode::kill` terminates the process without
/// flushing, so any insert still sitting in sled's memory buffer is lost.
/// 2s covers several flush cycles; with 800ms the last-created sessions'
/// `BASIC` row regularly failed to survive the kill (load then reports
/// "offline session basic info is None" and drops them as garbage).
const PERSIST_WAIT: Duration = Duration::from_secs(2);
/// Expiry (seconds) of the sessions that must lapse while the broker is down.
const SHORT_EXPIRY_SECS: u32 = 3;
/// Expiry (seconds) of the sessions that must survive the restart.
const LONG_EXPIRY_SECS: u32 = 3600;
/// How long the broker stays down so the short-expiry sessions lapse
/// (must exceed SHORT_EXPIRY_SECS).
const DOWNTIME: Duration = Duration::from_secs(5);

/// Write a throwaway self-contained config (rmqtt.toml + plugins dir) under
/// `<workspace>/target/session-expired-cleanup-edge/` with a fresh sled path
/// and ports that cannot clash with other suites. Returns the path to
/// `rmqtt.toml`.
fn write_test_config() -> Result<PathBuf, anyhow::Error> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = root.join("target").join("session-expired-cleanup-edge");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("plugins"))?;

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
/// CONNECT session-expiry (seconds; `None` omits the property → default 0),
/// subscribe it to a unique topic and disconnect. When `disconnect_expiry` is
/// `Some`, the DISCONNECT packet carries that session-expiry property, which
/// overrides the CONNECT value for the ending session.
async fn create_session(
    addr: &str,
    cid: &str,
    expiry_secs: Option<u32>,
    disconnect_expiry: Option<u32>,
) -> Result<(), anyhow::Error> {
    let mut c = crate::mqtt::v5::MqttV5Client::connect_with_options(
        addr,
        cid,
        CLIENT_IO_TIMEOUT,
        false, // clean_start = false -> persistent session
        60,
        None,
        None,
        None,
        expiry_secs,
        None,
        None,
    )
    .await?;
    c.subscribe(&format!("edge/{cid}"), QoS::AtLeastOnce).await?;
    match disconnect_expiry {
        Some(secs) => c.disconnect_with_session_expiry(Some(secs)).await?,
        None => c.disconnect_with_session_expiry(None).await?,
    }
    Ok(())
}

/// Reconnect with the same identity and return whether the broker reported
/// `session_present`.
async fn reconnect_session_present(
    addr: &str,
    cid: &str,
    expiry_secs: Option<u32>,
) -> Result<bool, anyhow::Error> {
    let c = crate::mqtt::v5::MqttV5Client::connect_with_options(
        addr,
        cid,
        CLIENT_IO_TIMEOUT,
        false,
        60,
        None,
        None,
        None,
        expiry_secs,
        None,
        None,
    )
    .await?;
    let present = c.connack().session_present;
    let _ = c.disconnect().await;
    Ok(present)
}

async fn run_expired_cleanup_edge() -> Result<(), anyhow::Error> {
    let binary = rmqttd_binary();
    if !binary.exists() {
        return Err(anyhow::anyhow!(
            "rmqttd binary not found at {:?}; build it first (cargo build -p rmqttd)",
            binary
        ));
    }
    let config = write_test_config()?;
    let log_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target");
    let log_file = log_dir.join("session-expired-cleanup-edge-node.log");
    let mut node = ClusterNode::new(config, TEST_ADDR, log_file.clone());

    // ---- bring up the broker
    node.spawn(&binary)?;
    if !node.wait_healthy(NODE_START_TIMEOUT) {
        return Err(anyhow::anyhow!("broker did not become healthy (first start)"));
    }

    // ---- create the five persistent sessions.
    //
    // Order matters for the short-expiry ones: `edge-short` and
    // `edge-disc-dec` lapse 3s after *their* disconnect, so they must be
    // created last so the broker is killed while they are still unexpired.
    // The others (long or extended-by-DISCONNECT) cannot lapse while running.
    create_session(TEST_ADDR, "edge-live", Some(LONG_EXPIRY_SECS), None).await?;
    create_session(TEST_ADDR, "edge-default", None, None).await?;
    create_session(TEST_ADDR, "edge-disc-inc", Some(SHORT_EXPIRY_SECS), Some(LONG_EXPIRY_SECS)).await?;
    // connect 3600s, DISCONNECT shrinks to 3s -> must be cleaned by the
    // rebuild pass (the pre-check's `max` bound stays conservative)
    create_session(TEST_ADDR, "edge-disc-dec", Some(LONG_EXPIRY_SECS), Some(SHORT_EXPIRY_SECS)).await?;
    create_session(TEST_ADDR, "edge-short", Some(SHORT_EXPIRY_SECS), None).await?;
    // Give sled a moment to persist (disconnect handling is asynchronous).
    tokio::time::sleep(PERSIST_WAIT).await;

    // ---- kill the broker (TerminateProcess; sled is crash-safe) and let the
    // short-expiry sessions lapse while the broker is down
    node.kill();
    // Keep the first-start log for post-mortem (the restart below truncates
    // the file via File::create).
    if let Err(e) = std::fs::copy(&log_file, log_file.with_extension("first.log")) {
        tracing::warn!("failed to preserve first-start broker log: {e}");
    }
    tokio::time::sleep(DOWNTIME).await;

    // ---- restart: the expired sessions must be filtered at load time
    node.spawn(&binary)?;
    if !node.wait_healthy(NODE_START_TIMEOUT) {
        return Err(anyhow::anyhow!("broker did not become healthy (restart)"));
    }

    // Only the two live sessions (edge-live, edge-disc-inc) may survive:
    // edge-short/edge-default/edge-disc-dec must be gone after the load pass.
    let loaded = load_logged_session_count(&log_file)?;
    if loaded != 2 {
        return Err(anyhow::anyhow!(
            "expected 2 stored sessions after restart (3 expired ones pre-filtered), got {loaded}"
        ));
    }

    // ---- session_present per client must match the expected survival
    async fn check(cid: &str, expiry: Option<u32>, expect_present: bool) -> Result<(), anyhow::Error> {
        let present = reconnect_session_present(TEST_ADDR, cid, expiry).await?;
        if present != expect_present {
            return Err(anyhow::anyhow!("{cid}: expected session_present = {expect_present}, got {present}"));
        }
        Ok(())
    }
    check("edge-live", Some(LONG_EXPIRY_SECS), true).await?;
    check("edge-disc-inc", Some(SHORT_EXPIRY_SECS), true).await?;
    check("edge-short", Some(SHORT_EXPIRY_SECS), false).await?;
    check("edge-default", None, false).await?;
    check("edge-disc-dec", Some(LONG_EXPIRY_SECS), false).await?;

    Ok(())
}

/// Verify the edge semantics of the session-storage startup-load pre-filter:
/// default-expiry (0), DISCONNECT-extended (must survive) and
/// DISCONNECT-shortened (must be cleaned by the rebuild fallback).
pub struct SessionStorageExpiredCleanupEdgeTest;

impl TestCase for SessionStorageExpiredCleanupEdgeTest {
    fn name(&self) -> &str {
        "session_storage_expired_cleanup_edge"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(run_expired_cleanup_edge());
        match result {
            Ok(()) => TestResult::passed(self.name(), "chaos", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "chaos", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(120)
    }
}
