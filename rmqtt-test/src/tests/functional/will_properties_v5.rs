//! MQTT 5.0 Will Message properties (G16)
//!
//! designs/mqtt-5.0-standalone-test-gap-analysis.md
//!
//! A Will Message's properties (Message Expiry Interval, Content Type,
//! Payload Format Indicator, User Properties) MUST be delivered with the Will
//! Message when it is published after an unclean disconnect.
use std::time::{Duration, Instant};

use bytes::Bytes;
use bytestring::ByteString;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// G16: Will properties are forwarded verbatim with the Will Message.
pub struct WillPropertiesV5DeliveryTest;

impl TestCase for WillPropertiesV5DeliveryTest {
    fn name(&self) -> &str {
        "will_properties_v5_delivery"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let mut observer = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "willprops-obs",
                ctx.config.connect_timeout,
            )
            .await?;
            let will_topic = "test/v5/willprops";
            observer.subscribe(will_topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let will = rmqtt_codec::v5::LastWill {
                qos: QoS::AtLeastOnce,
                retain: false,
                topic: ByteString::from(will_topic),
                message: Bytes::from("will-with-props"),
                will_delay_interval_sec: None,
                correlation_data: None,
                message_expiry_interval: std::num::NonZeroU32::new(120),
                content_type: Some(ByteString::from("application/json")),
                user_properties: vec![(ByteString::from("wk"), ByteString::from("wv"))],
                is_utf8_payload: Some(true),
                response_topic: None,
            };
            let willer = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "willprops-client",
                ctx.config.connect_timeout,
                true,
                60,
                Some(will),
                None,
                None,
                None,
                None,
                None,
            )
            .await?;

            // Unclean disconnect publishes the Will immediately (no delay set).
            willer.abort_connection().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            let msg = observer.recv_message_timeout(Duration::from_secs(5)).await;
            observer.disconnect().await?;

            let m = match msg {
                Some(m) if m.payload.as_ref() == b"will-with-props" => m,
                Some(m) => return Err(anyhow::anyhow!("unexpected will payload: {:?}", m.payload)),
                None => return Err(anyhow::anyhow!("will message was not received")),
            };

            // Payload Format Indicator = UTF-8
            if !m.is_utf8_payload {
                return Err(anyhow::anyhow!("Payload Format Indicator was not forwarded"));
            }
            // Content Type
            match &m.content_type {
                Some(ct) if &**ct == "application/json" => {}
                other => return Err(anyhow::anyhow!("Content Type not forwarded correctly: {:?}", other)),
            }
            // Message Expiry Interval
            match m.message_expiry_interval {
                Some(exp) if exp.get() <= 120 => {}
                other => return Err(anyhow::anyhow!("Message Expiry Interval not forwarded: {:?}", other)),
            }
            // User Properties
            let has_user_prop = m.user_properties.iter().any(|(k, v)| &**k == "wk" && &**v == "wv");
            if !has_user_prop {
                return Err(anyhow::anyhow!("Will User Properties not forwarded: {:?}", m.user_properties));
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
