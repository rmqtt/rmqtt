//! MQTT 5.0 Retain Handling subscription option tests

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use rmqtt_codec::v5::RetainHandling;

/// Test RetainHandling::NoAtSubscribe (value 2) - no retained messages on subscribe (v5)
pub struct RetainHandlingNoAtSubscribeV5Test;

impl TestCase for RetainHandlingNoAtSubscribeV5Test {
    fn name(&self) -> &str {
        "retain_handling_no_at_subscribe_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let topic = "test/v5/retain/noatsubscribe";

            // Publish a retained message
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "retain-noatsub-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            publisher.publish(topic, b"retained_msg", QoS::AtLeastOnce, true).await?;
            publisher.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(200)).await;

            // Subscribe with RetainHandling::NoAtSubscribe - should NOT receive retained
            let mut sub = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "retain-noatsub-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            sub.subscribe_with_options(topic, QoS::AtLeastOnce, false, false, RetainHandling::NoAtSubscribe)
                .await?;

            let msg = sub.recv_message_timeout(Duration::from_secs(2)).await;
            sub.disconnect().await?;

            if msg.is_some() {
                return Err(anyhow::anyhow!(
                    "received retained message despite RetainHandling::NoAtSubscribe"
                ));
            }

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

/// Test RetainHandling::AtSubscribeNew (value 1) - send retained if new subscription (v5)
pub struct RetainHandlingNewV5Test;

impl TestCase for RetainHandlingNewV5Test {
    fn name(&self) -> &str {
        "retain_handling_new_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let topic = "test/v5/retain/new";

            // Publish a retained message
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "retain-new-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            publisher.publish(topic, b"retained_new", QoS::AtLeastOnce, true).await?;
            publisher.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(200)).await;

            // Subscribe with AtSubscribeNew - should receive retained
            let mut sub = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "retain-new-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            sub.subscribe_with_options(topic, QoS::AtLeastOnce, false, false, RetainHandling::AtSubscribeNew)
                .await?;

            let msg = sub.recv_message_timeout(Duration::from_secs(3)).await;
            sub.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"retained_new" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected retained msg: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("no retained message received despite AtSubscribeNew")),
            }
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
