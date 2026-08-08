//! MQTT v3.1 Session management tests (Clean Session semantics)
//!
//! Covers spec section 3.1 (Clean Session flag):
//! - clean_session = 0: subscriptions and QoS 1/2 messages persist across
//!   reconnect; session present flag is 1 on resume
//! - clean_session = 1: all state is discarded; session present is 0
//! - offline QoS 1 messages are queued and delivered in order

use std::time::{Duration, Instant};

use rmqtt_codec::v3::QoS;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Positive: with clean_session = 0 the session (subscriptions + queued
/// messages) survives a reconnect; session present = 1 on resume.
pub struct SessionV3PersistentTest;

impl TestCase for SessionV3PersistentTest {
    fn name(&self) -> &str {
        "session_v3_persistent"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let cid = format!("session-v3-{uid}");
            let topic = format!("test/v3/session/persist/{uid}");
            let payload = b"queued_msg_v3";

            // Phase 1: Connect with clean_session = false and subscribe
            let mut client = crate::mqtt::v3::MqttV3Client::connect_with_options(
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

            client.subscribe(&topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Clean disconnect (session should persist)
            client.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 2: Publish while client is disconnected
            let publisher = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("session-v3-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            publisher.publish(&topic, payload, QoS::AtLeastOnce, false).await?;
            publisher.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 3: Reconnect with same client_id + clean_session = false
            let mut reconnected = crate::mqtt::v3::MqttV3Client::connect_with_options(
                &ctx.config.broker_addr,
                &cid,
                ctx.config.connect_timeout,
                false,
                60,
                None,
                None,
                None,
            )
            .await?;

            let session_present = reconnected.connack().session_present;
            let msg = reconnected.recv_message_timeout(Duration::from_secs(5)).await;
            reconnected.disconnect().await?;

            if !session_present {
                return Err(anyhow::anyhow!("session present = 0 after reconnect with clean_session = 0"));
            }
            match msg {
                Some(m) if m.payload.as_ref() == payload => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected queued msg: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("no queued message received after reconnect")),
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

/// Negative: with clean_session = 1 the broker discards all session state on
/// disconnect; no queued messages and session present = 0 on reconnect.
pub struct SessionV3CleanTest;

impl TestCase for SessionV3CleanTest {
    fn name(&self) -> &str {
        "session_v3_clean"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let cid = format!("session-clean-v3-{uid}");
            let topic = format!("test/v3/session/clean/{uid}");

            // Phase 1: clean_session = true, subscribe
            let mut client = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &cid,
                ctx.config.connect_timeout,
            )
            .await?;
            client.subscribe(&topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;
            client.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 2: publish while offline
            let publisher = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("session-clean-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            publisher.publish(&topic, b"should_not_be_queued", QoS::AtLeastOnce, false).await?;
            publisher.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 3: reconnect with clean_session = true
            let mut reconnected = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &cid,
                ctx.config.connect_timeout,
            )
            .await?;

            let session_present = reconnected.connack().session_present;
            let msg = reconnected.recv_message_timeout(Duration::from_secs(2)).await;
            reconnected.disconnect().await?;

            if session_present {
                return Err(anyhow::anyhow!("session present = 1 after clean_session = true disconnect"));
            }
            if msg.is_some() {
                return Err(anyhow::anyhow!(
                    "clean_session = true client received a message queued while offline"
                ));
            }
            Ok(())
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

/// Positive: offline QoS 1 messages are queued and delivered in order after
/// reconnect with clean_session = 0.
pub struct SessionV3OfflineQueueTest;

impl TestCase for SessionV3OfflineQueueTest {
    fn name(&self) -> &str {
        "session_v3_offline_queue"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let cid = format!("queue-v3-{uid}");
            let topic = format!("test/v3/session/queue/{uid}");
            let n = 5;

            // Phase 1: connect with clean_session = false, subscribe
            let mut client = crate::mqtt::v3::MqttV3Client::connect_with_options(
                &ctx.config.broker_addr,
                &cid,
                ctx.config.connect_timeout,
                false,
                60,
                None,
                None,
                None,
            )
            .await?;
            client.subscribe(&topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;
            client.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 2: publish N messages while offline
            let publisher = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("queue-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            for i in 0..n {
                publisher.publish(&topic, format!("msg-{i}").as_bytes(), QoS::AtLeastOnce, false).await?;
            }
            publisher.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 3: reconnect, all N messages delivered in order
            let mut reconnected = crate::mqtt::v3::MqttV3Client::connect_with_options(
                &ctx.config.broker_addr,
                &cid,
                ctx.config.connect_timeout,
                false,
                60,
                None,
                None,
                None,
            )
            .await?;

            for i in 0..n {
                let msg = reconnected
                    .recv_message_timeout(Duration::from_secs(5))
                    .await
                    .ok_or_else(|| anyhow::anyhow!("missing queued message #{i}"))?;
                if msg.payload.as_ref() != format!("msg-{i}").as_bytes() {
                    return Err(anyhow::anyhow!(
                        "out-of-order queue: expected msg-{i}, got {:?}",
                        msg.payload
                    ));
                }
            }
            reconnected.disconnect().await?;
            Ok(())
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
