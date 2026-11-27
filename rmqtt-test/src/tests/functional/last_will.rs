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

/// Positive: a QoS 2 will is delivered exactly once (v3.1.1)
pub struct LastWillV311Qos2Test;

impl TestCase for LastWillV311Qos2Test {
    fn name(&self) -> &str {
        "last_will_v311_qos2"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let will_topic = format!("test/v311/lwt/qos2/{uid}");

            let mut sub = crate::mqtt::v311::MqttV311Client::connect(
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
            let client = crate::mqtt::v311::MqttV311Client::connect_with_options(
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
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// Positive: the will is published when the client is disconnected by the
/// broker for a keep-alive timeout (an abnormal disconnect). [MQTT-3.1.2-8]
pub struct LastWillV311KeepAliveTimeoutTest;

impl TestCase for LastWillV311KeepAliveTimeoutTest {
    fn name(&self) -> &str {
        "last_will_v311_keepalive_timeout"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let will_topic = format!("test/v311/lwt/katimeout/{uid}");

            let mut sub = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &format!("lwt-ka-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            sub.subscribe(&will_topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let will = LastWill {
                qos: QoS::AtLeastOnce,
                retain: false,
                topic: ByteString::from(will_topic.as_str()),
                message: bytes::Bytes::from_static(b"will-ka-timeout"),
            };
            // keep_alive = 2s → effective window = 2 + 3 = 5s (value < 6
            // bumped by +3 in DefaultFitter::keep_alive). Stay silent > 8s.
            let client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                &format!("lwt-ka-client-{uid}"),
                ctx.config.connect_timeout,
                true,
                2,
                Some(will),
                None,
                None,
            )
            .await?;
            assert!(client.is_connected());

            // Stay silent — the broker must drop us, firing the will
            tokio::time::sleep(Duration::from_secs(8)).await;
            let _ = client.disconnect().await;

            let msg = sub.recv_message_timeout(Duration::from_secs(5)).await;
            sub.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"will-ka-timeout" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected will: {:?}", m.payload)),
                None => {
                    Err(anyhow::anyhow!("will not fired after keep-alive timeout disconnect [MQTT-3.1.2-8]"))
                }
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

// ---------------------------------------------------------------------------
// P1 conformance gap fill (G20): Will QoS 0/1 + no-fire on rejected connect
// ---------------------------------------------------------------------------

/// Positive: a Will published at QoS 0 fires on an unclean disconnect.
/// [MQTT-3.1.2-8/9] [MQTT-3.1.2-13]
pub struct LastWillV311Qos0Test;

impl TestCase for LastWillV311Qos0Test {
    fn name(&self) -> &str {
        "last_will_v311_qos0"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let will_topic = format!("test/v311/lwt/qos0/{uid}");

            let mut sub = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &format!("lwt-q0-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            sub.subscribe(&will_topic, QoS::AtMostOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let will = LastWill {
                qos: QoS::AtMostOnce,
                retain: false,
                topic: ByteString::from(will_topic.as_str()),
                message: bytes::Bytes::from_static(b"will-qos0"),
            };
            let client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                &format!("lwt-q0-client-{uid}"),
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
            let _ = sub.disconnect().await;

            match msg {
                Some(m) if m.payload.as_ref() == b"will-qos0" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected will: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("QoS 0 will not fired on unclean disconnect")),
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

/// Positive: a Will published at QoS 1 fires on an unclean disconnect.
/// [MQTT-3.1.2-8/9]
pub struct LastWillV311Qos1Test;

impl TestCase for LastWillV311Qos1Test {
    fn name(&self) -> &str {
        "last_will_v311_qos1"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let will_topic = format!("test/v311/lwt/qos1/{uid}");

            let mut sub = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &format!("lwt-q1-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            sub.subscribe(&will_topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let will = LastWill {
                qos: QoS::AtLeastOnce,
                retain: false,
                topic: ByteString::from(will_topic.as_str()),
                message: bytes::Bytes::from_static(b"will-qos1"),
            };
            let client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                &format!("lwt-q1-client-{uid}"),
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
            let _ = sub.disconnect().await;

            match msg {
                Some(m) if m.payload.as_ref() == b"will-qos1" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected will: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("QoS 1 will not fired on unclean disconnect")),
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

/// Negative: a Will must NOT fire when the CONNECT itself is rejected by the
/// broker (no session is ever established). [MQTT-3.1.2-10]
///
/// Uses the well-defined rejection path: empty Client Identifier with
/// Clean Session = 0 → CONNACK 0x02 (Identifier Rejected) [MQTT-3.1.3-6].
pub struct WillNotFireOnRejectedConnectTest;

impl TestCase for WillNotFireOnRejectedConnectTest {
    fn name(&self) -> &str {
        "will_not_fire_on_rejected_connect"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let will_topic = format!("test/v311/lwt/rejected/{uid}");

            let mut sub = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &format!("lwt-rej-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            sub.subscribe(&will_topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let will = LastWill {
                qos: QoS::AtLeastOnce,
                retain: false,
                topic: ByteString::from(will_topic.as_str()),
                message: bytes::Bytes::from_static(b"must-not-appear"),
            };
            // Empty client id + Clean Session = 0 → the broker MUST reject
            // with CONNACK 0x02 and close the connection [MQTT-3.1.3-6].
            let res = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "", // empty client id
                ctx.config.connect_timeout,
                false, // clean_session = false
                60,
                Some(will),
                None,
                None,
            )
            .await;
            assert!(res.is_err(), "expected the CONNECT to be rejected (empty client id + clean session 0)");

            // The will must not have been published.
            let msg = sub.recv_message_timeout(Duration::from_secs(2)).await;
            let _ = sub.disconnect().await;

            match msg {
                None => Ok(()),
                Some(m) => Err(anyhow::anyhow!(
                    "will fired although the CONNECT was rejected: {:?} [MQTT-3.1.2-10]",
                    m.payload
                )),
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

/// The Will Message payload is binary data, NOT a UTF-8 string: a CONNECT
/// carrying non-UTF-8 will payload bytes must be accepted (unlike the will
/// topic, which is a UTF-8 string), and the payload must be delivered
/// verbatim when the will fires. [MQTT-1.5.3 / test spec §13, G31]
pub struct LastWillV311InvalidUtf8PayloadTest;

impl TestCase for LastWillV311InvalidUtf8PayloadTest {
    fn name(&self) -> &str {
        "last_will_v311_invalid_utf8_payload"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let mut sub = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "lwt-binary-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            let will_topic = "test/v311/lwt/binary";
            sub.subscribe(will_topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Binary (non-UTF-8) will payload: overlong, a surrogate half and
            // a lone continuation byte.
            let binary_payload: Vec<u8> = vec![0xC0, 0x80, 0xED, 0xA0, 0x80, 0x80, 0xFF];
            let will = LastWill {
                qos: QoS::AtLeastOnce,
                retain: false,
                topic: ByteString::from(will_topic),
                message: bytes::Bytes::from(binary_payload.clone()),
            };
            // The broker must NOT reject the CONNECT because of the binary
            // will payload (it is not UTF-8 constrained).
            let client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "lwt-binary-client",
                ctx.config.connect_timeout,
                true,
                60,
                Some(will),
                None,
                None,
            )
            .await?;

            // Unclean disconnect fires the will.
            client.abort_connection().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            let msg = sub.recv_message_timeout(Duration::from_secs(5)).await;
            sub.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == binary_payload.as_slice() && m.topic == will_topic => Ok(()),
                Some(m) => Err(anyhow::anyhow!(
                    "will payload corrupted or topic mismatch: topic={}, payload={:?}",
                    m.topic,
                    m.payload
                )),
                None => Err(anyhow::anyhow!("binary will message was not received")),
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
