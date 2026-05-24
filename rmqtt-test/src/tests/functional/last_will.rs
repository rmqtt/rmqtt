//! Last Will and Testament (LWT) functional tests

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use bytestring::ByteString;
use rmqtt_codec::v3::LastWill;

/// Test that Last Will fires when client disconnects unexpectedly (v3.1.1)
pub struct LastWillV311Test;

impl TestCase for LastWillV311Test {
    fn name(&self) -> &str {
        "last_will_v311_fires"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            // Subscriber that will receive the will message
            let mut sub = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "lwt-sub-v311",
                ctx.config.connect_timeout,
            )
            .await?;
            let will_topic = "test/v311/lwt/willmsg";
            sub.subscribe(will_topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Connect a client with LWT configured
            let will = LastWill {
                qos: QoS::AtLeastOnce,
                retain: false,
                topic: ByteString::from(will_topic),
                message: bytes::Bytes::from("goodbye"),
            };
            let client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "lwt-client-v311",
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
                Some(m) if m.payload.as_ref() == b"goodbye" && m.topic == will_topic => Ok(()),
                Some(m) => Err(anyhow::anyhow!(
                    "unexpected will message: topic={}, payload={:?}",
                    m.topic,
                    m.payload
                )),
                None => Err(anyhow::anyhow!("will message was not received")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// Test that LWT does NOT fire on clean disconnect (v3.1.1)
pub struct LastWillV311CleanTest;

impl TestCase for LastWillV311CleanTest {
    fn name(&self) -> &str {
        "last_will_v311_no_fire_on_clean_disconnect"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let mut sub = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "lwt-clean-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            let will_topic = "test/v311/lwt/cleanwill";
            sub.subscribe(will_topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let will = LastWill {
                qos: QoS::AtLeastOnce,
                retain: false,
                topic: ByteString::from(will_topic),
                message: bytes::Bytes::from("should_not_appear"),
            };
            let client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "lwt-clean-client",
                ctx.config.connect_timeout,
                true,
                60,
                Some(will),
                None,
                None,
            )
            .await?;

            // Clean disconnect - LWT should NOT fire
            client.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Should NOT receive the will message
            let msg = sub.recv_message_timeout(Duration::from_secs(3)).await;
            sub.disconnect().await?;

            if msg.is_some() {
                Err(anyhow::anyhow!("LWT fired on clean disconnect, which should not happen"))
            } else {
                Ok(())
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// Test LWT with MQTT 5.0 client
pub struct LastWillV5Test;

impl TestCase for LastWillV5Test {
    fn name(&self) -> &str {
        "last_will_v5_fires"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let mut sub = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "lwt-sub-v5",
                ctx.config.connect_timeout,
            )
            .await?;
            let will_topic = "test/v5/lwt/willmsg";
            sub.subscribe(will_topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let will = rmqtt_codec::v5::LastWill {
                qos: QoS::AtLeastOnce,
                retain: false,
                topic: ByteString::from(will_topic),
                message: bytes::Bytes::from("goodbye-v5"),
                will_delay_interval_sec: None,
                correlation_data: None,
                message_expiry_interval: None,
                content_type: None,
                user_properties: Vec::new(),
                is_utf8_payload: None,
                response_topic: None,
            };
            let client = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "lwt-client-v5",
                ctx.config.connect_timeout,
                true,
                60,
                Some(will),
                None,
                None,
                None,
            )
            .await?;

            // Unclean disconnect via abort - close TCP without DISCONNECT
            client.abort_connection().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            let msg = sub.recv_message_timeout(Duration::from_secs(5)).await;
            sub.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"goodbye-v5" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected v5 will message: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("v5 will message was not received")),
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

/// Test LWT fires on client crash via abort_connection
pub struct LastWillUncleanTest;

impl TestCase for LastWillUncleanTest {
    fn name(&self) -> &str {
        "last_will_unclean_disconnect"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let mut sub = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "lwt-unclean-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            let will_topic = "test/v311/lwt/unclean";
            sub.subscribe(will_topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let will = LastWill {
                qos: QoS::AtLeastOnce,
                retain: false,
                topic: ByteString::from(will_topic),
                message: bytes::Bytes::from("crash_was_here"),
            };
            let client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "lwt-crash-client",
                ctx.config.connect_timeout,
                true,
                60,
                Some(will),
                None,
                None,
            )
            .await?;

            // Abort connection to simulate crash (no DISCONNECT sent)
            client.abort_connection().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            let msg = sub.recv_message_timeout(Duration::from_secs(5)).await;
            sub.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"crash_was_here" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected crash will message: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("crash will message was not received")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}
