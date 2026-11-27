//! G26 (P2): a single SUBSCRIBE carrying multiple topic filters with mixed
//! QoS levels (MQTT-3.8.4).
//!
//! Each filter must be granted its own QoS (SUBACK statuses in filter order)
//! and each delivery must use the granted QoS of its matching subscription.

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use rmqtt_codec::v5::SubscribeAckReason;

/// One SUBSCRIBE with three filters at QoS 0/1/2; SUBACK must grant each
/// filter independently and deliveries must honour the granted QoS.
pub struct SubscribeMultiFilterMixedV5Test;

impl TestCase for SubscribeMultiFilterMixedV5Test {
    fn name(&self) -> &str {
        "subscribe_multi_filter_mixed_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "multifilter-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "multifilter-pub",
                ctx.config.connect_timeout,
            )
            .await?;

            let ack = subscriber
                .subscribe_many(&[
                    ("test/v5/mf/a", QoS::AtMostOnce),
                    ("test/v5/mf/b", QoS::AtLeastOnce),
                    ("test/v5/mf/c", QoS::ExactlyOnce),
                ])
                .await?;

            // SUBACK statuses must be granted per filter, in filter order.
            let expected_codes = [
                SubscribeAckReason::GrantedQos0,
                SubscribeAckReason::GrantedQos1,
                SubscribeAckReason::GrantedQos2,
            ];
            if ack.status.len() != 3 {
                return Err(anyhow::anyhow!(
                    "SUBACK carried {} return codes, expected 3 (one per filter)",
                    ack.status.len()
                ));
            }
            for (i, (got, want)) in ack.status.iter().zip(expected_codes.iter()).enumerate() {
                if got != want {
                    return Err(anyhow::anyhow!(
                        "filter #{i} granted 0x{:02X}, expected 0x{:02X}",
                        *got as u8,
                        *want as u8
                    ));
                }
            }

            tokio::time::sleep(Duration::from_millis(200)).await;

            // Publish to each topic (publisher uses QoS 2, the max); delivery
            // must be downgraded to each subscription's granted QoS.
            publisher.publish("test/v5/mf/a", b"mf-a", QoS::ExactlyOnce, false).await?;
            publisher.publish("test/v5/mf/b", b"mf-b", QoS::ExactlyOnce, false).await?;
            publisher.publish("test/v5/mf/c", b"mf-c", QoS::ExactlyOnce, false).await?;

            let mut seen: Vec<(String, QoS)> = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(6);
            while seen.len() < 3 && Instant::now() < deadline {
                match subscriber.recv_message_timeout(Duration::from_secs(2)).await {
                    Some(msg) => seen.push((msg.topic.to_string(), msg.qos)),
                    None => break,
                }
            }

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            let mut topics: Vec<&str> = seen.iter().map(|(t, _)| t.as_str()).collect();
            topics.sort();
            if topics != ["test/v5/mf/a", "test/v5/mf/b", "test/v5/mf/c"] {
                return Err(anyhow::anyhow!("expected deliveries on all 3 filters, got {topics:?}"));
            }

            let want: &[(&str, QoS)] = &[
                ("test/v5/mf/a", QoS::AtMostOnce),
                ("test/v5/mf/b", QoS::AtLeastOnce),
                ("test/v5/mf/c", QoS::ExactlyOnce),
            ];
            for (topic, want_qos) in want {
                let got = seen.iter().find(|(t, _)| t == topic).map(|(_, q)| *q);
                if got.as_ref() != Some(want_qos) {
                    return Err(anyhow::anyhow!(
                        "delivery on {topic} had QoS {:?}, expected {want_qos:?} (granted QoS must be honoured)",
                        got
                    ));
                }
            }
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}
