//! MQTT v3.1.1 PubSub functional tests (QoS 0/1/2, retain, unsubscribe)

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Test basic QoS 0 publish/subscribe with v311 client
pub struct PubSubV311Qos0Test;

impl TestCase for PubSubV311Qos0Test {
    fn name(&self) -> &str {
        "pubsub_v311_qos0"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "v311-pub-qos0",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "v311-sub-qos0",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "test/v311/pubsub/qos0";
            subscriber.subscribe(topic, QoS::AtMostOnce).await?;

            tokio::time::sleep(Duration::from_millis(100)).await;

            publisher.publish(topic, b"hello qos0", QoS::AtMostOnce, false).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) => {
                    if m.payload.as_ref() == b"hello qos0" && m.topic == topic {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("unexpected message: topic={}, payload={:?}", m.topic, m.payload))
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

/// Test QoS 1 publish/subscribe with v311 client
pub struct PubSubV311Qos1Test;

impl TestCase for PubSubV311Qos1Test {
    fn name(&self) -> &str {
        "pubsub_v311_qos1"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "v311-pub-qos1",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "v311-sub-qos1",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "test/v311/pubsub/qos1";
            subscriber.subscribe(topic, QoS::AtLeastOnce).await?;

            tokio::time::sleep(Duration::from_millis(100)).await;

            publisher.publish(topic, b"hello qos1", QoS::AtLeastOnce, false).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) => {
                    if m.payload.as_ref() == b"hello qos1" {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("unexpected payload"))
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

/// Test QoS 2 publish/subscribe with v311 client
pub struct PubSubV311Qos2Test;

impl TestCase for PubSubV311Qos2Test {
    fn name(&self) -> &str {
        "pubsub_v311_qos2"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "v311-pub-qos2",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "v311-sub-qos2",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "test/v311/pubsub/qos2";
            subscriber.subscribe(topic, QoS::ExactlyOnce).await?;

            tokio::time::sleep(Duration::from_millis(100)).await;

            publisher.publish(topic, b"hello qos2", QoS::ExactlyOnce, false).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) => {
                    if m.payload.as_ref() == b"hello qos2" {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("unexpected payload"))
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

/// Test retain message with v311 client
pub struct RetainV311Test;

impl TestCase for RetainV311Test {
    fn name(&self) -> &str {
        "retain_v311_message"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            // Publish a retained message
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "v311-retain-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let topic = "test/v311/retain/msg";
            publisher.publish(topic, b"retained payload", QoS::AtLeastOnce, true).await?;
            publisher.disconnect().await?;

            tokio::time::sleep(Duration::from_millis(200)).await;

            // Subscribe and receive the retained message
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "v311-retain-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            subscriber.subscribe(topic, QoS::AtLeastOnce).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            subscriber.disconnect().await?;

            match msg {
                Some(m) => {
                    if m.retain && m.payload.as_ref() == b"retained payload" {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!(
                            "unexpected retained message: retain={}, payload={:?}",
                            m.retain,
                            m.payload
                        ))
                    }
                }
                None => Err(anyhow::anyhow!("no retained message received")),
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

/// Test unsubscribe with v311 client
pub struct UnsubscribeV311Test;

impl TestCase for UnsubscribeV311Test {
    fn name(&self) -> &str {
        "unsubscribe_v311"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "v311-unsub-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "v311-unsub-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "test/v311/unsub/topic";
            subscriber.subscribe(topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Should receive
            publisher.publish(topic, b"before unsub", QoS::AtLeastOnce, false).await?;
            let msg = subscriber.recv_message_timeout(Duration::from_secs(3)).await;
            if msg.is_none() {
                return Err(anyhow::anyhow!("should have received message before unsubscribe"));
            }

            // Unsubscribe
            subscriber.unsubscribe(topic).await?;
            tokio::time::sleep(Duration::from_millis(200)).await;

            // Should NOT receive
            publisher.publish(topic, b"after unsub", QoS::AtLeastOnce, false).await?;
            let msg = subscriber.recv_message_timeout(Duration::from_secs(2)).await;
            if msg.is_some() {
                return Err(anyhow::anyhow!("should NOT have received message after unsubscribe"));
            }

            publisher.disconnect().await?;
            subscriber.disconnect().await?;
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
