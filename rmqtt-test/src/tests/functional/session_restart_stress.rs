//! Issue #475 — STRESS tests: many persistent sessions × many messages × restart.
//!
//! The functional tests (`cluster_session_restart.rs` and
//! `chaos_broker_restart_session_routing`) prove the *behaviour* with a
//! single client and a single message. These stress tests push the same
//! scenario to the concurrency/volume dimension:
//!
//!   - `STRESS_SESSIONS` persistent (clean_start = false) sessions subscribe
//!     to unique topics on node 1 (or the standalone broker) and go offline;
//!   - the broker / cluster is restarted (single-node restart or whole
//!     cluster restart);
//!   - `STRESS_MSGS_PER_SESSION` QoS 1 messages are published to every topic
//!     (from node 2 in the cluster variants) while the sessions are offline;
//!   - all sessions reconnect WITHOUT re-subscribing and must receive every
//!     message (`session_present` must be 1).
//!
//! With the issue #475 fix the restored subscriptions are registered into
//! the router after `set`, so the offline messages are queued and delivered
//! on reconnect → all messages received → PASS. Without the fix the
//! publishes are silently dropped → received << expected → FAIL, reproducing
//! #475 at scale.
//!
//! # Coverage
//!
//! - `stress_single_node_restart_session_routing` — standalone broker
//!   (harness-managed, `session-sled-stress` config; its http-api on 6070 is
//!   enabled so the drop counter can be read)
//! - `stress_cluster_restart_session_routing_broadcast` — broadcast, node 1
//!   restart only
//! - `stress_cluster_whole_restart_session_routing_broadcast` — broadcast,
//!   whole cluster restart
//! - `stress_cluster_restart_session_routing_raft` — raft (3 nodes), node 1
//!   restart only
//! - `stress_cluster_whole_restart_session_routing_raft` — raft (3 nodes),
//!   whole cluster restart
//!
//! # How to run
//!
//! Prerequisites (Windows / Linux both fine):
//!
//!   - Build the broker and the harness:
//!     `cargo build -p rmqttd && cargo build -p rmqtt-test`
//!   - This repo requires rustc ≥ 1.94; if your default toolchain is older,
//!     prefix with `RUSTUP_TOOLCHAIN=1.97` (the version used during dev).
//!   - **Clean the sled data first** — the stress runs accumulate sessions in
//!     `rmqtt-test/configs/*/.sled/` and a large sled makes the broker take
//!     >20s to start (harness "broker failed to become healthy" errors).
//!
//!     The four cluster variants now clear their own store automatically
//!     (`reset_sled_data`, called before the nodes are spawned); the standalone
//!     variant cannot — its broker is started by the harness with
//!     `session-sled-stress` *before* the test body runs and keeps the sled
//!     open, so a dirty store there must still be removed by hand (the
//!     functional cluster tests in `cluster_session_restart.rs` too):
//!
//!     ```bash
//!     rm -rf rmqtt-test/configs/{session-sled,session-sled-stress,cluster-broadcast-sled,cluster-raft-sled}/.sled
//!     ```
//!   - Do not run another broker on the ports used by the suite (1883/1886/
//!     1887/1888/1889/1890 MQTT, 6060/6061/6070 http-api, 5363..5370 gRPC,
//!     6008..6010 raft), or while another harness instance is running.
//!
//! Run the whole chaos suite (functional restart tests + all 5 stress tests,
//! ~6.5 minutes at 1000×100):
//!
//!   ```
//!   ./target/debug/mqtt_harness --binary target/debug/rmqttd \
//!     --config rmqtt-test/configs/default/rmqtt.toml \
//!     --workspace . --suites chaos --workers 1
//!   ```
//!
//! Or only the standalone-broker stress test (`chaos@session-sled-stress`
//! sub-suite contains exactly `stress_single_node_restart_session_routing`,
//! ~25s):
//!
//!   ```
//!   ./target/debug/mqtt_harness --binary target/debug/rmqttd \
//!     --config rmqtt-test/configs/default/rmqtt.toml \
//!     --workspace . --suites chaos@session-sled-stress --workers 1
//!   ```
//!
//! The cluster stress tests (broadcast/raft, single-node & whole-cluster
//! restart) are self-managed processes and live in the main `chaos` suite —
//! there is no dedicated sub-suite for them, so they run as part of the
//! full `--suites chaos` run above. Per-node broker logs are written to
//! `target/cluster-stress-{broadcast,raft,...}-node{1,2,3}.log` and are the
//! first place to look when a stress run fails (e.g. the
//! `forwards_to failed ... TrySendError { Disconnected }` signature).
//!
//! Scale is controlled by `STRESS_SESSIONS` / `STRESS_MSGS_PER_SESSION`
//! (default 1000 sessions × 100 messages = 100k QoS 1 publishes). The
//! stress client's publishes are fire-and-forget (PUBACKs are not awaited),
//! so after `publish_all` returns the cross-node Forwards backlog can keep
//! trickling into node 1 for tens of seconds. Reconnecting while the
//! backlog is still in flight loses messages whose target session is being
//! taken over at that exact moment (inherent cross-node race without a
//! message store; see design doc §12) — the 2026-09-13 broadcast run lost
//! 2394/100000 this way after a fixed 30s wait was defeated by a slow
//! (debug-build) backlog. The cluster variants therefore use a
//! **deterministic drain wait**: node 1's http-api is enabled (port 6061)
//! and the harness polls the Prometheus `message_queues.count` gauge
//! (session deliver-queue push/pop counter) until it reaches
//! baseline + expected, i.e. every offline message is fully enqueued, and
//! only then reconnects the sessions. The baseline is sampled **before** the
//! publishes are issued and the gauge is monotonic while the sessions are
//! offline, so `baseline + expected` is reachable exactly when all 100k
//! messages of this run are in. Finally, every variant reports (as a test
//! *note*, never as a failure) the cumulative `messages.offline.saves.dropped`
//! metric: the number of offline-message persistence tasks the bounded
//! `rmqtt-session-storage` executor discarded — a green run must not hide that.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::tests::functional::cluster_session_restart::{
    cluster_probe, rmqttd_binary, ClusterNode, ClusterSpec, RestartMode,
};

/// Number of persistent sessions per stress run.
const STRESS_SESSIONS: usize = 1000;
/// QoS 1 messages published per session topic while offline.
const STRESS_MSGS_PER_SESSION: usize = 100;

const NODE_START_TIMEOUT: Duration = Duration::from_secs(20);
const POST_RESTART_SETTLE: Duration = Duration::from_millis(1500);
const CLUSTER_PROBE_TIMEOUT: Duration = Duration::from_secs(20);
/// Per-client connect timeout. 1000 clients connect concurrently, so the
/// broker handshake can take a while.
const CLIENT_IO_TIMEOUT: Duration = Duration::from_secs(10);
/// Per-message receive timeout. Offline delivery of 100k messages across
/// 1000 sessions is processed by the broker's exec queues, so a single
/// message may take a while to arrive under load.
const RECV_TIMEOUT: Duration = Duration::from_secs(120);
/// Time to let node 1 persist the 1000 offline sessions (sled batch writes).
const SESSION_PERSIST_WAIT: Duration = Duration::from_secs(5);
/// Publish concurrency. 1000 concurrent publisher connections × 100 QoS 1
/// messages hit the cluster broadcast Forwards path (node 2 -> node 1) with
/// 100k messages at once, which can drop a small fraction under load
/// (exec queue / gRPC timeout). Sharding the publishes keeps the test below
/// that cluster-forwarding bottleneck while still exercising ~100k messages.
const PUBLISH_CONCURRENCY: usize = 250;

/// Prometheus metrics path on node 1's http-api (port 6061 for the cluster
/// stress nodes, 6070 for the standalone one). The path shape is identical for
/// the `rmqtt_stats` and `rmqtt_metrics` families; only the `item` label
/// differs.
const NODE1_METRICS_PATH: &str = "/api/v1/metrics/prometheus/1";
/// `rmqtt_stats` item polled by the Forwards-drain wait.
const STATS_ITEM_MESSAGE_QUEUES: &str = "message_queues.count";
/// `rmqtt_metrics` item reported as the run's note: offline-message persistence
/// tasks dropped because `rmqtt-session-storage`'s bounded executor queue was
/// full (`OFFLINE_STORAGE_EXEC`). Cumulative, never decreases.
const METRICS_ITEM_OFFLINE_SAVES_DROPPED: &str = "messages.offline.saves.dropped";
/// http-api address of the harness-managed broker used by the standalone
/// stress case (port from `configs/session-sled-stress/plugins/rmqtt-http-api.toml`).
const STANDALONE_HTTP_ADDR: &str = "127.0.0.1:6070";
/// Poll interval for the drain wait.
const DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// Upper bound for the drain wait: the whole 100k-message cross-node
/// backlog was observed to drain in ~35s on a Windows debug build.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(300);
/// Settle time after the drain signal before reconnecting.
const DRAIN_SETTLE: Duration = Duration::from_secs(1);
/// Hard bound for each metrics HTTP GET (a misbehaving server must never
/// block the drain wait indefinitely).
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
/// Progress log interval for the drain wait (the raft backlog can drain
/// for minutes on a debug build — without progress output that is
/// indistinguishable from a hang).
const PROGRESS_INTERVAL: Duration = Duration::from_secs(5);
/// Plateau detection arms only after this much total wait time.
const PLATEAU_AFTER: Duration = Duration::from_secs(60);
/// Fail fast when the count has not changed for this long (after
/// [`PLATEAU_AFTER`]): an idle pipeline with the target unreachable means
/// messages were dropped before enqueueing — waiting longer cannot help.
const PLATEAU_WINDOW: Duration = Duration::from_secs(30);

/// Create `count` persistent sessions on `addr`, each subscribing to a unique
/// topic, then disconnect them (concurrently). Returns the client ids and
/// topics.
async fn setup_offline_sessions(
    addr: &str,
    uid: &str,
    count: usize,
) -> Result<(Vec<String>, Vec<String>), anyhow::Error> {
    let cids: Vec<String> = (0..count).map(|i| format!("stress-sub-{uid}-{i}")).collect();
    let topics: Vec<String> = (0..count).map(|i| format!("stress/{uid}/{i}")).collect();
    let results = futures::future::join_all((0..count).map(|i| {
        let cid = cids[i].clone();
        let topic = topics[i].clone();
        async move {
            let mut c = crate::mqtt::v5::MqttV5Client::connect_with_options(
                addr,
                &cid,
                CLIENT_IO_TIMEOUT,
                false, // clean_start = false -> persistent session
                60,
                None,
                None,
                None,
                Some(3600),
                None,
                None,
            )
            .await?;
            c.subscribe(&topic, QoS::AtLeastOnce).await?;
            c.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        }
    }))
    .await;
    for r in results {
        r?;
    }
    Ok((cids, topics))
}

/// Publish `msgs` QoS 1 messages to every topic in `topics` from `addr`.
/// One publisher connection per topic, `PUBLISH_CONCURRENCY` at a time
/// (1000 topics × 100 messages = 100k publishes would be far too slow
/// serially, but 1000 concurrent publishers saturate the cluster broadcast
/// forwarding path and drop a small fraction).
async fn publish_all(
    addr: &str,
    pub_id: &str,
    topics: &[String],
    msgs: usize,
    payload: &[u8],
) -> Result<(), anyhow::Error> {
    for (offset, chunk) in topics.chunks(PUBLISH_CONCURRENCY).enumerate() {
        let base = offset * PUBLISH_CONCURRENCY;
        let results = futures::future::join_all(chunk.iter().enumerate().map(|(i, topic)| {
            let topic = topic.clone();
            let cid = format!("{pub_id}-{}", base + i);
            async move {
                let pubc = crate::mqtt::v5::MqttV5Client::connect(addr, &cid, CLIENT_IO_TIMEOUT).await?;
                for _ in 0..msgs {
                    pubc.publish(&topic, payload, QoS::AtLeastOnce, false).await?;
                }
                pubc.disconnect().await?;
                Ok::<(), anyhow::Error>(())
            }
        }))
        .await;
        for r in results {
            r?;
        }
    }
    Ok(())
}

/// Minimal HTTP/1.1 GET (no external HTTP client dependency). Returns the
/// response body (head stripped). Hard-bounded by [`HTTP_TIMEOUT`] so a
/// misbehaving server can never block the drain wait indefinitely.
async fn http_get_body(addr: &str, path: &str) -> Result<String, anyhow::Error> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    tokio::time::timeout(HTTP_TIMEOUT, async {
        let mut stream = tokio::net::TcpStream::connect(addr).await?;
        let req = format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).await?;
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await?;
        let raw = String::from_utf8_lossy(&buf).into_owned();
        Ok::<String, anyhow::Error>(raw.split("\r\n\r\n").nth(1).map(|b| b.to_string()).unwrap_or(raw))
    })
    .await
    .map_err(|_| anyhow::anyhow!("HTTP GET {path} timed out after {HTTP_TIMEOUT:?}"))?
}

/// Extract one Prometheus item value from the text exposition. Works for both
/// families, only the label value differs:
/// `rmqtt_stats{node="1",item="message_queues.count"} 12345` and
/// `rmqtt_metrics{node="1",item="messages.offline.saves.dropped"} 0`.
fn parse_stat_item(body: &str, item: &str) -> Option<i64> {
    let needle = format!("item=\"{item}\"");
    for line in body.lines() {
        if line.contains(&needle) {
            return line.rsplit(' ').next()?.trim().parse().ok();
        }
    }
    None
}

/// Read one Prometheus item from `http_addr`'s node-1 exposition
/// (`None` when the endpoint is unreachable or the item is absent).
async fn read_prometheus_item(http_addr: &str, item: &str) -> Option<i64> {
    let body = http_get_body(http_addr, NODE1_METRICS_PATH).await.ok()?;
    parse_stat_item(&body, item)
}

/// Read node 1's `message_queues.count` gauge (0 when the endpoint is
/// unreachable or the value cannot be parsed).
///
/// The value is the **live** number of messages waiting in all sessions'
/// deliver queues on node 1 (`stats.message_queues`: `inc` on push, `dec` on
/// pop), so it has to be sampled *before* publishing to serve as the drain
/// baseline — see [`wait_forwards_enqueued`].
async fn read_message_queues_count(http_addr: &str) -> i64 {
    read_prometheus_item(http_addr, STATS_ITEM_MESSAGE_QUEUES).await.unwrap_or(0)
}

/// Observation note for the offline-persistence drop counter.
///
/// `subject` names the reporting broker (`"node 1"`, `"the standalone broker"`).
/// Returns `None` when the counter reads 0 — nothing worth reporting.
/// Otherwise a description, including the case where the counter could not be
/// read at all: a missing endpoint must not be able to masquerade as "no drops".
fn offline_drops_note(
    subject: &str,
    http_addr: &str,
    dropped: Option<i64>,
    broker_log: Option<&Path>,
) -> Option<String> {
    let metric = format!("{http_addr}{NODE1_METRICS_PATH} item={METRICS_ITEM_OFFLINE_SAVES_DROPPED}");
    match dropped {
        Some(0) => None,
        Some(n) => Some(format!(
            "{subject} dropped offline-message persistence for {n} message(s) under this burst \
             (bounded session-storage exec queue full; in-memory delivery unaffected, a further \
             restart would lose them) — metric {metric}{}",
            broker_log.map(|p| format!(", broker log {}", p.display())).unwrap_or_default()
        )),
        None => Some(format!(
            "{subject}: offline-persistence drop counter unavailable — item {metric} not readable \
             (http-api not started, or the broker was built without the `metrics` feature)"
        )),
    }
}

/// Deterministic Forwards-drain wait.
///
/// Blocks until node 1's `message_queues.count` gauge (session
/// deliver-queue push/pop counter) reaches `baseline + expected`, i.e.
/// every cross-node Forwards has been fully enqueued into its target
/// session's deliver queue. Reconnects must not start before this point:
/// in-flight Forwards hitting a session being taken over lose those
/// messages (see the module docs).
///
/// `baseline` must be sampled **before the publishes are issued**
/// ([`read_message_queues_count`]). It folds in whatever the queues already
/// hold — normally nothing, since the store is cleared per run; stale entries
/// restored from a dirty store otherwise — so that only *this* run's messages
/// count towards `target`. Sampling it after `publish_all` would fold most of
/// the 100k still-in-flight messages into the baseline too and push `target`
/// above anything the gauge can ever reach.
async fn wait_forwards_enqueued(http_addr: &str, baseline: i64, expected: i64) -> Result<(), anyhow::Error> {
    let target = baseline + expected;

    let start = Instant::now();
    let mut last_count: Option<i64> = None;
    let mut last_change = start;
    let mut last_progress_log = start;
    loop {
        if start.elapsed() > DRAIN_TIMEOUT {
            return Err(anyhow::anyhow!(
                "Forwards drain wait timed out after {DRAIN_TIMEOUT:?}: node 1 \
                 message_queues.count never reached {target} (last: {last_count:?})"
            ));
        }
        // Fast-fail on a plateau: with the pipeline idle but the target
        // unreachable, messages were dropped before enqueueing — waiting
        // the full timeout cannot change the outcome.
        if start.elapsed() > PLATEAU_AFTER && last_change.elapsed() > PLATEAU_WINDOW {
            if let Some(count) = last_count {
                return Err(anyhow::anyhow!(
                    "Forwards drain stalled: node 1 message_queues.count stuck at {count} \
                     (target {target}) for {PLATEAU_WINDOW:?} — messages were likely dropped \
                     before enqueueing (see node 1 log)"
                ));
            }
        }
        if let Ok(body) = http_get_body(http_addr, NODE1_METRICS_PATH).await {
            if let Some(count) = parse_stat_item(&body, STATS_ITEM_MESSAGE_QUEUES) {
                if count >= target {
                    break;
                }
                if last_count != Some(count) {
                    last_count = Some(count);
                    last_change = Instant::now();
                }
            }
        }
        // Progress output: a multi-minute raft backlog must be
        // distinguishable from a hang.
        if last_progress_log.elapsed() > PROGRESS_INTERVAL {
            eprintln!(
                "[drain] {:?}: message_queues.count = {} / {target}",
                start.elapsed(),
                last_count.unwrap_or(baseline)
            );
            last_progress_log = Instant::now();
        }
        tokio::time::sleep(DRAIN_POLL_INTERVAL).await;
    }
    tokio::time::sleep(DRAIN_SETTLE).await;
    Ok(())
}

/// Reconnect every session concurrently (no re-subscribe) and require that
/// each one receives exactly `msgs` messages. Fails when a session receives
/// nothing at all (the hallmark of issue #475: restored subscriptions not
/// routable → offline publishes dropped), otherwise reports the total
/// received vs expected.
async fn reconnect_and_verify(addr: &str, cids: &[String], msgs: usize) -> Result<(), anyhow::Error> {
    let results = futures::future::join_all(cids.iter().map(|cid| {
        let cid = cid.clone();
        async move {
            let mut c = crate::mqtt::v5::MqttV5Client::connect_with_options(
                addr,
                &cid,
                CLIENT_IO_TIMEOUT,
                false,
                60,
                None,
                None,
                None,
                Some(3600),
                None,
                None,
            )
            .await?;
            if !c.connack().session_present {
                let _ = c.disconnect().await;
                return Err(anyhow::anyhow!(
                    "session_present = 0 for {cid} after restart — sessions were NOT restored \
                     from sled (precondition of issue #475 violated)"
                ));
            }
            let mut got = 0usize;
            for _ in 0..msgs {
                if c.recv_message_timeout(RECV_TIMEOUT).await.is_some() {
                    got += 1;
                } else {
                    break;
                }
            }
            let _ = c.disconnect().await;
            if got == 0 {
                // Fast failure: this session received nothing — the restored
                // subscription is not routable (issue #475 at scale).
                Err(anyhow::anyhow!(
                    "stress: session {cid} received 0/{msgs} messages after restart — issue #475 \
                     reproduced at scale (restored subscriptions not routable, offline publishes \
                     dropped)"
                ))
            } else {
                Ok(got)
            }
        }
    }))
    .await;

    let mut received = 0usize;
    let mut failures = Vec::new();
    for (cid, r) in cids.iter().zip(results) {
        match r {
            Ok(got) => received += got,
            Err(e) => failures.push((cid.clone(), e.to_string())),
        }
    }
    if !failures.is_empty() {
        return Err(anyhow::anyhow!(
            "stress: {} of {} sessions failed ({})",
            failures.len(),
            cids.len(),
            failures[0].1
        ));
    }
    let expected = cids.len() * msgs;
    if received == expected {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "stress: received {received}/{expected} messages across {} sessions after restart \
             (some sessions partially delivered)",
            cids.len()
        ))
    }
}

/// Remove the sled session store belonging to `config_file` (if one exists).
///
/// Every stress run leaves its persistent sessions in the config's `.sled`
/// directory, and a run that fails before [`reconnect_and_verify`] also leaves
/// its whole offline backlog there. The next run then restores all of it, which
/// (a) makes the broker take longer to become healthy and (b) pre-populates
/// `message_queues.count` — the gauge the drain wait below is based on counts
/// the messages restored into the deliver queue at session rebuild
/// (`rmqtt-session-storage` `rebuild_sessions`). Starting each run from an
/// empty store keeps the startup time and the metric comparable between runs.
///
/// Only safe while no broker is running with that config: sled keeps its files
/// open (the delete fails on Windows) and a live node would keep writing into
/// the removed directory. The cluster variants own their nodes and kill them on
/// `ClusterNode::drop`, so calling this at the top of [`run_cluster_stress`] is
/// safe; the standalone variant cannot (its broker is harness-managed and
/// already running — see the module docs).
///
/// The store lives either at `<config_dir>/.sled` (standalone configs) or one
/// level up at `<config_dir>/../.sled` (per-node cluster configs), so both
/// layouts are probed.
fn reset_sled_data(config_file: &Path) -> Result<(), anyhow::Error> {
    let mut dir = match config_file.parent() {
        Some(dir) => dir,
        None => return Ok(()),
    };
    for _ in 0..2 {
        let sled = dir.join(".sled");
        if sled.is_dir() {
            std::fs::remove_dir_all(&sled)
                .map_err(|e| anyhow::anyhow!("failed to remove {}: {e}", sled.display()))?;
            eprintln!("[stress] cleared stale sled data: {}", sled.display());
            return Ok(());
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }
    Ok(())
}

/// Core cluster stress reproduction (see module docs).
///
/// `node1_http_addr` is node 1's http-api `host:port` (port 6061 in the
/// stress configs), used by the deterministic drain wait.
///
/// Returns `Ok(note)` on success, where `note` carries an observation that must
/// not change the verdict (currently: offline-message persistence dropped by
/// the bounded `rmqtt-session-storage` executor, read from the cumulative
/// `messages.offline.saves.dropped` metric) — so a green run reports it instead of
/// silently swallowing it.
async fn run_cluster_stress(
    cluster: &str,
    nodes: &[ClusterSpec],
    node1_http_addr: &str,
    mode: RestartMode,
) -> Result<Option<String>, anyhow::Error> {
    assert!(nodes.len() >= 2, "cluster stress needs at least 2 nodes");
    let node1_addr = &nodes[0].addr;
    let node2_addr = &nodes[1].addr;

    let binary = rmqttd_binary();
    if !binary.exists() {
        return Err(anyhow::anyhow!(
            "rmqttd binary not found at {:?}; build it first (cargo build -p rmqttd)",
            binary
        ));
    }

    let log_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target");
    let mut cluster_nodes: Vec<ClusterNode> = nodes
        .iter()
        .enumerate()
        .map(|(i, spec)| {
            ClusterNode::new(
                spec.config.clone(),
                &spec.addr,
                log_dir.join(format!("cluster-{cluster}-node{}.log", i + 1)),
            )
        })
        .collect();

    // ---- start from an empty session store (see `reset_sled_data`): sessions
    // left behind by an earlier run are restored on startup and their queue
    // entries stay in `message_queues.count` forever, which skews the drain
    // wait in phase 3.5. Must happen before the first node is spawned.
    for spec in nodes {
        reset_sled_data(&spec.config)?;
    }

    // ---- bring up all nodes and wait for the cluster to converge
    for (i, node) in cluster_nodes.iter_mut().enumerate() {
        node.spawn(&binary)?;
        if !node.wait_healthy(NODE_START_TIMEOUT) {
            return Err(anyhow::anyhow!("{cluster}: node {} did not become healthy", i + 1));
        }
    }
    let probe_topic = format!("cluster/{cluster}/probe/{}", uuid::Uuid::new_v4().simple());
    if let Err(e) = cluster_probe(node1_addr, node2_addr, &probe_topic, CLUSTER_PROBE_TIMEOUT).await {
        return Err(anyhow::anyhow!("{cluster}: cross-node probe failed before stress: {e}"));
    }

    // ---- Phase 1: persistent sessions on node 1, then offline
    let uid = uuid::Uuid::new_v4().simple().to_string();
    let (cids, topics) = setup_offline_sessions(node1_addr, &uid, STRESS_SESSIONS).await?;
    tokio::time::sleep(SESSION_PERSIST_WAIT).await;

    // ---- Phase 2: restart
    match mode {
        RestartMode::SingleNode => {
            cluster_nodes[0].kill();
            if nodes.len() >= 3 {
                // raft: wait for the remaining nodes to elect a new leader
                // before respawning node 1 (see cluster_session_restart.rs).
                tokio::time::sleep(Duration::from_secs(6)).await;
            }
            cluster_nodes[0].spawn(&binary)?;
            if !cluster_nodes[0].wait_healthy(NODE_START_TIMEOUT) {
                return Err(anyhow::anyhow!("{cluster}: node 1 did not recover after restart"));
            }
        }
        RestartMode::WholeCluster => {
            for node in &mut cluster_nodes {
                node.kill();
            }
            for (i, node) in cluster_nodes.iter_mut().enumerate() {
                node.spawn(&binary)?;
                if !node.wait_healthy(NODE_START_TIMEOUT) {
                    return Err(anyhow::anyhow!(
                        "{cluster}: node {} did not recover after cluster restart",
                        i + 1
                    ));
                }
            }
        }
    }
    tokio::time::sleep(POST_RESTART_SETTLE).await;
    if let Err(e) = cluster_probe(node1_addr, node2_addr, &probe_topic, CLUSTER_PROBE_TIMEOUT).await {
        return Err(anyhow::anyhow!("{cluster}: cross-node probe failed after restart: {e}"));
    }

    // ---- Phase 3: publish while sessions are offline (from node 2)
    let pub_id = format!("stress-pub-{uid}");
    // Baseline BEFORE publishing (see `wait_forwards_enqueued`): the gauge only
    // holds what was already queued — at this point nothing, unless a dirty
    // store was restored — so the drain target below is exactly "all 100k of
    // this run's messages are in".
    let baseline = read_message_queues_count(node1_http_addr).await;
    publish_all(node2_addr, &pub_id, &topics, STRESS_MSGS_PER_SESSION, b"stress-payload").await?;

    // ---- Phase 3.5: deterministic Forwards-drain wait. The publishes are
    // fire-and-forget, so `publish_all` returning only means the packets
    // were written to node 2; the cross-node backlog can keep trickling
    // into node 1 for tens of seconds. Reconnecting earlier lets the
    // still-queued Forwards hit sessions that were already removed by the
    // reconnect, so those in-flight messages are lost (inherent cross-node
    // race without a message store; `offline_run_loop`'s kick drain covers
    // only messages already in the session's own rx). Wait until every
    // message is fully enqueued instead of guessing with a fixed sleep.
    let expected = (STRESS_SESSIONS * STRESS_MSGS_PER_SESSION) as i64;
    wait_forwards_enqueued(node1_http_addr, baseline, expected).await?;

    // Observation, not an assertion: the bounded `rmqtt-session-storage`
    // executor drops offline-message persistence tasks when its queue
    // saturates. Delivery in this run is unaffected (the sessions are still
    // reachable in memory), but a *further* restart would lose those messages —
    // and until this note existed the drop was only visible indirectly, as a
    // shorter drain baseline, which clearing the store per run fixes (and
    // thereby hides). Read after the drain: by then every route has been
    // processed and the counter is final for this run.
    let drops = read_prometheus_item(node1_http_addr, METRICS_ITEM_OFFLINE_SAVES_DROPPED).await;
    let note = offline_drops_note("node 1", node1_http_addr, drops, Some(cluster_nodes[0].log_path()));

    // ---- Phase 4: reconnect all sessions on node 1 and verify delivery
    reconnect_and_verify(node1_addr, &cids, STRESS_MSGS_PER_SESSION).await?;
    Ok(note)
}

/// Map a stress run's outcome onto a verdict, attaching the run's observation
/// (if any) as the test `note` — reported in the console/JSON/HTML/detail
/// output without affecting pass/fail.
fn stress_result(
    test: &dyn TestCase,
    start: Instant,
    result: Result<Option<String>, anyhow::Error>,
) -> TestResult {
    match result {
        Ok(None) => TestResult::passed(test.name(), "chaos", start.elapsed()),
        Ok(Some(note)) => TestResult::passed_with_note(test.name(), "chaos", start.elapsed(), &note),
        Err(e) => TestResult::failed(test.name(), "chaos", start.elapsed(), e.to_string()),
    }
}

/// Standalone-broker stress: many sessions × messages × broker restart.
pub struct StressSingleNodeRestartTest;

impl TestCase for StressSingleNodeRestartTest {
    fn name(&self) -> &str {
        "stress_single_node_restart_session_routing"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        Some(crate::tests::config_path("session-sled-stress"))
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        if !ctx.has_broker() {
            return TestResult::skipped(
                self.name(),
                "chaos",
                start.elapsed(),
                "no broker managed by this context (--no-broker mode)",
            );
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let addr = &ctx.config.broker_addr;
            let (cids, topics) = setup_offline_sessions(addr, &uid, STRESS_SESSIONS).await?;
            tokio::time::sleep(SESSION_PERSIST_WAIT).await;

            ctx.restart_broker()?;
            if !ctx.broker_healthy() {
                tokio::time::sleep(Duration::from_secs(2)).await;
                if !ctx.broker_healthy() {
                    return Err(anyhow::anyhow!("broker not healthy after restart"));
                }
            }

            let pub_id = format!("stress-single-pub-{uid}");
            publish_all(addr, &pub_id, &topics, STRESS_MSGS_PER_SESSION, b"stress-payload").await?;
            tokio::time::sleep(Duration::from_millis(200)).await;

            reconnect_and_verify(addr, &cids, STRESS_MSGS_PER_SESSION).await?;
            // Single broker: no cross-node Forwards and no drain wait, but the
            // 100k messages are still routed into offline sessions here, so the
            // bounded persistence executor can saturate just as it does on node
            // 1 of the cluster variants — report it the same way (see
            // `offline_drops_note`). Read after the verification, when the
            // counter is final for this run.
            let drops = read_prometheus_item(STANDALONE_HTTP_ADDR, METRICS_ITEM_OFFLINE_SAVES_DROPPED).await;
            Ok(offline_drops_note("the standalone broker", STANDALONE_HTTP_ADDR, drops, None))
        });
        stress_result(self, start, result)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(900)
    }
}

/// cluster-broadcast, node 1 restart only.
pub struct StressClusterRestartBroadcastTest;

impl TestCase for StressClusterRestartBroadcastTest {
    fn name(&self) -> &str {
        "stress_cluster_restart_session_routing_broadcast"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            run_cluster_stress(
                "stress-broadcast",
                &[
                    ClusterSpec::new(
                        "127.0.0.1:1886",
                        crate::tests::config_path("cluster-broadcast-sled-stress/node1"),
                    ),
                    ClusterSpec::new(
                        "127.0.0.1:1887",
                        crate::tests::config_path("cluster-broadcast-sled-stress/node2"),
                    ),
                ],
                "127.0.0.1:6061",
                RestartMode::SingleNode,
            )
            .await
        });
        stress_result(self, start, result)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(900)
    }
}

/// cluster-broadcast, whole cluster restart.
pub struct StressClusterWholeRestartBroadcastTest;

impl TestCase for StressClusterWholeRestartBroadcastTest {
    fn name(&self) -> &str {
        "stress_cluster_whole_restart_session_routing_broadcast"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            run_cluster_stress(
                "stress-broadcast-whole",
                &[
                    ClusterSpec::new(
                        "127.0.0.1:1886",
                        crate::tests::config_path("cluster-broadcast-sled-stress/node1"),
                    ),
                    ClusterSpec::new(
                        "127.0.0.1:1887",
                        crate::tests::config_path("cluster-broadcast-sled-stress/node2"),
                    ),
                ],
                "127.0.0.1:6061",
                RestartMode::WholeCluster,
            )
            .await
        });
        stress_result(self, start, result)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(900)
    }
}

/// cluster-raft (3 nodes), node 1 restart only.
pub struct StressClusterRestartRaftTest;

impl TestCase for StressClusterRestartRaftTest {
    fn name(&self) -> &str {
        "stress_cluster_restart_session_routing_raft"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            run_cluster_stress(
                "stress-raft",
                &[
                    ClusterSpec::new(
                        "127.0.0.1:1888",
                        crate::tests::config_path("cluster-raft-sled-stress/node1"),
                    ),
                    ClusterSpec::new(
                        "127.0.0.1:1889",
                        crate::tests::config_path("cluster-raft-sled-stress/node2"),
                    ),
                    ClusterSpec::new(
                        "127.0.0.1:1890",
                        crate::tests::config_path("cluster-raft-sled-stress/node3"),
                    ),
                ],
                "127.0.0.1:6061",
                RestartMode::SingleNode,
            )
            .await
        });
        stress_result(self, start, result)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(900)
    }
}

/// cluster-raft (3 nodes), whole cluster restart.
pub struct StressClusterWholeRestartRaftTest;

impl TestCase for StressClusterWholeRestartRaftTest {
    fn name(&self) -> &str {
        "stress_cluster_whole_restart_session_routing_raft"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            run_cluster_stress(
                "stress-raft-whole",
                &[
                    ClusterSpec::new(
                        "127.0.0.1:1888",
                        crate::tests::config_path("cluster-raft-sled-stress/node1"),
                    ),
                    ClusterSpec::new(
                        "127.0.0.1:1889",
                        crate::tests::config_path("cluster-raft-sled-stress/node2"),
                    ),
                    ClusterSpec::new(
                        "127.0.0.1:1890",
                        crate::tests::config_path("cluster-raft-sled-stress/node3"),
                    ),
                ],
                "127.0.0.1:6061",
                RestartMode::WholeCluster,
            )
            .await
        });
        stress_result(self, start, result)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(900)
    }
}
