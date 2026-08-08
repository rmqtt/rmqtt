//! QoS 2 PUBREL resume packet-id collision reproduction test
//!
//! Reproduces the concurrency bug analysed in
//! `designs/pubrel-resume-inflight-id-collision.md`:
//!
//! On session resume (`Clean Start = 0`), `transfer_session_state` both
//!   1. spawns `send_storaged_messages` (async) to load **stored** QoS messages, and
//!   2. re-forwards the owed PUBREL for every incomplete QoS 2 exchange
//!      (`send_rerelease`, which re-registers the message into `out_inflight`
//!      under its **old** packet-id).
//!
//! The new session's packet-id allocator (`OutInflight::next_id`) restarts at 1,
//! so a stored message delivered concurrently can be assigned the **same**
//! packet-id as a transferred message. `OutInflight::push_back` is a
//! `HashMap::insert` — the second registration **silently overwrites** the first,
//! destroying the overwritten message's QoS 2 state.
//!
//! Observable symptom: the client receives **two PUBREL with the same packet id**
//! (one from `send_rerelease`, one from the PUBREC-response path acting on the
//! overwritten entry). A fixed broker (packet-id space isolation) never produces
//! duplicate PUBRELs.
//!
//! # Reproduction window (timing-sensitive)
//!
//! A stored message is only re-loaded by `send_storaged_messages` if it was not
//! yet `mark_forwarded`-ed. `mark_forwarded` is executed **asynchronously** by
//! the message-storage workers (`route_msg` → worker queue), so if the client
//! disconnects and **immediately** reconnects, some messages may still be seen
//! as un-forwarded and be re-delivered — racing with the owed PUBREL re-sends.
//! Larger message counts lengthen the worker backlog and widen the window.
//!
//! The test is a *probabilistic* reproduction: it reports FAILED (with evidence)
//! when a duplicate PUBREL is observed, and PASSES when no round hits the window.
//! Tune `UNCOMPLETE_COUNT` / `ROUNDS` to increase the hit rate.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v5::MqttV5Client;

/// Number of incomplete QoS 2 exchanges left in the session before resume.
/// Larger values lengthen the async mark_forwarded backlog.
const UNCOMPLETE_COUNT: usize = 8;

/// Disconnect → reconnect latencies tried (ms). 0 = reconnect as fast as possible.
const RECONNECT_LATENCIES_MS: &[u64] = &[0, 20, 50];

/// Reproduction: duplicate PUBREL on session resume with concurrent stored-message delivery.
pub struct Qos2PubrelResumeCollisionTest;

impl TestCase for Qos2PubrelResumeCollisionTest {
    fn name(&self) -> &str {
        "qos2_pubrel_resume_collision"
    }

    /// The reproduction needs the `rmqtt-message-storage` plugin (stored
    /// message re-delivery on resume), which the default config does not
    /// load; the harness splits this test into the
    /// `functional_v5@pubrel-collision` sub-suite automatically.
    fn broker_config(&self) -> Option<std::path::PathBuf> {
        Some(crate::tests::config_path("pubrel-collision"))
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let pub_cid = format!("coll-pub-{uid}");

            let mut reproductions = Vec::new();

            for (round, latency_ms) in RECONNECT_LATENCIES_MS.iter().enumerate() {
                let topic = format!("pubrel-collision/{uid}/r{round}");
                let sub_cid = format!("coll-sub-{uid}-r{round}");

                // ---- Phase 1: persistent session, leave N incomplete QoS 2 exchanges
                let mut subscriber = MqttV5Client::connect_with_options(
                    &ctx.config.broker_addr,
                    &sub_cid,
                    ctx.config.connect_timeout,
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
                // Never answer PUBREL → every QoS 2 exchange stays incomplete.
                subscriber.set_auto_pubcomp(false);

                let publisher =
                    MqttV5Client::connect(&ctx.config.broker_addr, &pub_cid, ctx.config.connect_timeout)
                        .await?;

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
                            Some(300), // message_expiry_interval → message-storage stores it
                            None,
                            None,
                            None,
                            None,
                        )
                        .await?;

                    // Subscriber receives PUBLISH (auto-replies PUBREC), then PUBREL.
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

                // Leave the session with the incomplete exchanges, then reconnect
                // quickly so the async mark_forwarded may not have landed yet.
                let _ = subscriber.abort_connection().await;
                let _ = publisher.disconnect().await;
                tokio::time::sleep(Duration::from_millis(*latency_ms)).await;

                // ---- Phase 2: resume (Clean Start = 0), collect all PUBRELs
                let mut resumed = MqttV5Client::connect_with_options(
                    &ctx.config.broker_addr,
                    &sub_cid,
                    ctx.config.connect_timeout,
                    false, // clean_start = 0 → session resume
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

                // ---- Assert: no packet-id may appear twice among the PUBRELs.
                let mut counts: HashMap<u16, usize> = HashMap::new();
                for id in &rel_ids {
                    *counts.entry(*id).or_default() += 1;
                }
                let duplicates: Vec<(u16, usize)> =
                    counts.iter().filter(|(_, c)| **c >= 2).map(|(id, c)| (*id, *c)).collect();

                tracing::info!(
                    "pubrel-collision round {round}: expected {expected_rel_ids:?}, got {rel_ids:?}, \
                     duplicates: {duplicates:?}"
                );

                if !duplicates.is_empty() {
                    reproductions.push((round, duplicates.clone()));
                }
            }

            if !reproductions.is_empty() {
                Err(anyhow::anyhow!(
                    "BUG REPRODUCED: duplicate PUBREL packet-id(s) on session resume \
                     (rounds: {reproductions:?}); stored-message registration was overwritten \
                     by send_rerelease — see designs/pubrel-resume-inflight-id-collision.md"
                ))
            } else {
                Ok(())
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(120)
    }
}
