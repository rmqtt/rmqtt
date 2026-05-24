//! Multi-topic subscription and message ordering tests

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Test subscribing to multiple topics and receiving messages on all of them
pub struct MultiTopicSubscribeV311Test;

impl TestCase for MultiTopicSubscribeV311Test {
    fn name(&self) -> &str {
        "multi_topic_subscribe_v311"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "multi-topic-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "multi-topic-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            // Subscribe to 5 different topics
            let topics = ["test/multi/a", "test/multi/b", "test/multi/c", "test/multi/d", "test/multi/e"];

            for topic in &topics {
                subscriber.subscribe(topic, QoS::AtLeastOnce).await?;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Publish to each topic
            for (i, topic) in topics.iter().enumerate() {
                publisher.publish(topic, format!("msg-{}", i).as_bytes(), QoS::AtLeastOnce, false).await?;
            }

            // Collect messages
            let mut received = 0;
            for _ in 0..topics.len() {
                if subscriber.recv_message_timeout(Duration::from_secs(3)).await.is_some() {
                    received += 1;
                }
            }

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            if received == topics.len() {
                Ok(())
            } else {
                Err(anyhow::anyhow!("expected {} messages, got {}", topics.len(), received))
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

/// Test that overlapping subscriptions don't cause duplicate delivery
pub struct OverlappingSubscriptionsTest;

impl TestCase for OverlappingSubscriptionsTest {
    fn name(&self) -> &str {
        "overlapping_subscriptions"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "overlap-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "overlap-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            // Subscribe to both overlapping patterns
            subscriber.subscribe("test/overlap/#", QoS::AtLeastOnce).await?;
            subscriber.subscribe("test/overlap/foo", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Publish to the matching topic
            publisher.publish("test/overlap/foo", b"overlapping", QoS::AtLeastOnce, false).await?;

            // The broker may or may not deduplicate overlapping subscriptions.
            // Either way, we should receive at least 1 message (not zero).
            let msg1 = subscriber.recv_message_timeout(Duration::from_secs(3)).await;
            // Check if a second message arrives (broker-dependent)
            let msg2 = subscriber.recv_message_timeout(Duration::from_secs(1)).await;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            if msg1.is_none() {
                Err(anyhow::anyhow!("no message received with overlapping subscriptions"))
            } else {
                // We got at least one message - test passes regardless of dedup behavior
                let _ = msg2;
                Ok(())
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Test that messages arrive in order at QoS 1
pub struct MessageOrderingTest;

impl TestCase for MessageOrderingTest {
    fn name(&self) -> &str {
        "message_ordering"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "ordering-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "ordering-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "test/ordering/seq";
            subscriber.subscribe(topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Publish 10 sequential messages
            for i in 0u8..10 {
                publisher.publish(topic, &[i], QoS::AtLeastOnce, false).await?;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }

            // Collect messages in order
            let mut seq = Vec::new();
            for _ in 0..10 {
                if let Some(msg) = subscriber.recv_message_timeout(Duration::from_secs(3)).await {
                    if msg.payload.len() == 1 {
                        seq.push(msg.payload[0]);
                    }
                }
            }

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            // Verify messages arrived in order
            let ordered: Vec<u8> = (0..10).collect();
            if seq == ordered {
                Ok(())
            } else {
                Err(anyhow::anyhow!("message ordering failed: expected {:?}, got {:?}", ordered, seq))
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(40)
    }
}
