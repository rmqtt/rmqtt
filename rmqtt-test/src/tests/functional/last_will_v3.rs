//! MQTT v3.1 Last Will and Testament (LWT) functional tests
//!
//! Covers spec section 3.1 (CONNECT Will Flag / Will QoS / Will Retain):
//! - will fires on unclean disconnect (abnormal TCP close)
//! - will does NOT fire on a clean DISCONNECT
//! - will is published at the configured QoS
//! - will retain flag is honored (see also retain_v3.rs)

use std::time::{Duration, Instant};

use bytestring::ByteString;
use rmqtt_codec::v3::{LastWill, QoS};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Positive: the will message is published when the client disconnects
/// abnormally (TCP close without DISCONNECT).
pub struct LastWillV3Test;

impl TestCase for LastWillV3Test {
    fn name(&self) -> &str {
        "last_will_v3_fires_on_unclean"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let will_topic = format!("test/v3/lwt/fires/{uid}");

            // Subscriber that will receive the will message
            let mut sub = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("lwt-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            sub.subscribe(&will_topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Connect a client with LWT configured
            let will = LastWill {
                qos: QoS::AtLeastOnce,
                retain: false,
                topic: ByteString::from(will_topic.as_str()),
                message: bytes::Bytes::from_static(b"goodbye-v3"),
            };
            let client = crate::mqtt::v3::MqttV3Client::connect_with_options(
                &ctx.config.broker_addr,
                &format!("lwt-client-{uid}"),
                ctx.config.connect_timeout,
                true,
                60,
                Some(will),
                None,
                None,
            )
            .await?;

            // Simulate unclean disconnect by shutting down TCP without DISCONNECT
            client.abort_connection().await?;

            tokio::time::sleep(Duration::from_millis(500)).await;

            // Subscriber should receive the will message
            let msg = sub.recv_message_timeout(Duration::from_secs(5)).await;
            sub.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"goodbye-v3" && m.topic == will_topic => Ok(()),
                Some(m) => Err(anyhow::anyhow!(
                    "unexpected will message: topic={}, payload={:?}",
                    m.topic,
                    m.payload
                )),
                None => Err(anyhow::anyhow!("will message was not received")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// Negative: the will must NOT fire when the client sends a clean DISCONNECT.
pub struct LastWillV3CleanTest;

impl TestCase for LastWillV3CleanTest {
    fn name(&self) -> &str {
        "last_will_v3_no_fire_on_clean_disconnect"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let will_topic = format!("test/v3/lwt/clean/{uid}");

            let mut sub = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("lwt-clean-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            sub.subscribe(&will_topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let will = LastWill {
                qos: QoS::AtLeastOnce,
                retain: false,
                topic: ByteString::from(will_topic.as_str()),
                message: bytes::Bytes::from_static(b"should_not_appear"),
            };
            let client = crate::mqtt::v3::MqttV3Client::connect_with_options(
                &ctx.config.broker_addr,
                &format!("lwt-clean-client-{uid}"),
                ctx.config.connect_timeout,
                true,
                60,
                Some(will),
                None,
                None,
            )
            .await?;

            // Clean disconnect (sends DISCONNECT) — will must not fire
            client.disconnect().await?;

            tokio::time::sleep(Duration::from_millis(500)).await;

            let msg = sub.recv_message_timeout(Duration::from_secs(2)).await;
            sub.disconnect().await?;

            if msg.is_some() {
                Err(anyhow::anyhow!("will fired on a clean DISCONNECT"))
            } else {
                Ok(())
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// Positive: will QoS is honored — a QoS 2 will is delivered exactly once.
pub struct LastWillV3Qos2Test;

impl TestCase for LastWillV3Qos2Test {
    fn name(&self) -> &str {
        "last_will_v3_qos2"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let will_topic = format!("test/v3/lwt/qos2/{uid}");

            let mut sub = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("lwt-qos2-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            sub.subscribe(&will_topic, QoS::ExactlyOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let will = LastWill {
                qos: QoS::ExactlyOnce,
                retain: false,
                topic: ByteString::from(will_topic.as_str()),
                message: bytes::Bytes::from_static(b"will-qos2"),
            };
            let client = crate::mqtt::v3::MqttV3Client::connect_with_options(
                &ctx.config.broker_addr,
                &format!("lwt-qos2-client-{uid}"),
                ctx.config.connect_timeout,
                true,
                60,
                Some(will),
                None,
                None,
            )
            .await?;

            client.abort_connection().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            let msg = sub.recv_message_timeout(Duration::from_secs(5)).await;
            let dup = sub.recv_message_timeout(Duration::from_secs(2)).await;
            sub.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"will-qos2" && m.qos == QoS::ExactlyOnce => {
                    if dup.is_some() {
                        Err(anyhow::anyhow!("QoS 2 will delivered more than once"))
                    } else {
                        Ok(())
                    }
                }
                Some(m) => Err(anyhow::anyhow!("unexpected will: payload={:?}, qos={:?}", m.payload, m.qos)),
                None => Err(anyhow::anyhow!("QoS 2 will not received")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}
