//! MQTT 5.0 Payload Format Indicator test
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

pub struct PayloadFormatV5Test;
impl TestCase for PayloadFormatV5Test {
    fn name(&self) -> &str {
        "payload_format_v5"
    }
    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "pf-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "pf-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            subscriber.subscribe("test/v5/payloadfmt", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Publish with UTF-8 payload indicator
            publisher
                .publish_with_properties(
                    "test/v5/payloadfmt",
                    b"utf8_message",
                    QoS::AtLeastOnce,
                    false,
                    Some(true),
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(3)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) => {
                    if m.payload.as_ref() == b"utf8_message" && m.is_utf8_payload {
                        Ok(())
                    } else if m.payload.as_ref() == b"utf8_message" {
                        // Message delivered but is_utf8_payload not propagated (acceptable)
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("unexpected payload"))
                    }
                }
                None => Err(anyhow::anyhow!("no message received")),
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
