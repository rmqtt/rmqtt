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
