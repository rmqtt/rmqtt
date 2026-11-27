//! Issue #532 — the connect-ramp stall in cluster form, with the ramp on **TLS**.
//!
//! Companion of [`super::issue532_cluster_connect_ramp_stall`], which carries
//! the full rationale for the workload, the detector, the fixture shape and the
//! deliberate deviations from the reporter's deployment. This case is that
//! experiment with one variable changed: the handshake ramp arrives on each
//! node's **TLS** listener instead of its plain TCP one.
//!
//! ## Why the transport is not a detail
//!
//! `rmqtt/src/server.rs::listen_tls` performs the TLS handshake inside a
//! per-connection `tokio::spawn`, i.e. **outside** the per-port
//! `TaskExecQueue` that `max_handshaking_limit` sizes; only the MQTT handshake
//! that follows goes through that queue. TLS therefore adds CPU work (rustls +
//! certificate verification) at unbounded concurrency right next to the
//! queue's dispatcher task — the one variable this case isolates. The
//! reporter's deployment ran TLS; the plain-TCP case is the control.
//!
//! ## Fixture
//!
//! `configs/issue532-ramp-tls-cluster/`: each node has **two** listeners —
//! TLS (1909/1910/1911) for the ramp and plain TCP (1912/1913/1914) for the
//! fan-out publishers, the cross-node subscriber and the convergence probe. The
//! fan-out stays on plain TCP deliberately: `MqttV5Client` is plain-only, and
//! keeping it off the TLS listener leaves the ramp as the *only* TLS workload,
//! so "the TLS handshake path is what changed" stays true. gRPC ports are
//! 5386/5387/5388; certificates are the repository's own test pair
//! (`rmqtt-bin/rmqtt.pem` + `rmqtt.key`, server-auth only).
//!
//! ## Per-node rate profile
//!
//! Beyond the sibling case's hard stall criterion, this case records every
//! node's accepted-handshake rate once per second and reports the profile, so a
//! *degradation* that never becomes a full stall shows up in the evidence
//! instead of passing silently.
//!
//! ## Running it
//!
//! The case owns its nodes, so it needs `--no-broker` and is excluded from the
//! default full run (`main.rs::should_run`):
//!
//! ```text
//! cargo build -p rmqttd && cargo build -p rmqtt-test
//! target/debug/mqtt_harness --no-broker --workspace . --suites functional_v5_cluster --workers 1 -t issue532_tls
//! ```
//!
//! Nodes already listening on the fixture ports are reused instead of spawned
//! (the two-terminal debugging flow); logs land in
//! `target/cluster-issue532-tls-nodeN.log`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::broker::healthcheck::health_check_sync;
use crate::framework::context::TestContext;
use crate::framework::testcase::{Expectation, TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v5::MqttV5Client;
use crate::tests::functional::cluster_session_restart::{cluster_probe, rmqttd_binary, ClusterNode};
use crate::tests::functional::issue532_connect_ramp_stall::{
    handshake, rate_sampler, tls_client_config, Counters, RateStats, Transport, CA_PEM,
};

/// Suite this case is reported under: the self-managed cluster suite, excluded
/// from the default full run and requiring `--no-broker`.
const SUITE: &str = "functional_v5_cluster";

/// MQTT/TLS addresses of `configs/issue532-ramp-tls-cluster/node{1,2,3}/rmqtt.toml`.
/// Must match `listener.tls.external.addr` there, and the gRPC ports in those
/// files must match `node_grpc_addrs` in the shared broadcast plugin config.
pub(crate) const NODE_ADDRS: [&str; 3] = ["127.0.0.1:1909", "127.0.0.1:1910", "127.0.0.1:1911"];
/// Plain-TCP addresses of the same nodes (`listener.tcp.external.addr`), used by the
/// fan-out publishers, the cross-node subscriber and the convergence probe:
/// `MqttV5Client` and `cluster_probe` are plain-only, and keeping them here leaves the
/// TLS listener carrying the ramp and nothing else.
pub(crate) const PLAIN_ADDRS: [&str; 3] = ["127.0.0.1:1912", "127.0.0.1:1913", "127.0.0.1:1914"];
/// Labels used in the evidence, one per node.
const NODE_LABELS: [&str; 3] = ["node1", "node2", "node3"];

/// Node start budget (config/plugin errors fail fast inside `wait_healthy`).
const NODE_START_TIMEOUT: Duration = Duration::from_secs(30);
/// Cross-node convergence probe budget, before the ramp starts.
const CLUSTER_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Topic the fan-out load publishes to and the cross-node subscriber reads.
const FANOUT_TOPIC: &str = "issue532/fanout";

/// Concurrent connectors **per node** (three nodes, so three times this in
/// total). High enough to keep every node's two-worker handshake pool
/// saturated; low enough that one development machine is not itself the
/// bottleneck — a CPU-starved test host would be indistinguishable from the
/// defect under test.
const RAMP_CLIENTS_PER_NODE: usize = 128;
/// Long-lived publishers per node (each publishes back-to-back).
const PUBLISHERS_PER_NODE: usize = 8;
/// Grace period before the watchdog starts, so cluster bring-up and the first
/// connections are not mistaken for a stall.
const WARMUP: Duration = Duration::from_secs(3);
/// Ramp length. Longer than the single-node case: a stall here has to be
/// detected per node, and the cascade (which node went first, whether the
/// survivors followed) only shows up if the ramp keeps running.
const RAMP_DURATION: Duration = Duration::from_secs(90);
/// Budget for one handshake attempt (TCP connect + CONNECT + CONNACK).
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(3);
/// Publisher connect budget.
const PUBLISH_TIMEOUT: Duration = Duration::from_secs(5);
/// Watchdog cycle: probes all three nodes, one after another, then sleeps.
const PROBE_INTERVAL: Duration = Duration::from_millis(250);
/// Budget for one watchdog probe.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// Consecutive failures against the same node that count as a stall. Every
/// cycle probes all three nodes, so one hung node makes a cycle cost about
/// `PROBE_TIMEOUT` while the others stay fast — 4 of them is roughly 8-10 s of
/// that node completing nothing at all.
const STALL_PROBES: u32 = 4;

/// The cluster reproduction of issue #532: three broadcast nodes under a
/// connect ramp plus fan-out load; fails if any node stops completing
/// handshakes while its process stays alive.
pub struct Issue532TlsClusterConnectRampStallTest;

impl TestCase for Issue532TlsClusterConnectRampStallTest {
    fn name(&self) -> &str {
        "issue532_tls_cluster_connect_ramp_stall"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        // The cluster is owned by this case (or reused), never by the harness:
        // a harness-managed broker would bind the shared ports and stand in for
        // one of the nodes. Same guard as the other self-managed cluster case.
        if ctx.has_broker() {
            return TestResult::skipped(
                self.name(),
                SUITE,
                start.elapsed(),
                "requires --no-broker: this case spawns its own three-node cluster. Run: \
                 mqtt_harness --no-broker --workspace . --suites functional_v5_cluster --workers 1 \
                 -t issue532",
            );
        }

        // Keep the nodes alive for the whole case: self-spawned ones are killed
        // (and their ports released) when this vec is dropped.
        let nodes = match acquire_cluster() {
            Ok(nodes) => nodes,
            Err(e) => return TestResult::failed(self.name(), SUITE, start.elapsed(), e),
        };

        // One TLS client config for the whole case: a fresh `ClientConfig` per
        // connection would add certificate parsing to every attempt and blur the
        // very cost this case measures.
        let transport = match tls_client_config(CA_PEM) {
            Ok(cfg) => Transport::Tls(cfg),
            Err(e) => {
                return TestResult::error(
                    self.name(),
                    SUITE,
                    start.elapsed(),
                    format!("could not build the TLS client config from {CA_PEM}: {e}"),
                )
            }
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let (stalls, evidence) = rt.block_on(async move {
            // The cluster must be converged before the ramp means anything: a
            // node that cannot see its peers would fail for a different reason
            // than the one under test.
            let topic = format!("issue532/cluster/probe/{}", uuid::Uuid::new_v4().simple());
            if let Err(e) = cluster_probe(PLAIN_ADDRS[0], PLAIN_ADDRS[1], &topic, CLUSTER_PROBE_TIMEOUT).await
            {
                return (Vec::new(), Err(format!("cluster did not converge before the ramp: {e}")));
            }

            let load = Load::new(transport);
            let stalls = run_cluster_ramp(&load).await;
            let evidence = load.evidence(&stalls);
            (stalls, Ok(evidence))
        });

        let elapsed = start.elapsed();
        let evidence = match evidence {
            Ok(evidence) => evidence,
            Err(e) => {
                // Bring-up/convergence failure is an infrastructure problem, not
                // a reproduction: report it as an Error so it is not mistaken
                // for either outcome of the case.
                let logs = log_tails(&nodes);
                return TestResult::error(self.name(), SUITE, elapsed, format!("{e}. {logs}"));
            }
        };

        if stalls.is_empty() {
            let text = format!(
                "no node stalled during {:.1}s of cluster ramp; {evidence}; {}",
                elapsed.as_secs_f64(),
                log_tails(&nodes)
            );
            tracing::info!("{text}");
            return TestResult::passed_with_note(self.name(), SUITE, elapsed, &text);
        }

        let order = stalls
            .iter()
            .map(|s| {
                format!(
                    "{} at +{:.1}s (window {:.1}s, {} failed / {} ok probes, last error: {})",
                    s.label,
                    s.at.duration_since(start).as_secs_f64(),
                    s.window.as_secs_f64(),
                    s.probe_failures,
                    s.probes_ok,
                    s.last_error
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        let reason = format!(
            "cluster member(s) stopped completing handshakes while their processes stayed alive \
             (issue #532): {} of {} nodes stalled — {order}. {evidence}; {}",
            stalls.len(),
            NODE_ADDRS.len(),
            log_tails(&nodes)
        );
        tracing::warn!("{reason}");
        TestResult::failed(self.name(), SUITE, elapsed, reason)
    }

    fn timeout(&self) -> Duration {
        // Cluster bring-up + convergence probe + ramp + drain + evidence.
        Duration::from_secs(300)
    }

    /// Assertive: the case exists to reproduce issue #532, so any confirmed
    /// stall must fail the suite. Switch to `Expectation::ExpectedFail` if the
    /// suite is ever required to stay green while the defect is open.
    fn expectation(&self) -> Expectation {
        Expectation::Normal
    }
}

/// Everything the workload needs, plus the per-node counters the watchdog and
/// the evidence read.
struct Load {
    /// Early-exit switch every worker checks. Nothing sets it during this case
    /// on purpose — the reported incident is a cascade, so the ramp has to keep
    /// running after the first node stalls — but the workers honour it, so an
    /// abort can be added without touching them.
    stop: Arc<AtomicBool>,
    deadline: Instant,
    /// Handshake counters, one per node — the watchdog's per-node progress
    /// signal.
    ramp: Vec<Arc<Counters>>,
    /// Publisher counters, one per node (sends / failures / reconnects).
    publishers: Vec<Arc<Counters>>,
    /// Messages the cross-node subscriber received.
    delivered: Arc<AtomicU64>,
    /// How the handshake ramp reaches every node: TLS here, plain TCP in the
    /// sibling case.
    transport: Transport,
    /// Accepted-handshake rate profile, one per node: `rate_sampler` appends one
    /// bucket per node per second, so a degradation that never becomes a full
    /// stall is still visible in the evidence.
    profiles: Vec<Arc<parking_lot::Mutex<Vec<u64>>>>,
}

impl Load {
    fn new(transport: Transport) -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            deadline: Instant::now() + WARMUP + RAMP_DURATION,
            ramp: (0..NODE_ADDRS.len()).map(|_| Arc::new(Counters::default())).collect(),
            publishers: (0..NODE_ADDRS.len()).map(|_| Arc::new(Counters::default())).collect(),
            delivered: Arc::new(AtomicU64::new(0)),
            transport,
            profiles: (0..NODE_ADDRS.len()).map(|_| Arc::new(parking_lot::Mutex::new(Vec::new()))).collect(),
        }
    }

    /// Per-node counters and the cross-node delivery count — also the
    /// amplification evidence (whether the survivors kept working after the
    /// first stall).
    fn evidence(&self, stalls: &[NodeStall]) -> String {
        let per_node = (0..NODE_ADDRS.len())
            .map(|i| {
                format!(
                    "{}: {} connects ({} accepted, {} failed) | publish: {} sends ({} failed, {} connects) \
                     | ramp rate: {}",
                    NODE_LABELS[i],
                    self.ramp[i].attempts.load(Ordering::Relaxed),
                    self.ramp[i].accepted.load(Ordering::Relaxed),
                    self.ramp[i].errors.load(Ordering::Relaxed),
                    self.publishers[i].attempts.load(Ordering::Relaxed),
                    self.publishers[i].errors.load(Ordering::Relaxed),
                    self.publishers[i].accepted.load(Ordering::Relaxed),
                    RateStats::of(self.profiles[i].lock().as_slice()).describe(),
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        let stalled = if stalls.is_empty() {
            "none".to_string()
        } else {
            stalls.iter().map(|s| s.label).collect::<Vec<_>>().join(",")
        };
        format!(
            "cross-node deliveries on {}: {}; stalled: [{stalled}]; {per_node}",
            NODE_LABELS[NODE_ADDRS.len() - 1],
            self.delivered.load(Ordering::Relaxed),
        )
    }
}

/// One node's stall, as reported.
struct NodeStall {
    label: &'static str,
    at: Instant,
    window: Duration,
    probes_ok: u64,
    probe_failures: u64,
    last_error: String,
}

/// Per-node watchdog state. Only the watchdog task touches it.
struct NodeState {
    label: &'static str,
    addr: &'static str,
    ramp: Arc<Counters>,
    probes_ok: u64,
    probes_failed: u64,
    streak: u32,
    window_start: Instant,
    accepted_at_window_start: u64,
    last_error: String,
    stalled: bool,
}

/// Spawns the three fixture nodes, reusing any that already listen.
///
/// Returns the owned nodes (self-spawned ones are killed on drop). `Err`
/// carries a diagnosis; the caller turns it into a failed verdict. A node that
/// is already running is never killed, so the manual two-terminal flow works.
fn acquire_cluster() -> Result<Vec<ClusterNode>, String> {
    let binary = rmqttd_binary();
    if !binary.exists() {
        return Err(format!("rmqttd binary not found at {binary:?}; build it first (cargo build -p rmqttd)"));
    }

    let log_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target");
    let mut nodes: Vec<ClusterNode> = (0..NODE_ADDRS.len())
        .map(|i| {
            ClusterNode::new(
                node_config(i + 1),
                NODE_ADDRS[i],
                log_dir.join(format!("cluster-issue532-tls-node{}.log", i + 1)),
            )
        })
        .collect();

    for (i, node) in nodes.iter_mut().enumerate() {
        if health_check_sync(&node.addr, Duration::from_secs(2)) {
            tracing::info!(
                "issue532 cluster: reusing the externally started node already listening on {}",
                node.addr
            );
            continue;
        }
        if let Err(e) = node.spawn(&binary) {
            return Err(format!("failed to spawn {} ({}): {e}", NODE_LABELS[i], node.addr));
        }
        // `wait_healthy` dumps the node's log tail when it exits early or never
        // opens the port, so the reason is visible without digging.
        if !node.wait_healthy(NODE_START_TIMEOUT) {
            return Err(format!(
                "{} ({}) did not become healthy within {NODE_START_TIMEOUT:?} \
                 (log: target/cluster-issue532-tls-node{}.log)",
                NODE_LABELS[i],
                node.addr,
                i + 1
            ));
        }
    }
    Ok(nodes)
}

/// Path of one node's config inside the fixture.
fn node_config(node: usize) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("configs")
        .join("issue532-ramp-tls-cluster")
        .join(format!("node{node}"))
        .join("rmqtt.toml")
}

/// Drives the handshake ramp, the fan-out publishers, the cross-node
/// subscriber and the watchdog; returns every confirmed stall in the order the
/// nodes went down.
async fn run_cluster_ramp(load: &Load) -> Vec<NodeStall> {
    let mut tasks = Vec::new();

    // Handshake ramp: worker `w` connects to node `w % NODE_ADDRS.len()`.
    for w in 0..RAMP_CLIENTS_PER_NODE * NODE_ADDRS.len() {
        let node = w % NODE_ADDRS.len();
        tasks.push(tokio::spawn(ramp_worker(
            node,
            w,
            load.deadline,
            load.stop.clone(),
            load.ramp[node].clone(),
            load.transport.clone(),
        )));
    }

    // Per-node rate profile, one sampler per node: a dip that never becomes a
    // full stall is exactly what the hard criterion cannot see.
    for node in 0..NODE_ADDRS.len() {
        tasks.push(tokio::spawn(rate_sampler(
            load.deadline,
            load.stop.clone(),
            load.ramp[node].clone(),
            load.profiles[node].clone(),
        )));
    }

    // Fan-out load: a batch of long-lived publishers per node.
    for node in 0..NODE_ADDRS.len() {
        for i in 0..PUBLISHERS_PER_NODE {
            tasks.push(tokio::spawn(publisher(
                node,
                i,
                load.deadline,
                load.stop.clone(),
                load.publishers[node].clone(),
            )));
        }
    }

    // Cross-node consumer: subscribing on the last node means publishes on the
    // other two have to cross the cluster to arrive.
    tasks.push(tokio::spawn(subscriber(
        NODE_ADDRS.len() - 1,
        load.deadline,
        load.stop.clone(),
        load.delivered.clone(),
    )));

    let mut states: Vec<NodeState> = (0..NODE_ADDRS.len())
        .map(|i| NodeState {
            label: NODE_LABELS[i],
            addr: NODE_ADDRS[i],
            ramp: load.ramp[i].clone(),
            probes_ok: 0,
            probes_failed: 0,
            streak: 0,
            window_start: Instant::now(),
            accepted_at_window_start: 0,
            last_error: String::new(),
            stalled: false,
        })
        .collect();
    let stalls = watchdog(load, &mut states).await;

    // Everything checks `stop`/`deadline`, so this join is bounded; nothing is
    // left running when it returns.
    for task in tasks {
        let _ = task.await;
    }
    stalls
}

/// Reconnects to `NODE_ADDRS[node]` as fast as the broker lets it.
async fn ramp_worker(
    node: usize,
    worker: usize,
    deadline: Instant,
    stop: Arc<AtomicBool>,
    counters: Arc<Counters>,
    transport: Transport,
) {
    let mut seq = 0u64;
    while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
        seq += 1;
        let client_id = format!("i532c-{worker}-{seq}");
        counters.attempts.fetch_add(1, Ordering::Relaxed);
        match handshake(NODE_ADDRS[node], &transport, &client_id, ATTEMPT_TIMEOUT, ATTEMPT_TIMEOUT).await {
            Ok(rmqtt_codec::v5::ConnectAckReason::Success) => {
                counters.accepted.fetch_add(1, Ordering::Relaxed);
            }
            Ok(other) => {
                counters.refused.fetch_add(1, Ordering::Relaxed);
                counters.note(format!("unexpected CONNACK {other:?}"));
            }
            Err(e) => {
                counters.errors.fetch_add(1, Ordering::Relaxed);
                counters.note(e.to_string());
            }
        }
    }
}

/// Holds one v5 connection open and publishes QoS 0 back-to-back, so every
/// publish fans out to the other two nodes. Reconnects with a short backoff
/// when the connection fails, and stops at `deadline`.
async fn publisher(
    node: usize,
    idx: usize,
    deadline: Instant,
    stop: Arc<AtomicBool>,
    counters: Arc<Counters>,
) {
    let client_id = format!("i532c-pub-{node}-{idx}");
    let payload = vec![0x5Au8; 64];
    while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
        let Ok(client) = MqttV5Client::connect(PLAIN_ADDRS[node], &client_id, PUBLISH_TIMEOUT).await else {
            counters.errors.fetch_add(1, Ordering::Relaxed);
            counters.note("publisher connect failed".to_string());
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        };
        counters.accepted.fetch_add(1, Ordering::Relaxed);
        while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
            counters.attempts.fetch_add(1, Ordering::Relaxed);
            if let Err(e) = client.publish(FANOUT_TOPIC, &payload, QoS::AtMostOnce, false).await {
                counters.errors.fetch_add(1, Ordering::Relaxed);
                counters.note(e.to_string());
                break;
            }
        }
        let _ = client.disconnect().await;
    }
}

/// Subscribes on `NODE_ADDRS[node]` and counts what the other nodes' publishers
/// manage to deliver — a number for "did the cluster still route anything?".
async fn subscriber(node: usize, deadline: Instant, stop: Arc<AtomicBool>, delivered: Arc<AtomicU64>) {
    let client_id = format!("i532c-sub-{node}");
    let Ok(mut client) = MqttV5Client::connect(PLAIN_ADDRS[node], &client_id, PUBLISH_TIMEOUT).await else {
        return;
    };
    if client.subscribe(FANOUT_TOPIC, QoS::AtMostOnce).await.is_err() {
        return;
    }
    while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
        if client.recv_message_timeout(Duration::from_millis(200)).await.is_some() {
            delivered.fetch_add(1, Ordering::Relaxed);
        }
    }
    let _ = client.disconnect().await;
}

/// Probes every node once per cycle and confirms a stall when, for one node,
/// `STALL_PROBES` consecutive probes failed, that node's ramp counter did not
/// move over the same window, and it had served handshakes before the window.
async fn watchdog(load: &Load, states: &mut [NodeState]) -> Vec<NodeStall> {
    tokio::time::sleep(WARMUP).await;
    let mut stalls = Vec::new();
    let mut seq = 0u64;

    while Instant::now() < load.deadline {
        tokio::time::sleep(PROBE_INTERVAL).await;
        for state in states.iter_mut() {
            seq += 1;
            let client_id = format!("i532c-probe-{seq}");
            let failed = match handshake(
                state.addr,
                &load.transport,
                &client_id,
                PROBE_TIMEOUT,
                PROBE_TIMEOUT,
            )
            .await
            {
                Ok(rmqtt_codec::v5::ConnectAckReason::Success) => {
                    state.probes_ok += 1;
                    None
                }
                Ok(other) => {
                    state.probes_failed += 1;
                    Some(format!("unexpected CONNACK {other:?}"))
                }
                Err(e) => {
                    state.probes_failed += 1;
                    Some(e.to_string())
                }
            };

            match failed {
                None => state.streak = 0,
                Some(err) => {
                    if state.streak == 0 {
                        state.window_start = Instant::now();
                        state.accepted_at_window_start = state.ramp.accepted.load(Ordering::Relaxed);
                    }
                    state.streak += 1;
                    state.last_error = err;
                }
            }

            if state.streak >= STALL_PROBES && !state.stalled {
                let frozen = state.ramp.accepted.load(Ordering::Relaxed) == state.accepted_at_window_start;
                if frozen && state.accepted_at_window_start > 0 {
                    state.stalled = true;
                    stalls.push(NodeStall {
                        label: state.label,
                        at: Instant::now(),
                        window: state.window_start.elapsed(),
                        probes_ok: state.probes_ok,
                        probe_failures: state.probes_failed,
                        last_error: state.last_error.clone(),
                    });
                } else {
                    // Either the node still completes handshakes (slow, not
                    // stalled) or it never completed one (reported separately).
                    state.streak = 0;
                }
            }
        }
    }
    stalls
}

/// Last lines of every node's log, with how long ago each was written; a frozen
/// tail is the "its log ends" symptom of the report, while a log written in the
/// last seconds points at test-host starvation (three brokers and the ramp share
/// one machine) rather than at the defect.
fn log_tails(nodes: &[ClusterNode]) -> String {
    nodes
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let tail = node.log_tail(4).unwrap_or_else(|| "<log unreadable>".to_string());
            let last = tail.lines().last().unwrap_or("<empty>").trim().to_string();
            let age = std::fs::metadata(node.log_path())
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|m| std::time::SystemTime::now().duration_since(m).ok())
                .map(|d| format!("{}s ago", d.as_secs()))
                .unwrap_or_else(|| "age unknown".to_string());
            format!("{} log ({}), last write {age}: {last}", NODE_LABELS[i], node.log_path().display())
        })
        .collect::<Vec<_>>()
        .join(" | ")
}
