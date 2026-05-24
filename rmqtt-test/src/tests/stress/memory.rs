//! Memory and pressure stress tests

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Test publishing many retained messages to flood broker memory
pub struct RetainFloodTest;

impl TestCase for RetainFloodTest {
    fn name(&self) -> &str {
        "stress_retain_flood"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "retain-flood-pub",
                ctx.config.connect_timeout,
            )
            .await?;

            // Publish 100 retained messages on different topics
            for i in 0..100 {
                publisher
                    .publish(
                        &format!("test/retain/flood/{}", i),
                        format!("retained-{}", i).as_bytes(),
                        QoS::AtLeastOnce,
                        true,
                    )
                    .await?;
            }

            publisher.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(200)).await;

            // Subscribe to all 100 topics and verify retention
            let mut sub = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "retain-flood-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            sub.subscribe("test/retain/flood/#", QoS::AtLeastOnce).await?;

            let mut received = 0;
            for _ in 0..100 {
                if sub.recv_message_timeout(Duration::from_secs(3)).await.is_some() {
                    received += 1;
                }
            }

            sub.disconnect().await?;

            // Expect at least 90% of retained messages present
            if received >= 90 {
                Ok(())
            } else {
                Err(anyhow::anyhow!("retain flood: only {}/100 messages retained", received))
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "stress", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "stress", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(120)
    }
}

/// Test subscribing to many topics and verifying delivery
pub struct SubscriptionStressTest;

impl TestCase for SubscriptionStressTest {
    fn name(&self) -> &str {
        "stress_subscription_mass"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "sub-stress-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "sub-stress-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            // Subscribe to 50 unique topics
            let topic_count = 50;
            for i in 0..topic_count {
                subscriber.subscribe(&format!("test/stress/sub/{}", i), QoS::AtLeastOnce).await?;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;

            // Publish to 30 of those topics
            let publish_count = 30;
            for i in 0..publish_count {
                publisher
                    .publish(
                        &format!("test/stress/sub/{}", i),
                        format!("stress-{}", i).as_bytes(),
                        QoS::AtLeastOnce,
                        false,
                    )
                    .await?;
            }

            // Collect messages
            let mut received = 0;
            for _ in 0..publish_count {
                if subscriber.recv_message_timeout(Duration::from_secs(5)).await.is_some() {
                    received += 1;
                }
            }

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            if received >= publish_count * 90 / 100 {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "subscription stress: only {}/{} messages received",
                    received,
                    publish_count
                ))
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "stress", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "stress", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(120)
    }
}
