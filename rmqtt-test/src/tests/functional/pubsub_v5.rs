//! MQTT v5 PubSub functional tests (QoS 0/1/2)

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Test basic QoS 0 publish/subscribe with v5 client
pub struct PubSubV5Qos0Test;

impl TestCase for PubSubV5Qos0Test {
    fn name(&self) -> &str {
        "pubsub_v5_qos0"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "v5-pub-qos0",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "v5-sub-qos0",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "test/v5/pubsub/qos0";
            subscriber.subscribe(topic, QoS::AtMostOnce).await?;

            tokio::time::sleep(Duration::from_millis(100)).await;

            publisher.publish(topic, b"hello v5 qos0", QoS::AtMostOnce, false).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"hello v5 qos0" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected payload: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("no message received")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Test QoS 1 publish/subscribe with v5 client
pub struct PubSubV5Qos1Test;

impl TestCase for PubSubV5Qos1Test {
    fn name(&self) -> &str {
        "pubsub_v5_qos1"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "v5-pub-qos1",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "v5-sub-qos1",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "test/v5/pubsub/qos1";
            subscriber.subscribe(topic, QoS::AtLeastOnce).await?;

            tokio::time::sleep(Duration::from_millis(100)).await;

            publisher.publish(topic, b"hello v5 qos1", QoS::AtLeastOnce, false).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"hello v5 qos1" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected payload: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("no message received")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Test QoS 2 publish/subscribe with v5 client
pub struct PubSubV5Qos2Test;

impl TestCase for PubSubV5Qos2Test {
    fn name(&self) -> &str {
        "pubsub_v5_qos2"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "v5-pub-qos2",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "v5-sub-qos2",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = "test/v5/pubsub/qos2";
            subscriber.subscribe(topic, QoS::ExactlyOnce).await?;

            tokio::time::sleep(Duration::from_millis(100)).await;

            publisher.publish(topic, b"hello v5 qos2", QoS::ExactlyOnce, false).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"hello v5 qos2" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected payload: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("no message received")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}
