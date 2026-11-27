//! QoS 2 PUBREL resume packet-id collision — CLUSTER reproduction test
//!
//! Same bug as `qos2_pubrel_resume_collision`, but exercised through the
//! cluster broadcast path, which is the realistic end-to-end trigger:
//!
//! A message published on node B and forwarded to a subscriber on node A is
//! **stored on the publishing node (B)** and delivered on A without being
//! `mark_forwarded`-ed on B (`shared.rs::forwards` only marks recipients that
//! were matched on the publishing node itself, `from_node_id == node.id()`).
//!
//! So when the subscriber's persistent session (with incomplete QoS 2
//! exchanges) resumes **on node B**, `send_storaged_messages` reloads the
//! un-forwarded stored message and races with the owed PUBREL re-sends
//! (`send_rerelease`) — both can be assigned the same packet-id and the
//! stored message's registration is silently overwritten.
//!
//! Observable symptom: the client receives **two PUBREL with the same packet
//! id** (one from `send_rerelease`, one from the PUBREC-response path acting
//! on the overwritten entry).
//!
//! # Setup
//!
//! Two rmqttd nodes (`rmqtt-test/configs/pubrel-collision-cluster/`):
//! - node 1: 127.0.0.1:1884 (MQTT), 5364 (gRPC)
//! - node 2: 127.0.0.1:1885 (MQTT), 5365 (gRPC)
//!
//! The test owns those two processes: a node whose address already accepts
//! TCP connections is **reused** (the original manual two-terminal flow),
//! a missing one is **spawned** from its config and killed on exit. Either
//! way the addresses above are what the reproduction talks to — never
//! `ctx.config.broker_addr`.
//!
//! # Run
//!
//! ```
//! ./target/debug/mqtt_harness --no-broker --addr 127.0.0.1:1884 \
//!   --workspace . --suites functional_v5_cluster --workers 1
//! ```
//!
//! `--no-broker` is mandatory: a harness-managed broker binds the default
//! listener ports that node 1 also binds, and it would silently stand in for
//! node 1 while node 2 stays missing. Running without it reports SKIPPED
//! (with the command above) instead of failing on a bare `os error 10061`.
//!
//! Per-node broker logs: `target/pubrel-collision-cluster-node{1,2}.log`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::broker::healthcheck::health_check_sync;
use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v5::MqttV5Client;

use super::cluster_session_restart::{rmqttd_binary, ClusterNode};

/// Node 1 MQTT address (the node the harness must be pointed at).
const NODE1_ADDR: &str = "127.0.0.1:1884";

/// Node 2 MQTT address (the node the session migrates to).
const NODE2_ADDR: &str = "127.0.0.1:1885";

/// Per-node configs (relative to the repository root).
const NODE1_CONFIG: &str = "rmqtt-test/configs/pubrel-collision-cluster/node1/rmqtt.toml";
const NODE2_CONFIG: &str = "rmqtt-test/configs/pubrel-collision-cluster/node2/rmqtt.toml";

/// Suite tag used for the reported verdict (`functional_v5_cluster`).
const SUITE: &str = "functional_v5_cluster";

/// How long a self-spawned node may take to accept TCP connections.
const NODE_START_TIMEOUT: Duration = Duration::from_secs(20);

/// Connect timeout for the reproduction clients.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Number of incomplete QoS 2 exchanges left in the session before resume.
/// Larger values lengthen the reforward loop (each entry awaits the expiry
/// check hook), giving the async stored-message load more time to enqueue
/// first — which is required for the packet-id collision.
const UNCOMPLETE_COUNT: usize = 16;

/// Number of stored messages published on node 2 while the subscriber is offline.
/// Multiple messages load in parallel (one spawn per message batch), further
/// widening the race window.
const STORED_COUNT: usize = 2;

/// Reproduction rounds.
const ROUNDS: usize = 3;

/// Cluster reproduction: duplicate PUBREL on cross-node session resume.
pub struct Qos2PubrelResumeCollisionClusterTest;

/// Bring up the two-node cluster, reusing nodes that are already listening.
///
/// Returns the owned nodes (a self-spawned one is killed on drop) and whether
/// anything had to be started. `Err` carries a diagnosis with the node log
/// path; the caller turns it into a failed verdict. An already-running node is
/// never killed, so the manual two-terminal flow still works.
fn acquire_cluster() -> Result<(Vec<ClusterNode>, bool), String> {
    let binary = rmqttd_binary();
    if !binary.exists() {
        return Err(format!("rmqttd binary not found at {binary:?}; build it first (cargo build -p rmqttd)"));
    }

    // Logs live next to the other self-managed cluster logs, i.e. in target/.
    let log_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target");
    let mut nodes = vec![
        ClusterNode::new(
            PathBuf::from(NODE1_CONFIG),
            NODE1_ADDR,
            log_dir.join("pubrel-collision-cluster-node1.log"),
        ),
        ClusterNode::new(
            PathBuf::from(NODE2_CONFIG),
            NODE2_ADDR,
            log_dir.join("pubrel-collision-cluster-node2.log"),
        ),
    ];

    let mut spawned = false;
    for (i, node) in nodes.iter_mut().enumerate() {
        if health_check_sync(&node.addr, Duration::from_secs(2)) {
            tracing::info!(
                "pubrel-collision-cluster: reusing the externally started node already listening on {}",
                node.addr
            );
            continue;
        }
        if let Err(e) = node.spawn(&binary) {
            return Err(format!("failed to spawn node {} ({}): {e}", i + 1, node.addr));
        }
        // `wait_healthy` prints the node's log tail when it exits early or
        // never opens the port, so the reason is visible without digging.
        if !node.wait_healthy(NODE_START_TIMEOUT) {
            return Err(format!(
                "node {} ({}) did not become healthy within {NODE_START_TIMEOUT:?} \
                 (log: target/pubrel-collision-cluster-node{}.log)",
                i + 1,
                node.addr,
                i + 1
            ));
        }
        spawned = true;
    }
    Ok((nodes, spawned))
}

impl TestCase for Qos2PubrelResumeCollisionClusterTest {
    fn name(&self) -> &str {
        "qos2_pubrel_resume_collision_cluster"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        // The cluster is owned by this test (or reused), never by the harness:
        // a harness-managed broker binds the same default listeners as node 1
        // and would silently take node 1's place in `--addr`, leaving node 2
        // missing — the failure mode that reports a bare `os error 10061`.
        if ctx.has_broker() {
            return TestResult::skipped(
                self.name(),
                SUITE,
                start.elapsed(),
                "requires --no-broker: the harness-managed broker binds the same default \
                 listeners as node 1 (1883/11883/8883/8080/8443/9443) and would stand in for \
                 it while node 2 stays down. Run: mqtt_harness --no-broker --addr \
                 127.0.0.1:1884 --workspace . --suites functional_v5_cluster --workers 1",
            );
        }

        // Keep the handle alive for the whole test: self-spawned nodes are
        // killed (and their ports released) when it is dropped.
        let (_nodes, spawned) = match acquire_cluster() {
            Ok(v) => v,
            Err(e) => return TestResult::failed(self.name(), SUITE, start.elapsed(), e),
        };
        tracing::info!(
            "pubrel-collision-cluster: {} ({NODE1_ADDR} <-> {NODE2_ADDR})",
            if spawned { "started self-managed nodes" } else { "using externally started nodes" }
        );

        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let mut reproductions = Vec::new();

            for round in 0..ROUNDS {
                let topic = format!("pubrel-collision-cluster/{uid}/r{round}");
                let sub_cid = format!("cc-sub-{uid}-r{round}");
                let pub_cid = format!("cc-pub-{uid}-r{round}");

                // ---- Phase 1: subscriber session on NODE 1 with incomplete QoS 2 exchanges
                let mut subscriber = MqttV5Client::connect_with_options(
                    NODE1_ADDR,
                    &sub_cid,
                    CONNECT_TIMEOUT,
                    true, // clean_start
                    60,
                    None,
                    None,
                    None,
                    Some(300), // session_expiry_interval
                    None,
                    None,
                )
                .await?;
                subscriber.subscribe(&topic, QoS::ExactlyOnce).await?;
                subscriber.set_auto_pubcomp(false);

                // Cross-node forward sanity check: publish on node 2, receive on node 1.
                // Use QoS 0 so the probe does NOT consume a packet id (the transferred
                // exchanges must start at id 1 for the collision to be observable).
                let probe = MqttV5Client::connect(NODE2_ADDR, &pub_cid, CONNECT_TIMEOUT).await?;
                probe
                    .publish_with_properties(
                        &topic,
                        b"probe",
                        QoS::AtMostOnce,
                        false,
                        None,
                        Some(300),
                        None,
                        None,
                        None,
                        None,
                    )
                    .await?;
                subscriber.recv_message_timeout(Duration::from_secs(5)).await.ok_or_else(|| {
                    anyhow::anyhow!(
                        "round {round}: cross-node forward probe (publish on {NODE2_ADDR}) never \
                         reached the subscriber on {NODE1_ADDR} — the two addresses are not a \
                         converged pubrel-collision cluster"
                    )
                })?;
                let _ = probe.disconnect().await;

                // Now leave N incomplete QoS 2 exchanges (publisher on node 1).
                let publisher = MqttV5Client::connect(NODE1_ADDR, &pub_cid, CONNECT_TIMEOUT).await?;
                let mut expected_rel_ids = Vec::new();
                for i in 0..UNCOMPLETE_COUNT {
                    let payload = format!("inflight-{round}-{i}");
                    publisher
                        .publish_with_properties(
                            &topic,
                            payload.as_bytes(),
                            QoS::ExactlyOnce,
                            false,
                            None,
                            Some(300),
                            None,
                            None,
                            None,
                            None,
                        )
                        .await?;
                    let msg = subscriber
                        .recv_message_timeout(Duration::from_secs(5))
                        .await
                        .ok_or_else(|| anyhow::anyhow!("round {round}: no PUBLISH delivered"))?;
                    if msg.payload.as_ref() != payload.as_bytes() {
                        return Err(anyhow::anyhow!(
                            "round {round}: unexpected payload {:?}",
                            String::from_utf8_lossy(&msg.payload)
                        ));
                    }
                    let rel_id = subscriber
                        .recv_pubrel_timeout(Duration::from_secs(5))
                        .await
                        .ok_or_else(|| anyhow::anyhow!("round {round}: no PUBREL after PUBREC"))?;
                    expected_rel_ids.push(rel_id);
                }
                let _ = subscriber.abort_connection().await;
                let _ = publisher.disconnect().await;
                tokio::time::sleep(Duration::from_millis(500)).await;

                // ---- Phase 2: publish stored messages on NODE 2 while the subscriber
                // is offline. Stored on node 2, delivered on node 1 (offline), NOT
                // mark_forwarded on node 2 (no local recipients).
                let storer = MqttV5Client::connect(NODE2_ADDR, &pub_cid, CONNECT_TIMEOUT).await?;
                for i in 0..STORED_COUNT {
                    storer
                        .publish_with_properties(
                            &topic,
                            format!("stored-{round}-{i}").as_bytes(),
                            QoS::ExactlyOnce,
                            false,
                            None,
                            Some(300),
                            None,
                            None,
                            None,
                            None,
                        )
                        .await?;
                }
                let _ = storer.disconnect().await;
                tokio::time::sleep(Duration::from_millis(300)).await;

                // ---- Phase 3: resume the session on NODE 2 (cross-node session
                // migration) so that send_storaged_messages (node 2) reloads M1
                // while send_rerelease re-sends the owed PUBRELs.
                let mut resumed = MqttV5Client::connect_with_options(
                    NODE2_ADDR,
                    &sub_cid,
                    CONNECT_TIMEOUT,
                    false, // clean_start = 0
                    60,
                    None,
                    None,
                    None,
                    Some(300),
                    None,
                    None,
                )
                .await?;

                let mut rel_ids = Vec::new();
                if let Some(first) = resumed.recv_pubrel_timeout(Duration::from_secs(5)).await {
                    rel_ids.push(first);
                    // Drain the rest until the channel is quiet.
                    while let Some(id) = resumed.recv_pubrel_timeout(Duration::from_millis(600)).await {
                        rel_ids.push(id);
                    }
                }
                let _ = resumed.disconnect().await;

                let mut counts: HashMap<u16, usize> = HashMap::new();
                for id in &rel_ids {
                    *counts.entry(*id).or_default() += 1;
                }
                let duplicates: Vec<(u16, usize)> =
                    counts.iter().filter(|(_, c)| **c >= 2).map(|(id, c)| (*id, *c)).collect();

                tracing::info!(
                    "pubrel-collision-cluster round {round}: expected {expected_rel_ids:?}, got {rel_ids:?}, \
                     duplicates: {duplicates:?}"
                );

                if !duplicates.is_empty() {
                    reproductions.push((round, duplicates.clone()));
                }
            }

            if !reproductions.is_empty() {
                Err(anyhow::anyhow!(
                    "BUG REPRODUCED (cluster): duplicate PUBREL packet-id(s) on cross-node resume \
                     (rounds: {reproductions:?}) — see designs/pubrel-resume-inflight-id-collision.md"
                ))
            } else {
                Ok(())
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), SUITE, start.elapsed()),
            Err(e) => TestResult::failed(self.name(), SUITE, start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(120)
    }
}
