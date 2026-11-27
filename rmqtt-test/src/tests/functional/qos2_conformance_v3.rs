//! QoS 2 exactly-once conformance tests for MQTT v3.1
//!
//! Mirrors the v3.1.1 conformance tests (`qos2_conformance_v311.rs`) for the
//! MQTT v3.1 protocol (MQIsdp / level 3). The root causes live in the
//! protocol-version-agnostic inflight / session-resume code paths of
//! `rmqtt/src/session.rs`, so MQTT 3.1 is affected exactly like 3.1.1 and 5.0:
//!
//! - **MQTT-4.3.3-10** — a replayed QoS 2 PUBLISH (same Packet Identifier,
//!   DUP=1, before PUBREL) must not be delivered to the subscriber a second
//!   time.
//! - **MQTT-4.4.0-1** — after the Server has received PUBREC and sent PUBREL,
//!   it owes a PUBREL; on a Clean Session 0 reconnect the Server MUST resend
//!   the owed PUBREL with its original Packet Identifier.
//!
//! In MQTT 3.1 "Clean Session 0" plays the role of the persistent session for
//! session resume purposes.

use std::num::NonZeroU16;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoSTest;
use crate::mqtt::v3::MqttV3Client;

/// Issue 1 [MQTT-4.3.3-10]: a replayed QoS 2 PUBLISH is delivered twice (v3.1)
pub struct Qos2ReplayedPublishDedupV3Test;

impl TestCase for Qos2ReplayedPublishDedupV3Test {
    fn name(&self) -> &str {
        "qos2_replayed_publish_dedup_v3"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let topic = format!("issue456/dedup/v3/{uid}");
            let payload = format!("REPLAY-{uid}");
            let sub_cid = format!("sub-v3-{uid}");
            let pub_cid = format!("pub-v3-{uid}");

            // Subscriber with a QoS 2 subscription
            let mut subscriber =
                MqttV3Client::connect(&ctx.config.broker_addr, &sub_cid, ctx.config.connect_timeout).await?;
            subscriber.subscribe(&topic, QoSTest::ExactlyOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let publisher =
                MqttV3Client::connect(&ctx.config.broker_addr, &pub_cid, ctx.config.connect_timeout).await?;
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
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// Issue 2 [MQTT-4.4.0-1]: an owed PUBREL is not resent on session resume (v3.1)
pub struct Qos2PubrelResendOnResumeV3Test;

impl TestCase for Qos2PubrelResendOnResumeV3Test {
    fn name(&self) -> &str {
        "qos2_pubrel_resend_on_resume_v3"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let topic = format!("issue456/resume/v3/{uid}");
            let cid = format!("subr-v3-{uid}");
            let pub_cid = format!("pubr-v3-{uid}");
            let payload = format!("RESUME-{uid}");

            // Phase 1: persistent session (Clean Session 0) + QoS 2 subscription
            let mut subscriber = MqttV3Client::connect_with_options(
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
            // Disable auto-PUBCOMP BEFORE the exchange starts (the broker can
            // answer PUBREC with PUBREL within microseconds; disabling it
            // after the PUBLISH races the reader loop and auto-completes the
            // exchange, leaving no owed PUBREL on resume).
            subscriber.set_auto_pubcomp(false);
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Publish a QoS 2 message
            let publisher =
                MqttV3Client::connect(&ctx.config.broker_addr, &pub_cid, ctx.config.connect_timeout).await?;
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
            let mut resumed = MqttV3Client::connect_with_options(
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
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}
