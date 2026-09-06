//! GitHub issue #475 — CLUSTER reproduction tests
//!
//! Single-node reproduction (`chaos_broker_restart_session_routing`) proves
//! that after a broker restart a session restored from sled storage has its
//! subscriptions in `peers` but **no route in the router**, so publishes
//! matching them are PUBACKed and silently dropped until the client
//! reconnects (see `designs/issue-475-restored-session-routing-fix.md`).
//!
//! These two tests exercise the same end-to-end behaviour through a cluster —
//! once with `rmqtt-cluster-broadcast` (two nodes), once with
//! `rmqtt-cluster-raft` (three nodes):
//!
//!   1. Subscriber A connects to **node 1** with clean_start = false,
//!      subscribes to T and disconnects; the session is persisted to node 1's
//!      sled storage.
//!   2. **Node 1 is restarted** (kill + respawn with the same config, so the
//!      sled path is unchanged); A's offline session is rebuilt from storage.
//!   3. Client B connects to **node 2** and publishes QoS 1 to T while A is
//!      still offline, and A reconnects to node 1 WITHOUT re-subscribing
//!      (session_present must be 1): the message must be delivered.
//!
//! The two cluster plugins take **different routes** to this guarantee, which
//! is why the *reproduction* semantics differ:
//!
//! - **broadcast**: `ClusterRouter::add` is purely local
//!   (`rmqtt-cluster-broadcast/src/router.rs`), so the #475 defect (rebuild
//!   never calls `router().add`) is NOT masked: without the fix the publish is
//!   dropped → the test FAILS, reproducing #475 (a control publish after the
//!   reconnect's session transfer is delivered, proving the subscription was
//!   restored).
//! - **raft**: `ClusterRouter::add` is sent as a **raft proposal**
//!   (`rmqtt-cluster-raft/src/router.rs` `mailbox.send_proposal`), so
//!   subscriptions are replicated through the raft state machine. When node 1
//!   (the configured leader) dies, nodes 2/3 elect a new leader, and the
//!   restarted node 1 re-joins and **restores the subscription routes from the
//!   leader's replicated state** — the defect is architecturally masked, so
//!   this test passes with or without the fix. It stays as a **behavioural
//!   regression** test: a persistent session's subscriptions must remain
//!   routable across a leader restart in a raft cluster.
//!
//! The nodes are spawned and killed by the test itself (self-contained, no
//! manual `--no-broker` setup). A cross-node publish probe is used after the
//! restart to wait for the cluster to converge (gRPC re-connect for
//! broadcast, leader election + log sync for raft) before running the
//! reproduction steps.
//!
//! # Configs
//!
//! - broadcast: `rmqtt-test/configs/cluster-broadcast-sled/node{1,2}/`
//!   (MQTT 1886/1887, gRPC 5366/5367)
//! - raft:      `rmqtt-test/configs/cluster-raft-sled/node{1,2,3}/`
//!   (MQTT 1888/1889/1890, gRPC 5368/5369/5370, raft 6008/6009/6010;
//!   `leader_id = 1` so nodes 2/3 wait for node 1 to lead)
//!
//! # Run
//!
//! Prerequisites: `cargo build -p rmqttd && cargo build -p rmqtt-test`
//! (this repo needs rustc ≥ 1.94; during dev we used
//! `RUSTUP_TOOLCHAIN=1.97`), and clean the sled data from previous runs
//! (see `session_restart_stress.rs` for the exact `rm -rf` of the
//! `configs/*/.sled/` directories — stale sessions make the broker start
//! slowly and the harness reports "broker failed to become healthy").
//!
//! These four cluster tests are registered in the `chaos` suite:
//!
//!   ```
//!   ./target/debug/mqtt_harness --binary target/debug/rmqttd \
//!     --config rmqtt-test/configs/default/rmqtt.toml \
//!     --workspace . --suites chaos --workers 1
//!   ```
//!
//! (This also runs the standalone restart test and the 5 stress tests; the
//! whole suite takes ~6.5 minutes at 1000×100 stress scale.)
//!
//! Per-node broker logs: `target/cluster-{broadcast,raft,...}-node{1,2,3}.log`.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::broker::healthcheck::health_check_sync;
use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Node startup / restart wait.
const NODE_START_TIMEOUT: Duration = Duration::from_secs(20);
/// Extra settle time after the restart before probing cluster convergence.
const POST_RESTART_SETTLE: Duration = Duration::from_millis(1500);
/// Cluster convergence probe timeout (cross-node publish must arrive).
const CLUSTER_PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// Static spec of one cluster node (MQTT address + config file).
pub(crate) struct ClusterSpec {
    pub(crate) addr: String,
    pub(crate) config: PathBuf,
}

impl ClusterSpec {
    pub(crate) fn new(addr: &str, config: PathBuf) -> Self {
        Self { addr: addr.to_string(), config }
    }
}

/// A cluster node process owned by the test. Killed on drop so an early
/// error return cannot leak a broker process (or keep its ports bound).
pub(crate) struct ClusterNode {
    child: Option<Child>,
    pub(crate) config: PathBuf,
    pub(crate) addr: String,
    log_file: PathBuf,
}

impl ClusterNode {
    pub(crate) fn new(config: PathBuf, addr: &str, log_file: PathBuf) -> Self {
        Self { child: None, config, addr: addr.to_string(), log_file }
    }

    pub(crate) fn spawn(&mut self, binary: &Path) -> Result<(), anyhow::Error> {
        if self.child.is_some() {
            return Ok(());
        }
        let stdout = File::create(&self.log_file)?;
        let stderr = stdout.try_clone()?;
        let child = Command::new(binary)
            .arg("-f")
            .arg(&self.config)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()?;
        self.child = Some(child);
        Ok(())
    }

    /// Block until the MQTT port accepts TCP connections.
    ///
    /// Fails FAST when the broker process exits before the port opens:
    /// waiting the full timeout against a dead process only masks the real
    /// cause (config/plugin errors), so the log tail is dumped immediately
    /// instead. The same diagnostics are dumped when the timeout expires.
    pub(crate) fn wait_healthy(&mut self, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            // Detect an early broker exit before probing the port.
            if let Some(child) = self.child.as_mut() {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        eprintln!(
                            "[cluster-node] broker on {} exited early (status: {status}); log tail:\n{}",
                            self.addr,
                            self.log_tail(30).unwrap_or_else(|| "<log file unreadable>".into())
                        );
                        return false;
                    }
                    Ok(None) => {} // still running
                    Err(e) => eprintln!("[cluster-node] try_wait failed: {e}"),
                }
            }
            if health_check_sync(&self.addr, Duration::from_secs(2)) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        eprintln!(
            "[cluster-node] broker on {} not healthy after {timeout:?}; log tail:\n{}",
            self.addr,
            self.log_tail(30).unwrap_or_else(|| "<log file unreadable>".into())
        );
        false
    }

    /// Read the last `lines` lines of the node's log file (for diagnostics).
    pub(crate) fn log_tail(&self, lines: usize) -> Option<String> {
        let content = std::fs::read_to_string(&self.log_file).ok()?;
        let collected: Vec<&str> = content.lines().collect();
        let start = collected.len().saturating_sub(lines);
        Some(collected[start..].join("\n"))
    }

    pub(crate) fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        // Give the OS time to release the ports.
        std::thread::sleep(Duration::from_millis(500));
    }

    /// Kill and respawn with the same config (identical sled path → offline
    /// sessions are rebuilt on startup).
    fn restart(&mut self, binary: &Path) -> Result<(), anyhow::Error> {
        self.kill();
        self.spawn(binary)
    }
}

impl Drop for ClusterNode {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Locate the rmqttd binary for the self-managed cluster nodes.
///
/// Prefers the **debug** build (so a `cargo build -p rmqttd` right before the
/// test picks up the freshest code — `BrokerProcess::find_binary` prefers
/// `target/release`, which is typically an older build during development),
/// then falls back to release, then to `BrokerProcess::find_binary`.
pub(crate) fn rmqttd_binary() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    for dir in ["target/debug", "target/release"] {
        for name in ["rmqttd.exe", "rmqttd"] {
            let p = root.join(dir).join(name);
            if p.exists() {
                return p;
            }
        }
    }
    crate::broker::BrokerProcess::find_binary(Some(&root))
}

/// Cross-node convergence probe: subscribe on node 1, publish QoS 0 on
/// node 2. Returns `Ok(())` when the message arrives, or an explanation of
/// which step failed (for diagnostics).
pub(crate) async fn cluster_probe(
    node1_addr: &str,
    node2_addr: &str,
    topic: &str,
    timeout: Duration,
) -> Result<(), String> {
    let uid = uuid::Uuid::new_v4().simple();
    let mut sub = match crate::mqtt::v5::MqttV5Client::connect(
        node1_addr,
        &format!("cluster-probe-sub-{uid}"),
        Duration::from_secs(5),
    )
    .await
    {
        Ok(c) => c,
        Err(e) => return Err(format!("probe: subscriber connect to {node1_addr} failed: {e}")),
    };
    if let Err(e) = sub.subscribe(topic, QoS::AtMostOnce).await {
        let _ = sub.disconnect().await;
        return Err(format!("probe: subscribe on {node1_addr} failed: {e}"));
    }
    let pubc = match crate::mqtt::v5::MqttV5Client::connect(
        node2_addr,
        &format!("cluster-probe-pub-{uid}"),
        Duration::from_secs(5),
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            let _ = sub.disconnect().await;
            return Err(format!("probe: publisher connect to {node2_addr} failed: {e}"));
        }
    };
    if let Err(e) = pubc.publish(topic, b"probe", QoS::AtMostOnce, false).await {
        let _ = sub.disconnect().await;
        let _ = pubc.disconnect().await;
        return Err(format!("probe: publish on {node2_addr} failed: {e}"));
    }
    let _ = pubc.disconnect().await;
    let got = sub.recv_message_timeout(timeout).await;
    let _ = sub.disconnect().await;
    if got.is_some() {
        Ok(())
    } else {
        Err(format!(
            "probe: publish on {node2_addr} not delivered to subscriber on {node1_addr} \
             within {timeout:?} (cross-node routing broken or cluster not converged)"
        ))
    }
}

/// How the cluster is restarted in phase 2.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RestartMode {
    /// Only node 1 is restarted; the other nodes keep the cluster alive.
    SingleNode,
    /// All nodes are stopped, then brought back up in order (node 1 first).
    WholeCluster,
}

/// Core cross-node reproduction of issue #475.
///
/// `nodes[0]` is the subscriber / restarted node; `nodes[1]` is the publisher
/// node; any additional nodes (raft: a 3rd member so the cluster keeps
/// quorum while node 1 is down) are started and kept alive but not used for
/// client traffic.
///
/// Restart strategies (kill + respawn with the same config → identical sled
/// path → offline sessions are rebuilt on startup):
///
/// - `SingleNode`: only node 1 is restarted. The broadcast variant is a
///   two-node cluster; the raft variant uses three nodes so that after
///   node 1 (the configured leader) dies, nodes 2/3 still form a quorum and
///   elect a new leader — the restarted node 1 then `join`s the new leader
///   instead of trying to `join` itself (the two-node raft restart defect).
/// - `WholeCluster`: every node is stopped, then brought back up in order
///   (node 1 first, so the raft leader comes up before its followers).
async fn run_cluster_session_routing_check(
    cluster: &str,
    nodes: &[ClusterSpec],
    mode: RestartMode,
) -> Result<(), anyhow::Error> {
    assert!(nodes.len() >= 2, "cluster reproduction needs at least 2 nodes");
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

    // ---- bring up all nodes and wait for the cluster to converge
    for (i, node) in cluster_nodes.iter_mut().enumerate() {
        node.spawn(&binary)?;
        if !node.wait_healthy(NODE_START_TIMEOUT) {
            return Err(anyhow::anyhow!("{cluster}: node {} did not become healthy", i + 1));
        }
    }
    let probe_topic = format!("cluster/{cluster}/probe/{}", uuid::Uuid::new_v4().simple());
    if let Err(e) = cluster_probe(node1_addr, node2_addr, &probe_topic, CLUSTER_PROBE_TIMEOUT).await {
        return Err(anyhow::anyhow!("{cluster}: cross-node probe failed before reproduction: {e}"));
    }

    // ---- Phase 1: persistent subscriber on node 1
    let uid = uuid::Uuid::new_v4().simple().to_string();
    let sub_id = format!("issue475-cluster-sub-{uid}");
    let pub_id = format!("issue475-cluster-pub-{uid}");
    let topic = format!("test/issue475/cluster/{cluster}/{uid}");
    let payload_before = b"msg-before-reconnect".as_slice();
    let payload_control = b"msg-after-transfer".as_slice();

    let mut a = crate::mqtt::v5::MqttV5Client::connect_with_options(
        node1_addr,
        &sub_id,
        Duration::from_secs(5),
        false, // clean_start = false -> persistent session
        60,
        None,
        None,
        None,
        Some(3600), // session expiry interval keeps the session alive
        None,
        None,
    )
    .await?;
    a.subscribe(&topic, QoS::AtLeastOnce).await?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    a.disconnect().await?;
    // Let node 1 persist the session (OfflineMessage hook).
    tokio::time::sleep(Duration::from_secs(1)).await;

    // ---- Phase 2: restart the cluster (same sled paths -> offline sessions
    // are rebuilt on startup).
    match mode {
        RestartMode::SingleNode => {
            cluster_nodes[0].kill();
            if nodes.len() >= 3 {
                // raft: after the configured leader (node 1) dies, the remaining
                // nodes need ~1-3s to elect a new leader. Node 1's startup probes
                // peers with only a few 500ms rounds, so if it restarts before the
                // election completes it can receive its own stale leader record and
                // try to `join` itself. Wait for the new leader to be established
                // before respawning node 1.
                tokio::time::sleep(Duration::from_secs(6)).await;
            }
            cluster_nodes[0].spawn(&binary)?;
            if !cluster_nodes[0].wait_healthy(NODE_START_TIMEOUT) {
                return Err(anyhow::anyhow!("{cluster}: node 1 did not recover after restart"));
            }
        }
        RestartMode::WholeCluster => {
            // Stop every node first, then bring them back up in order
            // (node 1 first: as the configured raft leader it must come up
            // before the followers can re-join).
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
    // Wait for the cluster to converge again (and the rebuild to settle).
    tokio::time::sleep(POST_RESTART_SETTLE).await;
    if let Err(e) = cluster_probe(node1_addr, node2_addr, &probe_topic, CLUSTER_PROBE_TIMEOUT).await {
        return Err(anyhow::anyhow!("{cluster}: cross-node probe failed after restart: {e}"));
    }

    // ---- Phase 3: B publishes QoS 1 to T on node 2 while A is offline
    let b = crate::mqtt::v5::MqttV5Client::connect(node2_addr, &pub_id, Duration::from_secs(5)).await?;
    b.publish(&topic, payload_before, QoS::AtLeastOnce, false).await?;
    b.disconnect().await?;
    // Give the cross-node forwards (node 2 -> node 1) time to be processed on
    // node 1 BEFORE A reconnects. With the bug node 1 has no route at that
    // point, so the publish is dropped; if A reconnected first, its session
    // transfer would register the route and mask the defect.
    tokio::time::sleep(Duration::from_secs(2)).await;

    // ---- Phase 4: A reconnects to node 1 WITHOUT re-subscribing
    let mut a2 = crate::mqtt::v5::MqttV5Client::connect_with_options(
        node1_addr,
        &sub_id,
        Duration::from_secs(5),
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
    if !a2.connack().session_present {
        let _ = a2.disconnect().await;
        return Err(anyhow::anyhow!(
            "{cluster}: session_present = 0 after node 1 restart — session was NOT restored \
             from sled (precondition of issue #475 violated)"
        ));
    }

    let first = a2.recv_message_timeout(Duration::from_secs(5)).await;
    match first {
        Some(m) if m.payload.as_ref() == payload_before => {
            // Cross-node publish delivered to the rebuilt offline session →
            // subscription routing works across restart (issue fixed).
            let _ = a2.disconnect().await;
            Ok(())
        }
        Some(m) => {
            let _ = a2.disconnect().await;
            Err(anyhow::anyhow!("{cluster}: unexpected payload: {:?}", m.payload))
        }
        None => {
            // Control probe: a further publish must now be delivered (the
            // reconnect's session transfer registered the route), proving the
            // subscription was restored and the first message was dropped.
            let b2 =
                crate::mqtt::v5::MqttV5Client::connect(node2_addr, &pub_id, Duration::from_secs(5)).await?;
            b2.publish(&topic, payload_control, QoS::AtLeastOnce, false).await?;
            tokio::time::sleep(Duration::from_millis(200)).await;
            b2.disconnect().await?;

            let control = a2.recv_message_timeout(Duration::from_secs(5)).await;
            let _ = a2.disconnect().await;
            match control {
                Some(m) if m.payload.as_ref() == payload_control => Err(anyhow::anyhow!(
                    "{cluster}: issue #475 reproduced — the cross-node publish (node 2 -> node 1) \
                     was PUBACKed but silently dropped (no router entry for the restored \
                     subscription on node 1); the control publish is only delivered after the \
                     reconnect's session transfer registered the route"
                )),
                Some(m) => Err(anyhow::anyhow!(
                    "{cluster}: first publish lost, control publish got unexpected payload: {:?}",
                    m.payload
                )),
                None => Err(anyhow::anyhow!(
                    "{cluster}: no message delivered even after session transfer — subscription \
                     was not restored at all (different failure mode than issue #475)"
                )),
            }
        }
    }
}

/// issue #475 cross-node reproduction with `rmqtt-cluster-broadcast`.
pub struct ClusterBroadcastRestartSessionRoutingTest;

impl TestCase for ClusterBroadcastRestartSessionRoutingTest {
    fn name(&self) -> &str {
        "cluster_restart_session_routing_broadcast"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            run_cluster_session_routing_check(
                "broadcast",
                &[
                    ClusterSpec::new(
                        "127.0.0.1:1886",
                        crate::tests::config_path("cluster-broadcast-sled/node1"),
                    ),
                    ClusterSpec::new(
                        "127.0.0.1:1887",
                        crate::tests::config_path("cluster-broadcast-sled/node2"),
                    ),
                ],
                RestartMode::SingleNode,
            )
            .await
        });
        match result {
            Ok(()) => TestResult::passed(self.name(), "chaos", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "chaos", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(180)
    }
}

/// issue #475 cross-node reproduction with `rmqtt-cluster-raft`.
pub struct ClusterRaftRestartSessionRoutingTest;

impl TestCase for ClusterRaftRestartSessionRoutingTest {
    fn name(&self) -> &str {
        "cluster_restart_session_routing_raft"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            run_cluster_session_routing_check(
                "raft",
                &[
                    ClusterSpec::new("127.0.0.1:1888", crate::tests::config_path("cluster-raft-sled/node1")),
                    ClusterSpec::new("127.0.0.1:1889", crate::tests::config_path("cluster-raft-sled/node2")),
                    ClusterSpec::new("127.0.0.1:1890", crate::tests::config_path("cluster-raft-sled/node3")),
                ],
                RestartMode::SingleNode,
            )
            .await
        });
        match result {
            Ok(()) => TestResult::passed(self.name(), "chaos", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "chaos", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(240)
    }
}

/// issue #475 whole-cluster restart reproduction with `rmqtt-cluster-broadcast`:
/// ALL nodes are stopped, then brought back up in order. The rebuilt offline
/// session on node 1 must remain routable (the defect drops cross-node
/// publishes until the session transfer is consumed, exactly as in the
/// single-node-restart variant).
pub struct ClusterWholeRestartSessionRoutingBroadcastTest;

impl TestCase for ClusterWholeRestartSessionRoutingBroadcastTest {
    fn name(&self) -> &str {
        "cluster_whole_restart_session_routing_broadcast"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            run_cluster_session_routing_check(
                "broadcast-whole",
                &[
                    ClusterSpec::new(
                        "127.0.0.1:1886",
                        crate::tests::config_path("cluster-broadcast-sled/node1"),
                    ),
                    ClusterSpec::new(
                        "127.0.0.1:1887",
                        crate::tests::config_path("cluster-broadcast-sled/node2"),
                    ),
                ],
                RestartMode::WholeCluster,
            )
            .await
        });
        match result {
            Ok(()) => TestResult::passed(self.name(), "chaos", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "chaos", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(180)
    }
}

/// issue #475 whole-cluster restart reproduction with `rmqtt-cluster-raft`
/// (three nodes): ALL nodes are stopped, then brought back up in order
/// (node 1 first as the configured leader). Unlike the single-node-restart
/// variant (where node 1 re-joins a new leader and restores subscription
/// routes from the raft state), a whole-cluster restart has node 1 come up
/// as the leader with no replicated state — so the rebuild MUST register the
/// restored subscriptions itself (this variant reproduces #475).
pub struct ClusterWholeRestartSessionRoutingRaftTest;

impl TestCase for ClusterWholeRestartSessionRoutingRaftTest {
    fn name(&self) -> &str {
        "cluster_whole_restart_session_routing_raft"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            run_cluster_session_routing_check(
                "raft-whole",
                &[
                    ClusterSpec::new("127.0.0.1:1888", crate::tests::config_path("cluster-raft-sled/node1")),
                    ClusterSpec::new("127.0.0.1:1889", crate::tests::config_path("cluster-raft-sled/node2")),
                    ClusterSpec::new("127.0.0.1:1890", crate::tests::config_path("cluster-raft-sled/node3")),
                ],
                RestartMode::WholeCluster,
            )
            .await
        });
        match result {
            Ok(()) => TestResult::passed(self.name(), "chaos", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "chaos", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(240)
    }
}
