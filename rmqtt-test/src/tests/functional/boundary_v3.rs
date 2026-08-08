//! MQTT v3.1 Boundary & edge-case tests
//!
//! Covers payload / topic / keep-alive boundary conditions:
//! - empty payload publish
//! - large payload publish
//! - very long topic name
//! - special characters in topic names
//! - maximum keep alive value (65535)
//! - multiple rapid subscriptions

use std::time::{Duration, Instant};

use rmqtt_codec::v3::QoS;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Boundary: an empty-payload QoS 0 message is delivered (topic remains).
pub struct BoundaryV3EmptyPayloadTest;

impl TestCase for BoundaryV3EmptyPayloadTest {
    fn name(&self) -> &str {
        "boundary_v3_empty_payload"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let publisher = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-bnd-empty-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-bnd-empty-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = format!("test/v3/boundary/empty/{uid}");
            subscriber.subscribe(&topic, QoS::AtMostOnce).await?;
            tokio::time::sleep(Duration::from_millis(1000)).await;

            publisher.publish(&topic, b"", QoS::AtMostOnce, false).await?;
            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.payload.is_empty() && m.topic == topic => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected message: payload={:?}", m.payload)),
                None => Err(anyhow::anyhow!("empty-payload message was not delivered")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Boundary: a 256 KiB payload is delivered intact.
pub struct BoundaryV3LargePayloadTest;

impl TestCase for BoundaryV3LargePayloadTest {
    fn name(&self) -> &str {
        "boundary_v3_large_payload"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let publisher = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-bnd-large-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-bnd-large-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = format!("test/v3/boundary/large/{uid}");
            subscriber.subscribe(&topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(1000)).await;

            let payload = vec![0xABu8; 256 * 1024];
            publisher.publish(&topic, &payload, QoS::AtLeastOnce, false).await?;
            let msg = subscriber.recv_message_timeout(Duration::from_secs(10)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == payload.as_slice() => Ok(()),
                Some(m) => Err(anyhow::anyhow!(
                    "large payload corrupted: got {} bytes, expected {}",
                    m.payload.len(),
                    payload.len()
                )),
                None => Err(anyhow::anyhow!("large payload was not delivered")),
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

/// Boundary: a long topic name (up to ~1 KiB) works end to end.
pub struct BoundaryV3LongTopicTest;

impl TestCase for BoundaryV3LongTopicTest {
    fn name(&self) -> &str {
        "boundary_v3_long_topic"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let publisher = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-bnd-topic-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-bnd-topic-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;

            // Build a ~900-char topic name (well below the 65535 limit)
            let long_level = "x".repeat(300);
            let topic = format!("test/{long_level}/{long_level}/{long_level}");
            assert!(topic.len() > 900);
            subscriber.subscribe(&topic, QoS::AtMostOnce).await?;
            tokio::time::sleep(Duration::from_millis(1000)).await;

            publisher.publish(&topic, b"long-topic", QoS::AtMostOnce, false).await?;
            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.topic == topic && m.payload.as_ref() == b"long-topic" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected message for long topic: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("long topic message was not delivered")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Boundary: topics with special characters (spaces, unicode, symbols) work.
pub struct BoundaryV3SpecialCharsTopicTest;

impl TestCase for BoundaryV3SpecialCharsTopicTest {
    fn name(&self) -> &str {
        "boundary_v3_special_chars_topic"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let publisher = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-bnd-sp-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-bnd-sp-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = format!("test/v3/special/space level/中文/emoji🎯/{uid}");
            subscriber.subscribe(&topic, QoS::AtMostOnce).await?;
            tokio::time::sleep(Duration::from_millis(1000)).await;

            publisher.publish(&topic, b"special", QoS::AtMostOnce, false).await?;
            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.topic == topic && m.payload.as_ref() == b"special" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected special-char message: topic={}", m.topic)),
                None => Err(anyhow::anyhow!("special-char topic message was not delivered")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Boundary: keep alive = 65535 (the max u16) is accepted and does not
/// immediately disconnect the client.
pub struct BoundaryV3MaxKeepAliveTest;

impl TestCase for BoundaryV3MaxKeepAliveTest {
    fn name(&self) -> &str {
        "boundary_v3_max_keepalive"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v3::MqttV3Client::connect_with_options(
                &ctx.config.broker_addr,
                "v3-bnd-ka-max",
                ctx.config.connect_timeout,
                true,
                u16::MAX, // 65535 seconds
                None,
                None,
                None,
            )
            .await?;
            assert!(client.is_connected());
            // Give the broker a moment; the connection must stay up
            tokio::time::sleep(Duration::from_secs(2)).await;
            assert!(client.is_connected(), "max keep alive should not disconnect");
            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Boundary: rapid subscribe/unsubscribe cycles do not break the session.
pub struct BoundaryV3RapidSubscribeTest;

impl TestCase for BoundaryV3RapidSubscribeTest {
    fn name(&self) -> &str {
        "boundary_v3_rapid_subscribe"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let mut client = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-bnd-rapid-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;

            for i in 0..20 {
                let topic = format!("test/v3/rapid/{uid}/{i}");
                client.subscribe(&topic, QoS::AtMostOnce).await?;
                client.unsubscribe(&topic).await?;
            }
            assert!(client.is_connected(), "session must survive rapid subscribe/unsubscribe");
            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
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
