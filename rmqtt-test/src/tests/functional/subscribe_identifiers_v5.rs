//! MQTT 5.0 Subscription Identifiers test
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

pub struct SubscribeIdentifiersV5Test;
impl TestCase for SubscribeIdentifiersV5Test {
    fn name(&self) -> &str {
        "subscribe_identifiers_v5"
    }
    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let client = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "subid-v5",
                ctx.config.connect_timeout,
            )
            .await?;
            let ack = client.connack();
            let _ = ack.subscription_identifiers_available;
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

// ---------------------------------------------------------------------------
// P1 gap-analysis additions (designs/mqtt-5.0-standalone-test-gap-analysis.md)
// ---------------------------------------------------------------------------

/// G21: re-subscribing to the same filter with a different Subscription
/// Identifier must replace the stored one, so subsequent deliveries carry
/// the new identifier (and only the new one). [MQTT-3.8.2.1.2 / MQTT-3.8.4-3]
pub struct SubscribeIdentifiersV5UpdateTest;

impl TestCase for SubscribeIdentifiersV5UpdateTest {
    fn name(&self) -> &str {
        "subscribe_identifiers_v5_update"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let topic = "test/v5/subid/upd";

            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "subid-upd",
                ctx.config.connect_timeout,
            )
            .await?;

            // Subscribe with id 1, then re-subscribe with id 2.
            subscriber.subscribe_with_id(topic, QoS::AtLeastOnce, 1).await?;
            subscriber.subscribe_with_id(topic, QoS::AtLeastOnce, 2).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "subid-upd-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            publisher.publish(topic, b"which-id", QoS::AtMostOnce, false).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            let m = match msg {
                Some(m) if m.payload.as_ref() == b"which-id" => m,
                Some(m) => return Err(anyhow::anyhow!("unexpected payload: {:?}", m.payload)),
                None => return Err(anyhow::anyhow!("no message delivered")),
            };

            if m.subscription_ids.len() != 1 || m.subscription_ids[0].get() != 2 {
                return Err(anyhow::anyhow!(
                    "delivery carried Subscription Identifiers {:?}, expected [2] \
                     (the re-subscribe must replace id 1 with id 2)",
                    m.subscription_ids.iter().map(|v| v.get()).collect::<Vec<_>>()
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
        Duration::from_secs(20)
    }
}
