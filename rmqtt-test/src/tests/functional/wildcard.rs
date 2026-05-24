//! Wildcard subscription tests (using v311 client)

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Test single-level wildcard (+)
pub struct WildcardPlusTest;

impl TestCase for WildcardPlusTest {
    fn name(&self) -> &str { "wildcard_plus" }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "wc-plus-pub",
                ctx.config.connect_timeout,
            ).await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "wc-plus-sub",
                ctx.config.connect_timeout,
            ).await?;

            let sub_topic = "test/wildcard/+/message";
            subscriber.subscribe(sub_topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Should match
            publisher.publish("test/wildcard/foo/message", b"match1", QoS::AtLeastOnce, false).await?;
            publisher.publish("test/wildcard/bar/message", b"match2", QoS::AtLeastOnce, false).await?;

            // Should NOT match
            publisher.publish("test/wildcard/foo/bar/message", b"no_match", QoS::AtLeastOnce, false).await?;

            let msg1 = subscriber.recv_message_timeout(Duration::from_secs(3)).await;
            let msg2 = subscriber.recv_message_timeout(Duration::from_secs(3)).await;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            if msg1.is_some() && msg2.is_some() {
                Ok(())
            } else {
                Err(anyhow::anyhow!("wildcard + did not match expected messages"))
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration { Duration::from_secs(15) }
}

/// Test multi-level wildcard (#)
pub struct WildcardHashTest;

impl TestCase for WildcardHashTest {
    fn name(&self) -> &str { "wildcard_hash" }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "wc-hash-pub",
                ctx.config.connect_timeout,
            ).await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "wc-hash-sub",
                ctx.config.connect_timeout,
            ).await?;

            let sub_topic = "test/wildcard/#";
            subscriber.subscribe(sub_topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // All of these should match
            publisher.publish("test/wildcard/a", b"match1", QoS::AtLeastOnce, false).await?;
            publisher.publish("test/wildcard/a/b/c", b"match2", QoS::AtLeastOnce, false).await?;

            let msg1 = subscriber.recv_message_timeout(Duration::from_secs(3)).await;
            let msg2 = subscriber.recv_message_timeout(Duration::from_secs(3)).await;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            if msg1.is_some() && msg2.is_some() {
                Ok(())
            } else {
                Err(anyhow::anyhow!("wildcard # did not match expected messages"))
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration { Duration::from_secs(15) }
}
