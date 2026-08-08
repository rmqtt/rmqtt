//! MQTT v3.1 Retained Message functional tests
//!
//! Covers spec section 3.3.1 (PUBLISH RETAIN flag):
//! - a retained message is stored and delivered to new subscribers with RETAIN=1
//! - a live (non-retained) message is delivered with RETAIN=0
//! - an empty-payload retained publish deletes the retained message
//! - a retained publish overwrites the previous retained message
//! - will retain

use std::time::{Duration, Instant};

use bytestring::ByteString;
use rmqtt_codec::v3::{LastWill, QoS};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Positive: a retained message is stored and delivered to a NEW subscriber
/// with the RETAIN flag set to 1.
pub struct RetainV3Test;

impl TestCase for RetainV3Test {
    fn name(&self) -> &str {
        "retain_v3_store_and_deliver"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        if let Some(r) = ctx.guard_retain_required(self.name(), "functional_v3", start) {
            return r;
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let publisher = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                "v3-retain-pub",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "test/v3/retain/store";
            publisher.publish(topic, b"retained-data", QoS::AtMostOnce, true).await?;
            tokio::time::sleep(Duration::from_millis(200)).await;
            publisher.disconnect().await?;

            // New subscriber (after the retain was stored)
            let mut subscriber = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                "v3-retain-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            subscriber.subscribe(topic, QoS::AtMostOnce).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            subscriber.disconnect().await?;

            let verdict = match msg {
                Some(m)
                    if m.payload.as_ref() == b"retained-data"
                        && m.topic == topic
                        && m.retain
                        && m.qos == QoS::AtMostOnce =>
                {
                    Ok(())
                }
                Some(m) => Err(anyhow::anyhow!(
                    "unexpected retained message: topic={}, payload={:?}, retain={}",
                    m.topic,
                    m.payload,
                    m.retain
                )),
                None => Err(anyhow::anyhow!("retained message was not delivered to new subscriber")),
            };

            // Cleanup: delete the retained message so it doesn't leak into
            // other tests (e.g. `#` subscriptions in wildcard tests).
            if let Ok(client) = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                "v3-retain-cleanup",
                ctx.config.connect_timeout,
            )
            .await
            {
                let _ = client.publish(topic, b"", QoS::AtMostOnce, true).await;
                let _ = client.disconnect().await;
            }

            verdict
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

/// Positive: a live (non-retained) message is delivered to an existing
/// subscriber with RETAIN=0, even if a retained message exists on the topic.
pub struct RetainV3LiveNotRetainedTest;

impl TestCase for RetainV3LiveNotRetainedTest {
    fn name(&self) -> &str {
        "retain_v3_live_message_not_retained"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        if let Some(r) = ctx.guard_retain_required(self.name(), "functional_v3", start) {
            return r;
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let publisher = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                "v3-live-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                "v3-live-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "test/v3/retain/live";
            subscriber.subscribe(topic, QoS::AtMostOnce).await?;
            tokio::time::sleep(Duration::from_millis(1000)).await;

            // Live message, RETAIN flag = 0
            publisher.publish(topic, b"live-data", QoS::AtMostOnce, false).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"live-data" && !m.retain => Ok(()),
                Some(m) => Err(anyhow::anyhow!("live message must have RETAIN=0, got retain={}", m.retain)),
                None => Err(anyhow::anyhow!("live message was not received")),
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

/// Boundary: publishing an empty-payload message with RETAIN=1 deletes the
/// retained message on that topic.
pub struct RetainV3EmptyDeleteTest;

impl TestCase for RetainV3EmptyDeleteTest {
    fn name(&self) -> &str {
        "retain_v3_empty_payload_deletes"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        if let Some(r) = ctx.guard_retain_required(self.name(), "functional_v3", start) {
            return r;
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let publisher = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                "v3-del-pub",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "test/v3/retain/delete";
            // Store a retained message
            publisher.publish(topic, b"to-be-deleted", QoS::AtMostOnce, true).await?;
            tokio::time::sleep(Duration::from_millis(200)).await;

            // Delete it with an empty-payload retained publish
            publisher.publish(topic, b"", QoS::AtMostOnce, true).await?;
            tokio::time::sleep(Duration::from_millis(200)).await;
            publisher.disconnect().await?;

            // A new subscriber must NOT receive the (deleted) retained message
            let mut subscriber = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                "v3-del-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            subscriber.subscribe(topic, QoS::AtMostOnce).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(2)).await;
            subscriber.disconnect().await?;

            if msg.is_some() {
                Err(anyhow::anyhow!("retained message was not deleted by empty-payload publish"))
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
        Duration::from_secs(15)
    }
}

/// Boundary: a second retained publish on the same topic overwrites the
/// previous retained message; new subscribers get the newest one.
pub struct RetainV3OverwriteTest;

impl TestCase for RetainV3OverwriteTest {
    fn name(&self) -> &str {
        "retain_v3_overwrite"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        if let Some(r) = ctx.guard_retain_required(self.name(), "functional_v3", start) {
            return r;
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let publisher = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                "v3-ovw-pub",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "test/v3/retain/overwrite";
            publisher.publish(topic, b"first-version", QoS::AtMostOnce, true).await?;
            tokio::time::sleep(Duration::from_millis(200)).await;
            publisher.publish(topic, b"second-version", QoS::AtMostOnce, true).await?;
            tokio::time::sleep(Duration::from_millis(200)).await;
            publisher.disconnect().await?;

            let mut subscriber = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                "v3-ovw-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            subscriber.subscribe(topic, QoS::AtMostOnce).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            subscriber.disconnect().await?;

            let verdict = match msg {
                Some(m) if m.payload.as_ref() == b"second-version" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("expected overwritten retained payload, got {:?}", m.payload)),
                None => Err(anyhow::anyhow!("no retained message received after overwrite")),
            };

            // Cleanup: delete the retained message so it doesn't leak into
            // other tests (e.g. `#` subscriptions in wildcard tests).
            if let Ok(client) = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                "v3-ovw-cleanup",
                ctx.config.connect_timeout,
            )
            .await
            {
                let _ = client.publish(topic, b"", QoS::AtMostOnce, true).await;
                let _ = client.disconnect().await;
            }

            verdict
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

/// Positive: a retained Will message is stored by the broker when the
/// connection dies, and delivered to a new subscriber with RETAIN=1.
pub struct RetainV3WillTest;

impl TestCase for RetainV3WillTest {
    fn name(&self) -> &str {
        "retain_v3_will"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        if let Some(r) = ctx.guard_retain_required(self.name(), "functional_v3", start) {
            return r;
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let will_topic = "test/v3/retain/will";
            let will = LastWill {
                qos: QoS::AtLeastOnce,
                retain: true,
                topic: ByteString::from(will_topic),
                message: bytes::Bytes::from_static(b"retained-will"),
            };
            let client = crate::mqtt::v3::MqttV3Client::connect_with_options(
                &ctx.config.broker_addr,
                "v3-retain-will-client",
                ctx.config.connect_timeout,
                true,
                60,
                Some(will),
                None,
                None,
            )
            .await?;

            // Kill the connection so the will fires
            client.abort_connection().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // A new subscriber on the will topic must get the retained will
            let mut subscriber = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                "v3-retain-will-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            subscriber.subscribe(will_topic, QoS::AtLeastOnce).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            subscriber.disconnect().await?;

            let verdict = match msg {
                Some(m) if m.payload.as_ref() == b"retained-will" && m.retain => Ok(()),
                Some(m) => Err(anyhow::anyhow!(
                    "unexpected retained will: payload={:?}, retain={}",
                    m.payload,
                    m.retain
                )),
                None => Err(anyhow::anyhow!("retained will was not delivered")),
            };

            // Cleanup: delete the retained will so it doesn't leak into
            // other tests (e.g. `#` subscriptions in wildcard tests).
            if let Ok(c) = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                "v3-retain-will-cleanup",
                ctx.config.connect_timeout,
            )
            .await
            {
                let _ = c.publish(will_topic, b"", QoS::AtMostOnce, true).await;
                let _ = c.disconnect().await;
            }

            verdict
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
