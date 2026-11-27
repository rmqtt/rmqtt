//! QoS 2 exactly-once conformance tests for MQTT v3.1.1
//!
//! Same two conformance violations as the v5 tests (GitHub issue #456,
//! <https://github.com/rmqtt/rmqtt/issues/456>). The root causes live in the
//! protocol-version-agnostic inflight / session-resume code paths of
//! `rmqtt/src/session.rs`, so MQTT 3.1.1 is affected exactly like 3.1 and 5.0:
//!
//! - **MQTT-4.3.3-10** — a replayed QoS 2 PUBLISH (same Packet Identifier,
//!   DUP=1, before PUBREL) must not be delivered to the subscriber a second
//!   time.
//! - **MQTT-4.4.0-1** — after the Server has received PUBREC and sent PUBREL,
//!   it owes a PUBREL; on a Clean Session 0 reconnect the Server MUST resend
//!   the owed PUBREL with its original Packet Identifier.
//!
//! In MQTT 3.1.1 "Clean Session 0" plays the role of the v5 "Clean Start 0"
//! for session resume purposes. Both tests assert the spec-required behaviour,
//! so they FAIL on the buggy broker (reproducing the issue) and PASS once fixed.

use std::num::NonZeroU16;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoSTest;
use crate::mqtt::v311::MqttV311Client;
use crate::tests::functional::cluster_session_restart::{rmqttd_binary, ClusterNode};

/// MQTT port of the self-managed broker for the broker->client QoS 2 tests.
/// Reuses the `rl-boundary` config (ports 1891/5365, message_retry_interval
/// 5s, all default plugins disabled); the harness runs suites serially, so
/// the boundary tests and this test never overlap.
const QOS2_BROKER_ADDR: &str = "127.0.0.1:1891";
const QOS2_NODE_START_TIMEOUT: Duration = Duration::from_secs(20);

/// Spawn a self-managed broker for the broker->client QoS 2 test. Dropping
/// the returned node kills the broker.
fn spawn_qos2_broker() -> Result<(ClusterNode, PathBuf), anyhow::Error> {
    let binary = rmqttd_binary();
    if !binary.exists() {
        return Err(anyhow::anyhow!(
            "rmqttd binary not found at {:?}; build it first (cargo build -p rmqttd)",
            binary
        ));
    }
    let config = crate::tests::config_path("rl-boundary");
    let log_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target");
    let log_file = log_dir.join("qos2-nopubrec-node.log");
    let mut node = ClusterNode::new(config, QOS2_BROKER_ADDR, log_file);
    node.spawn(&binary)?;
    if !node.wait_healthy(QOS2_NODE_START_TIMEOUT) {
        return Err(anyhow::anyhow!("qos2 broker did not become healthy"));
    }
    Ok((node, binary))
}

/// Issue 1 [MQTT-4.3.3-10]: a replayed QoS 2 PUBLISH is delivered twice (v3.1.1)
pub struct Qos2ReplayedPublishDedupV311Test;

impl TestCase for Qos2ReplayedPublishDedupV311Test {
    fn name(&self) -> &str {
        "qos2_replayed_publish_dedup_v311"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let topic = format!("issue456/dedup/{uid}");
            let payload = format!("REPLAY-{uid}");
            let sub_cid = format!("sub-{uid}");
            let pub_cid = format!("pub-{uid}");

            // Subscriber with a QoS 2 subscription
            let mut subscriber =
                MqttV311Client::connect(&ctx.config.broker_addr, &sub_cid, ctx.config.connect_timeout)
                    .await?;
            subscriber.subscribe(&topic, QoSTest::ExactlyOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let publisher =
                MqttV311Client::connect(&ctx.config.broker_addr, &pub_cid, ctx.config.connect_timeout)
                    .await?;
            let pid = NonZeroU16::new(7).expect("7 is non-zero");

            // 1st PUBLISH (normal QoS 2 handshake start)
            publisher
                .publish_with_packet_id(&topic, payload.as_bytes(), QoSTest::ExactlyOnce, false, false, pid)
                .await?;

            // First delivery
            let first = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

            // Replay the same QoS 2 PUBLISH with the SAME Packet Identifier
            // (DUP=1) before the exchange completed (PUBREL not sent yet).
            publisher
                .publish_with_packet_id(&topic, payload.as_bytes(), QoSTest::ExactlyOnce, false, true, pid)
                .await?;

            // A conformant broker answers PUBREC and does NOT deliver again.
            tokio::time::sleep(Duration::from_millis(500)).await;
            let second = subscriber.recv_message_timeout(Duration::from_millis(300)).await;

            // Best-effort finish of the exchange; the broker may already have
            // dropped the publisher connection on the duplicate.
            let _ = publisher.send_pubrel(pid).await;
            let _ = publisher.disconnect().await;
            let _ = subscriber.disconnect().await;

            if second.is_some() {
                return Err(anyhow::anyhow!("replayed QoS 2 PUBLISH was delivered twice [MQTT-4.3.3-10]"));
            }
            match first {
                Some(m) if m.payload.as_ref() == payload.as_bytes() => Ok(()),
                Some(_) => Err(anyhow::anyhow!("unexpected payload on first delivery")),
                None => Err(anyhow::anyhow!("no QoS 2 PUBLISH delivered")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// Issue 2 [MQTT-4.4.0-1]: an owed PUBREL is not resent on session resume (v3.1.1)
///
/// Clean Session 0 keeps the session (MQTT 3.1.1 "Session Present" semantics),
/// equivalent to the v5 Clean Start 0 + session expiry scenario.
pub struct Qos2PubrelResendOnResumeV311Test;

impl TestCase for Qos2PubrelResendOnResumeV311Test {
    fn name(&self) -> &str {
        "qos2_pubrel_resend_on_resume_v311"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let topic = format!("issue456/resume/{uid}");
            let cid = format!("subr-{uid}");
            let pub_cid = format!("pubr-{uid}");
            let payload = format!("RESUME-{uid}");

            // Phase 1: persistent session (Clean Session 0) + QoS 2 subscription
            let mut subscriber = MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                &cid,
                ctx.config.connect_timeout,
                false, // clean_session = false (persistent session)
                60,
                None,
                None,
                None,
            )
            .await?;
            subscriber.subscribe(&topic, QoSTest::ExactlyOnce).await?;
            // Disable auto-PUBCOMP BEFORE the exchange starts: the broker can
            // answer the PUBREC with PUBREL within microseconds, so disabling
            // it after receiving the PUBLISH races the reader loop and can
            // auto-complete the QoS 2 exchange (no owed PUBREL on resume).
            subscriber.set_auto_pubcomp(false);
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Publish a QoS 2 message
            let publisher =
                MqttV311Client::connect(&ctx.config.broker_addr, &pub_cid, ctx.config.connect_timeout)
                    .await?;
            publisher
                .publish_with_packet_id(
                    &topic,
                    payload.as_bytes(),
                    QoSTest::ExactlyOnce,
                    false,
                    false,
                    NonZeroU16::new(1).expect("1 is non-zero"),
                )
                .await?;

            // Subscriber receives the PUBLISH (client auto-replies PUBREC),
            // then the broker sends PUBREL. Do NOT answer with PUBCOMP so the
            // QoS 2 exchange is left incomplete.
            let msg = subscriber
                .recv_message_timeout(Duration::from_secs(5))
                .await
                .ok_or_else(|| anyhow::anyhow!("no QoS 2 PUBLISH delivered"))?;
            if msg.payload.as_ref() != payload.as_bytes() {
                return Err(anyhow::anyhow!("unexpected payload: {:?}", msg.payload));
            }

            let rel_pid = subscriber
                .recv_pubrel_timeout(Duration::from_secs(5))
                .await
                .ok_or_else(|| anyhow::anyhow!("no PUBREL received after PUBREC"))?;

            // Drop the connection without PUBCOMP, leaving the exchange incomplete
            let _ = subscriber.abort_connection().await;
            let _ = publisher.disconnect().await;

            // Give the broker time to detect the disconnect and transfer the
            // session state (inflight messages) to the offline session
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 2: reconnect with Clean Session 0 and the same client id
            let mut resumed = MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                &cid,
                ctx.config.connect_timeout,
                false, // clean_session = false
                60,
                None,
                None,
                None,
            )
            .await?;
            let session_present = resumed.connack().session_present;

            // A conformant broker must resend the owed PUBREL with its
            // original Packet Identifier.
            let resent = resumed.recv_pubrel_timeout(Duration::from_secs(5)).await;

            let _ = resumed.disconnect().await;

            match (session_present, resent) {
                (true, Some(pid)) if pid == rel_pid => Ok(()),
                (true, Some(pid)) => {
                    Err(anyhow::anyhow!("PUBREL resent with packet id {pid}, expected {rel_pid}"))
                }
                (true, None) => Err(anyhow::anyhow!(
                    "session resumed (session_present=1) but the owed PUBREL was not resent \
                     [MQTT-4.4.0-1]"
                )),
                (false, _) => Err(anyhow::anyhow!(
                    "session not resumed (session_present=0), expected a persistent session"
                )),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

// ---------------------------------------------------------------------------
// P0 conformance gap fill (G10): QoS 1 redelivery on session resume
// ---------------------------------------------------------------------------

/// [MQTT-4.4.0-1] — a QoS 1 message sent to a subscriber who never answered
/// with PUBACK must be resent on a Clean Session 0 reconnect, with DUP=1 and
/// the original packet identifier.
///
/// Mirrors `qos2_pubrel_resend_on_resume_v311` for the QoS 1 path.
pub struct Qos1PublishResendOnResumeV311Test;

impl TestCase for Qos1PublishResendOnResumeV311Test {
    fn name(&self) -> &str {
        "qos1_publish_resend_on_resume_v311"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let topic = format!("qos1/resume/{uid}");
            let cid = format!("qos1-sub-{uid}");
            let pub_cid = format!("qos1-pub-{uid}");
            let payload = format!("QOS1-RESUME-{uid}");
            let pid = NonZeroU16::new(5).expect("5 is non-zero");

            // Phase 1: persistent session (Clean Session 0) + QoS 1 subscription,
            // PUBACK auto-answer disabled so the exchange is left incomplete.
            let mut subscriber = MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                &cid,
                ctx.config.connect_timeout,
                false, // clean_session = false (persistent session)
                60,
                None,
                None,
                None,
            )
            .await?;
            subscriber.set_auto_puback(false);
            subscriber.subscribe(&topic, QoSTest::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Publish a QoS 1 message with a known packet identifier.
            let publisher =
                MqttV311Client::connect(&ctx.config.broker_addr, &pub_cid, ctx.config.connect_timeout)
                    .await?;
            publisher
                .publish_with_packet_id(&topic, payload.as_bytes(), QoSTest::AtLeastOnce, false, false, pid)
                .await?;

            // Subscriber receives the first delivery (no PUBACK sent).
            let first = subscriber
                .recv_message_timeout(Duration::from_secs(5))
                .await
                .ok_or_else(|| anyhow::anyhow!("no QoS 1 PUBLISH delivered"))?;
            if first.payload.as_ref() != payload.as_bytes() {
                return Err(anyhow::anyhow!("unexpected first payload: {:?}", first.payload));
            }
            if first.dup {
                return Err(anyhow::anyhow!("first delivery must have DUP=0"));
            }
            // NOTE: the packet id seen by the subscriber is the one assigned
            // by the BROKER for the broker→client direction; it is unrelated
            // to the packet id the publisher used. Record it for comparison.
            let first_pid = first
                .packet_id
                .ok_or_else(|| anyhow::anyhow!("QoS 1 delivery must carry a packet identifier"))?;

            // Drop the connection without PUBACK, leaving the exchange incomplete.
            let _ = subscriber.abort_connection().await;
            let _ = publisher.disconnect().await;

            // Give the broker time to move the in-flight message to the session.
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 2: reconnect with Clean Session 0 and the same client id.
            let mut resumed = MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                &cid,
                ctx.config.connect_timeout,
                false, // clean_session = false
                60,
                None,
                None,
                None,
            )
            .await?;
            let session_present = resumed.connack().session_present;

            // A conformant broker resends the QoS 1 message with DUP=1 and the
            // same packet identifier it originally used.
            let resent = resumed.recv_message_timeout(Duration::from_secs(5)).await;
            let _ = resumed.disconnect().await;

            match (session_present, resent) {
                (true, Some(m))
                    if m.dup
                        && m.packet_id == Some(first_pid)
                        && m.payload.as_ref() == payload.as_bytes() =>
                {
                    Ok(())
                }
                (true, Some(m)) => Err(anyhow::anyhow!(
                    "resent QoS 1 message mismatch: dup={}, packet_id={:?}, expected packet_id={first_pid}",
                    m.dup,
                    m.packet_id,
                )),
                (true, None) => Err(anyhow::anyhow!(
                    "session resumed (session_present=1) but the QoS 1 message was not resent \
                     [MQTT-4.4.0-1]"
                )),
                (false, _) => Err(anyhow::anyhow!(
                    "session not resumed (session_present=0), expected a persistent session"
                )),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// Broker -> client QoS 2 break point: a subscriber that receives the QoS 2
/// PUBLISH but never answers PUBREC leaves the broker's exchange incomplete
/// at WAIT_PUBREC; the broker must retransmit the PUBLISH with DUP=1
/// (MQTT-4.3.3). [G28]
///
/// Runs against a self-managed broker with `message_retry_interval = 5s`
/// (the `rl-boundary` config), so the retransmission arrives within a few
/// seconds.
pub struct Qos2BrokerToClientNoPubrecV311Test;

impl TestCase for Qos2BrokerToClientNoPubrecV311Test {
    fn name(&self) -> &str {
        "qos2_broker_to_client_no_pubrec_v311"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let (_node, _binary) = spawn_qos2_broker()?;
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let topic = format!("qos2/nopubrec/{uid}");
            let payload = format!("NO-PUBREC-{uid}");

            let mut subscriber = MqttV311Client::connect(
                QOS2_BROKER_ADDR,
                &format!("qos2-nopubrec-sub-{uid}"),
                Duration::from_secs(10),
            )
            .await?;
            // Do NOT answer the incoming QoS 2 PUBLISH with a PUBREC.
            subscriber.set_auto_pubrec(false);
            subscriber.subscribe(&topic, QoSTest::ExactlyOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let publisher = MqttV311Client::connect(
                QOS2_BROKER_ADDR,
                &format!("qos2-nopubrec-pub-{uid}"),
                Duration::from_secs(10),
            )
            .await?;
            publisher.publish(&topic, payload.as_bytes(), QoSTest::ExactlyOnce, false).await?;

            // First delivery: DUP=0.
            let first = subscriber
                .recv_message_timeout(Duration::from_secs(3))
                .await
                .ok_or_else(|| anyhow::anyhow!("no first QoS 2 delivery"))?;
            if first.payload.as_ref() != payload.as_bytes() {
                return Err(anyhow::anyhow!("first delivery payload mismatch"));
            }
            if first.dup {
                return Err(anyhow::anyhow!("first delivery must not have DUP set"));
            }

            // No PUBREC was sent: after message_retry_interval (5s) the broker
            // must retransmit the same PUBLISH with DUP=1.
            let second = subscriber.recv_message_timeout(Duration::from_secs(12)).await.ok_or_else(|| {
                anyhow::anyhow!("broker did not retransmit the un-ACKed QoS 2 PUBLISH [MQTT-4.3.3]")
            })?;
            if second.payload.as_ref() != payload.as_bytes() {
                return Err(anyhow::anyhow!("retransmission payload mismatch"));
            }
            if !second.dup {
                return Err(anyhow::anyhow!("retransmitted QoS 2 PUBLISH must carry DUP=1 [MQTT-4.3.3]"));
            }

            let _ = subscriber.disconnect().await;
            publisher.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}
