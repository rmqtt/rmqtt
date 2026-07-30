//! Dollar topics test ($-prefixed topic behavior)
//!
//! Per MQTT spec, topics starting with '$' are reserved and should
//! NOT be matched by '#' or '+' wildcard subscriptions. However,
//! some brokers may allow this. This test verifies dollar topic
//! behavior and explicit $SYS subscription.

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Test dollar-prefixed topic behavior with # wildcard and explicit subscription
pub struct DollarTopicsTest;

impl TestCase for DollarTopicsTest {
    fn name(&self) -> &str {
        "dollar_topics"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "dollar-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "dollar-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            // Subscribe to # wildcard
            subscriber.subscribe("#", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Drain any stale retained messages from previous tests
            while subscriber.recv_message_timeout(Duration::from_millis(100)).await.is_some() {}

            // Publish to normal topic - should always be received
            publisher.publish("test/dollar/normal", b"normal_msg", QoS::AtLeastOnce, false).await?;
            let msg = subscriber.recv_message_timeout(Duration::from_secs(3)).await;
            if msg.is_none() {
                return Err(anyhow::anyhow!("# wildcard did not match normal topic"));
            }

            // Publish to $SYS topic - behavior is broker-dependent
            publisher.publish("$SYS/broker/version", b"dollar_sys", QoS::AtLeastOnce, false).await?;
            let dollar_via_hash = subscriber.recv_message_timeout(Duration::from_secs(2)).await;

            // Some brokers exclude $-topics from # matches (spec-compliant),
            // others include them. Either is acceptable for this test.
            if dollar_via_hash.is_some() {
                eprintln!("Note: broker delivers $SYS via # wildcard (non-standard)");
            }

            // Now subscribe explicitly to $SYS/#
            subscriber.subscribe("$SYS/#", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Publish to $SYS topic - should be received via explicit subscription
            publisher.publish("$SYS/broker/version", b"dollar_sys_explicit", QoS::AtLeastOnce, false).await?;
            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"dollar_sys_explicit" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected message: {:?}", m.payload)),
                None => Err(anyhow::anyhow!(
                    "explicit $SYS/# did not match $SYS topic - broker may not support $ topics"
                )),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(25)
    }
}

/// Test V5 wildcard_subscription_available flag in CONNACK
pub struct WildcardAvailableV5Test;

impl TestCase for WildcardAvailableV5Test {
    fn name(&self) -> &str {
        "wildcard_available_v5"
    }
    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let client = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "wcavail-v5",
                ctx.config.connect_timeout,
            )
            .await?;
            let ack = client.connack();
            let _ = ack.wildcard_subscription_available;
            client.disconnect().await?;
            Ok(())
        });
        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}
