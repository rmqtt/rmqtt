//! MQTT 3.1.1 Session management tests

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Test session persistence with clean_session=false (v3.1.1)
pub struct CleanSessionFalseTest;

impl TestCase for CleanSessionFalseTest {
    fn name(&self) -> &str {
        "clean_session_false_v311"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let topic = "test/v311/session/persist";
            let payload = b"queued_msg";

            // Phase 1: Connect with clean_session=false and subscribe
            let mut client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "session-v311-client",
                ctx.config.connect_timeout,
                false, // clean_session = false
                60,
                None,
                None,
                None,
            )
            .await?;

            client.subscribe(topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Clean disconnect (session should persist)
            client.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 2: Publish while client is disconnected
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "session-v311-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            publisher.publish(topic, payload, QoS::AtLeastOnce, false).await?;
            publisher.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 3: Reconnect with same client_id + clean_session=false
            let mut reconnected = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "session-v311-client",
                ctx.config.connect_timeout,
                false,
                60,
                None,
                None,
                None,
            )
            .await?;

            // Should receive the queued message
            let msg = reconnected.recv_message_timeout(Duration::from_secs(5)).await;
            reconnected.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == payload => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected queued msg: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("no queued message received after reconnect")),
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

/// Test offline message queue - multiple messages delivered in order (v3.1.1)
pub struct OfflineQueueV311Test;

impl TestCase for OfflineQueueV311Test {
    fn name(&self) -> &str {
        "offline_queue_v311"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let topic = "test/v311/session/queue";

            // Phase 1: Connect with clean_session=false and subscribe
            let mut client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "queue-v311-client",
                ctx.config.connect_timeout,
                false,
                60,
                None,
                None,
                None,
            )
            .await?;

            client.subscribe(topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Clean disconnect
            client.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 2: Publish 10 messages while client is offline
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "queue-v311-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            for i in 0..10 {
                let payload = format!("msg{}", i);
                publisher.publish(topic, payload.as_bytes(), QoS::AtLeastOnce, false).await?;
            }
            publisher.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 3: Reconnect and verify all 10 messages arrive in order
            let mut reconnected = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "queue-v311-client",
                ctx.config.connect_timeout,
                false,
                60,
                None,
                None,
                None,
            )
            .await?;

            let mut received = Vec::new();
            for _ in 0..10 {
                match reconnected.recv_message_timeout(Duration::from_secs(5)).await {
                    Some(msg) => received.push(msg),
                    None => break,
                }
            }
            reconnected.disconnect().await?;

            if received.len() < 10 {
                return Err(anyhow::anyhow!("expected 10 queued messages, got {}", received.len()));
            }

            // Verify ordering
            for (i, msg) in received.iter().enumerate() {
                let expected = format!("msg{}", i);
                if msg.payload.as_ref() != expected.as_bytes() {
                    return Err(anyhow::anyhow!(
                        "message {} mismatch: expected {:?}, got {:?}",
                        i,
                        expected,
                        msg.payload
                    ));
                }
            }

            Ok(())
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

/// Positive: Session Present = 1 when reconnecting with clean_session = 0 and
/// an existing stored session. [MQTT-3.2.2.1]
pub struct SessionV311PresentOnResumeTest;

impl TestCase for SessionV311PresentOnResumeTest {
    fn name(&self) -> &str {
        "session_v311_present_on_resume"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let cid = format!("session-present-{uid}");

            // Phase 1: clean_session = 0, subscribe (creates a stored session)
            let mut client = crate::mqtt::v311::MqttV311Client::connect_with_options(
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
            client.subscribe(&format!("test/v311/present/{uid}"), QoS::AtLeastOnce).await?;
            client.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 2: reconnect with clean_session = 0 — session present = 1
            let resumed = crate::mqtt::v311::MqttV311Client::connect_with_options(
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
            let session_present = resumed.connack().session_present;
            resumed.disconnect().await?;

            if session_present {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "session present must be 1 when resuming a stored session [MQTT-3.2.2.1]"
                ))
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

/// Negative: clean_session = 1 discards all stored session state; reconnecting
/// with the same client id and clean_session = 1 must NOT receive messages
/// queued while offline, and session present = 0. [MQTT-3.1.2-6]
pub struct SessionV311CleanDiscardTest;

impl TestCase for SessionV311CleanDiscardTest {
    fn name(&self) -> &str {
        "session_v311_clean_discard"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let cid = format!("session-clean-discard-{uid}");
            let topic = format!("test/v311/session/clean/{uid}");

            // Phase 1: clean_session = false, subscribe
            let mut client = crate::mqtt::v311::MqttV311Client::connect_with_options(
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

            // Phase 2: publish while offline
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &format!("session-clean-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            publisher.publish(&topic, b"should_not_be_queued", QoS::AtLeastOnce, false).await?;
            publisher.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 3: reconnect with clean_session = true — state must be gone
            let mut reconnected = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &cid,
                ctx.config.connect_timeout,
            )
            .await?;
            let session_present = reconnected.connack().session_present;
            let msg = reconnected.recv_message_timeout(Duration::from_secs(2)).await;
            reconnected.disconnect().await?;

            if session_present {
                return Err(anyhow::anyhow!(
                    "session present must be 0 after clean_session = 1 disconnect [MQTT-3.1.2-6]"
                ));
            }
            if msg.is_some() {
                return Err(anyhow::anyhow!(
                    "clean_session = 1 client received a message queued while offline"
                ));
            }
            Ok(())
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
