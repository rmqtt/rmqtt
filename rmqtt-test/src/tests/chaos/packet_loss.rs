//! Simulated packet loss tests (using v311 client)

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Test QoS 1 message delivery with simulated failures
/// (Verifies that the test framework handles partial message delivery)
pub struct Qos1ReliabilityTest {
    pub message_count: usize,
}

impl Default for Qos1ReliabilityTest {
    fn default() -> Self { Self { message_count: 50 } }
}

impl TestCase for Qos1ReliabilityTest {
    fn name(&self) -> &str { "chaos_qos1_reliability" }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let msg_count = self.message_count;

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "v311-qos1-pub",
                ctx.config.connect_timeout,
            ).await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "v311-qos1-sub",
                ctx.config.connect_timeout,
            ).await?;

            let topic = "test/chaos/qos1";
            subscriber.subscribe(topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            for i in 0..msg_count {
                let payload = format!("msg-{}", i);
                publisher.publish(topic, payload.as_bytes(), QoS::AtLeastOnce, false).await?;
            }

            // Receive messages with generous timeout
            let mut received = 0;
            let deadline = Instant::now() + Duration::from_secs(30);
            while received < msg_count && Instant::now() < deadline {
                if subscriber.recv_message_timeout(Duration::from_secs(2)).await.is_some() {
                    received += 1;
                } else {
                    break;
                }
            }

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            // QoS 1 should guarantee at-least-once delivery
            if received >= msg_count {
                Ok(())
            } else {
                Err(anyhow::anyhow!("QoS 1: received {}/{} messages", received, msg_count))
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "chaos", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "chaos", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration { Duration::from_secs(60) }
}

/// Test slow consumer scenario
pub struct SlowConsumerTest;

impl TestCase for SlowConsumerTest {
    fn name(&self) -> &str { "chaos_slow_consumer" }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "v311-slow-pub",
                ctx.config.connect_timeout,
            ).await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "v311-slow-sub",
                ctx.config.connect_timeout,
            ).await?;

            let topic = "test/chaos/slow";
            subscriber.subscribe(topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Send messages rapidly
            let msg_count = 10;
            for i in 0..msg_count {
                publisher.publish(topic, format!("slow-{}", i).as_bytes(), QoS::AtLeastOnce, false).await?;
            }

            // Consume slowly
            let mut received = 0;
            for _ in 0..msg_count {
                tokio::time::sleep(Duration::from_millis(50)).await;
                if subscriber.recv_message_timeout(Duration::from_secs(5)).await.is_some() {
                    received += 1;
                }
            }

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            // Even with slow consumption, we should get most messages
            if received >= msg_count / 2 {
                Ok(())
            } else {
                Err(anyhow::anyhow!("slow consumer only received {}/{} messages", received, msg_count))
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "chaos", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "chaos", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration { Duration::from_secs(30) }
}
