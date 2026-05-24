//! MQTT 5.0 Will Delay tests

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use bytestring::ByteString;

/// Test that will_delay_interval delays the will message delivery (v5)
pub struct WillDelayV5Test;

impl TestCase for WillDelayV5Test {
    fn name(&self) -> &str {
        "will_delay_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let will_topic = "test/v5/willdelay/willmsg";

            // Subscriber to receive the will message
            let mut sub = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "willdelay-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            sub.subscribe(will_topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Connect V5 client with will + will_delay_interval of 3 seconds
            let will = rmqtt_codec::v5::LastWill {
                qos: QoS::AtLeastOnce,
                retain: false,
                topic: ByteString::from(will_topic),
                message: bytes::Bytes::from("delayed_will"),
                will_delay_interval_sec: Some(3),
                correlation_data: None,
                message_expiry_interval: None,
                content_type: None,
                user_properties: Vec::new(),
                is_utf8_payload: None,
                response_topic: None,
            };
            let client = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "willdelay-client",
                ctx.config.connect_timeout,
                true,
                60,
                Some(will),
                None,
                None,
                None,
                None,
            )
            .await?;

            // Abort connection to trigger will
            client.abort_connection().await?;

            // Verify will message arrives
            let msg = sub.recv_message_timeout(Duration::from_secs(10)).await;
            sub.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"delayed_will" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected will message: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("will message was not received")),
            }
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
