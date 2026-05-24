//! Boundary and edge case tests (v3.1.1 client)

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Test connecting with the maximum allowed client ID length (23 chars per MQTT spec)
pub struct MaxClientIdTest;

impl TestCase for MaxClientIdTest {
    fn name(&self) -> &str {
        "boundary_max_client_id"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let client_id = "A".repeat(23);
            let client = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &client_id,
                ctx.config.connect_timeout,
            )
            .await?;
            client.disconnect().await?;
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Test subscribing and publishing with a 200-character topic
pub struct LongTopicTest;

impl TestCase for LongTopicTest {
    fn name(&self) -> &str {
        "boundary_long_topic"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "boundary-long-topic-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "boundary-long-topic-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e/a/b/c/d/e";
            subscriber.subscribe(topic, QoS::AtLeastOnce).await?;

            tokio::time::sleep(Duration::from_millis(100)).await;

            publisher.publish(topic, b"long topic msg", QoS::AtLeastOnce, false).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) => {
                    if m.payload.as_ref() == b"long topic msg" && m.topic.as_bytes() == topic.as_bytes() {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!(
                            "unexpected message: topic={}, payload={:?}",
                            m.topic,
                            m.payload
                        ))
                    }
                }
                None => Err(anyhow::anyhow!("no message received within timeout")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Test publishing messages with empty (zero-byte) payload at all QoS levels
pub struct EmptyPayloadTest;

impl TestCase for EmptyPayloadTest {
    fn name(&self) -> &str {
        "boundary_empty_payload"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            // QoS 0
            {
                let publisher = crate::mqtt::v311::MqttV311Client::connect(
                    &ctx.config.broker_addr,
                    "boundary-empty-pub-qos0",
                    ctx.config.connect_timeout,
                )
                .await?;
                let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                    &ctx.config.broker_addr,
                    "boundary-empty-sub-qos0",
                    ctx.config.connect_timeout,
                )
                .await?;

                let topic = "test/boundary/empty/qos0";
                subscriber.subscribe(topic, QoS::AtMostOnce).await?;

                tokio::time::sleep(Duration::from_millis(100)).await;

                publisher.publish(topic, b"", QoS::AtMostOnce, false).await?;

                let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

                publisher.disconnect().await?;
                subscriber.disconnect().await?;

                match msg {
                    Some(m) => {
                        if !m.payload.is_empty() {
                            return Err(anyhow::anyhow!(
                                "QoS 0: expected empty payload, got {} bytes",
                                m.payload.len()
                            ));
                        }
                    }
                    None => return Err(anyhow::anyhow!("QoS 0: no message received within timeout")),
                }
            }

            // QoS 1
            {
                let publisher = crate::mqtt::v311::MqttV311Client::connect(
                    &ctx.config.broker_addr,
                    "boundary-empty-pub-qos1",
                    ctx.config.connect_timeout,
                )
                .await?;
                let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                    &ctx.config.broker_addr,
                    "boundary-empty-sub-qos1",
                    ctx.config.connect_timeout,
                )
                .await?;

                let topic = "test/boundary/empty/qos1";
                subscriber.subscribe(topic, QoS::AtLeastOnce).await?;

                tokio::time::sleep(Duration::from_millis(100)).await;

                publisher.publish(topic, b"", QoS::AtLeastOnce, false).await?;

                let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

                publisher.disconnect().await?;
                subscriber.disconnect().await?;

                match msg {
                    Some(m) => {
                        if !m.payload.is_empty() {
                            return Err(anyhow::anyhow!(
                                "QoS 1: expected empty payload, got {} bytes",
                                m.payload.len()
                            ));
                        }
                    }
                    None => return Err(anyhow::anyhow!("QoS 1: no message received within timeout")),
                }
            }

            // QoS 2
            {
                let publisher = crate::mqtt::v311::MqttV311Client::connect(
                    &ctx.config.broker_addr,
                    "boundary-empty-pub-qos2",
                    ctx.config.connect_timeout,
                )
                .await?;
                let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                    &ctx.config.broker_addr,
                    "boundary-empty-sub-qos2",
                    ctx.config.connect_timeout,
                )
                .await?;

                let topic = "test/boundary/empty/qos2";
                subscriber.subscribe(topic, QoS::ExactlyOnce).await?;

                tokio::time::sleep(Duration::from_millis(100)).await;

                publisher.publish(topic, b"", QoS::ExactlyOnce, false).await?;

                let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

                publisher.disconnect().await?;
                subscriber.disconnect().await?;

                match msg {
                    Some(m) => {
                        if !m.payload.is_empty() {
                            return Err(anyhow::anyhow!(
                                "QoS 2: expected empty payload, got {} bytes",
                                m.payload.len()
                            ));
                        }
                    }
                    None => return Err(anyhow::anyhow!("QoS 2: no message received within timeout")),
                }
            }

            Ok(())
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

/// Test publishing a ~64KB payload at QoS 1
pub struct LargePayloadTest;

impl TestCase for LargePayloadTest {
    fn name(&self) -> &str {
        "boundary_large_payload"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "boundary-large-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "boundary-large-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "test/boundary/large";
            subscriber.subscribe(topic, QoS::AtLeastOnce).await?;

            tokio::time::sleep(Duration::from_millis(100)).await;

            let payload = vec![0x42u8; 65536];
            publisher.publish(topic, &payload, QoS::AtLeastOnce, false).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(10)).await;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) => {
                    if m.payload.len() != 65536 {
                        return Err(anyhow::anyhow!("expected 65536 bytes, got {} bytes", m.payload.len()));
                    }
                    if m.payload[0] != 0x42 {
                        return Err(anyhow::anyhow!(
                            "first byte mismatch: expected 0x42, got 0x{:02x}",
                            m.payload[0]
                        ));
                    }
                    if m.payload[65535] != 0x42 {
                        return Err(anyhow::anyhow!(
                            "last byte mismatch: expected 0x42, got 0x{:02x}",
                            m.payload[65535]
                        ));
                    }
                    Ok(())
                }
                None => Err(anyhow::anyhow!("no message received within timeout")),
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

/// Test publishing to a topic with special characters (Unicode)
pub struct SpecialCharsTopicTest;

impl TestCase for SpecialCharsTopicTest {
    fn name(&self) -> &str {
        "boundary_special_chars_topic"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "boundary-special-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "boundary-special-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "test/special/你好/世界";
            subscriber.subscribe(topic, QoS::AtLeastOnce).await?;

            tokio::time::sleep(Duration::from_millis(100)).await;

            publisher.publish(topic, b"unicode topic", QoS::AtLeastOnce, false).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) => {
                    if m.payload.as_ref() == b"unicode topic" {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("unexpected payload: {:?}", m.payload))
                    }
                }
                None => Err(anyhow::anyhow!("no message received within timeout")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Test rapid subscribe/unsubscribe cycles and verify no crashes
pub struct RapidSubscribeTest;

impl TestCase for RapidSubscribeTest {
    fn name(&self) -> &str {
        "boundary_rapid_subscribe"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let mut client = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "boundary-rapid-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "test/boundary/rapid/sub";

            for i in 0..10 {
                client.subscribe(topic, QoS::AtLeastOnce).await?;
                client.unsubscribe(topic).await?;
                if !client.is_connected() {
                    return Err(anyhow::anyhow!("client disconnected during iteration {}", i));
                }
            }

            // Final subscribe, publish, and verify delivery
            client.subscribe(topic, QoS::AtLeastOnce).await?;

            tokio::time::sleep(Duration::from_millis(100)).await;

            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "boundary-rapid-pub",
                ctx.config.connect_timeout,
            )
            .await?;

            publisher.publish(topic, b"after rapid sub/unsub", QoS::AtLeastOnce, false).await?;

            let msg = client.recv_message_timeout(Duration::from_secs(5)).await;

            publisher.disconnect().await?;
            client.disconnect().await?;

            match msg {
                Some(m) => {
                    if m.payload.as_ref() == b"after rapid sub/unsub" {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("unexpected payload after rapid cycles: {:?}", m.payload))
                    }
                }
                None => Err(anyhow::anyhow!("no message received after rapid sub/unsub cycles")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}
