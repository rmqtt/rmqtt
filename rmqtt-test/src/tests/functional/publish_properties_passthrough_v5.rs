//! G25 (P2): Content Type / Correlation Data / Response Topic / User
//! Properties passthrough (MQTT-3.3.2.3.2).
//!
//! A publisher sends a QoS 1 PUBLISH carrying a full property set; the
//! subscriber must receive every property unchanged (values preserved).

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Multi-property passthrough: Content Type, Correlation Data, Response
/// Topic, Message Expiry and User Properties must survive broker forwarding.
pub struct PublishPropertiesPassthroughV5Test;

impl TestCase for PublishPropertiesPassthroughV5Test {
    fn name(&self) -> &str {
        "publish_properties_passthrough_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "passthrough-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "passthrough-pub",
                ctx.config.connect_timeout,
            )
            .await?;

            subscriber.subscribe("test/v5/passthrough", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(200)).await;

            publisher
                .publish_with_properties(
                    "test/v5/passthrough",
                    b"properties-payload",
                    QoS::AtLeastOnce,
                    false,
                    Some(true),                   // payload format indicator: UTF-8
                    Some(120),                    // message expiry interval
                    Some("test/v5/reply"),        // response topic
                    Some(b"corr-001".as_slice()), // correlation data
                    Some("application/json"),     // content type
                    Some(&[("k1".to_string(), "v1".to_string()), ("k2".to_string(), "v2".to_string())]),
                )
                .await?;

            let msg = subscriber
                .recv_message_timeout(Duration::from_secs(5))
                .await
                .ok_or_else(|| anyhow::anyhow!("no message received within timeout"))?;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            if msg.payload.as_ref() != b"properties-payload" {
                return Err(anyhow::anyhow!("unexpected payload: {:?}", msg.payload));
            }
            if &*msg.topic != "test/v5/passthrough" {
                return Err(anyhow::anyhow!("unexpected topic: {:?}", msg.topic));
            }
            if !msg.is_utf8_payload {
                return Err(anyhow::anyhow!("payload format indicator lost (is_utf8_payload=false)"));
            }
            match &msg.content_type {
                Some(ct) if &**ct == "application/json" => {}
                other => return Err(anyhow::anyhow!("content type not preserved: {other:?}")),
            }
            match &msg.correlation_data {
                Some(cd) if cd.as_ref() == b"corr-001" => {}
                other => return Err(anyhow::anyhow!("correlation data not preserved: {other:?}")),
            }
            match &msg.response_topic {
                Some(rt) if &**rt == "test/v5/reply" => {}
                other => return Err(anyhow::anyhow!("response topic not preserved: {other:?}")),
            }
            // Message Expiry Interval is decremented by the broker with the
            // time spent waiting, so it must stay in (0, 120].
            match msg.message_expiry_interval {
                Some(expiry) if expiry.get() > 0 && expiry.get() <= 120 => {}
                other => return Err(anyhow::anyhow!("message expiry not preserved/forwarded: {other:?}")),
            }
            let mut props = msg.user_properties.clone();
            props.sort();
            let mut expected = vec![
                (bytestring::ByteString::from("k1"), bytestring::ByteString::from("v1")),
                (bytestring::ByteString::from("k2"), bytestring::ByteString::from("v2")),
            ];
            expected.sort();
            if props != expected {
                return Err(anyhow::anyhow!(
                    "user properties not preserved: {props:?} (expected {expected:?})"
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
