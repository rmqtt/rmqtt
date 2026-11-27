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

// ---------------------------------------------------------------------------
// P1 gap-analysis additions (designs/mqtt-5.0-standalone-test-gap-analysis.md)
// ---------------------------------------------------------------------------

/// G18: QoS 1 messages published to the same topic must be delivered to the
/// subscriber in publish order. [MQTT-4.6.0-1 / MQTT-4.6.0-2]
pub struct PubSubV5Qos1OrderingTest;

impl TestCase for PubSubV5Qos1OrderingTest {
    fn name(&self) -> &str {
        "pubsub_v5_qos1_ordering"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let topic = "test/v5/order/qos1";
            let count = 20usize;

            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "order-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            subscriber.subscribe(topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let mut publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "order-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            // Publish sequentially, waiting for each PUBACK to keep the
            // in-flight window at 1 (strongest ordering guarantee).
            for i in 0..count {
                let payload = format!("m-{i:02}");
                publisher.publish(topic, payload.as_bytes(), QoS::AtLeastOnce, false).await?;
                // Wait for the PUBACK of this publish before the next one.
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    if Instant::now() > deadline {
                        return Err(anyhow::anyhow!("no PUBACK for message {i} within timeout"));
                    }
                    if publisher.recv_puback_reason(Duration::from_millis(500)).await.is_some() {
                        break;
                    }
                }
            }

            // Collect all messages and verify order.
            let mut payloads = Vec::with_capacity(count);
            for _ in 0..count {
                match subscriber.recv_message_timeout(Duration::from_secs(5)).await {
                    Some(m) => payloads.push(
                        String::from_utf8(m.payload.to_vec())
                            .map_err(|_| anyhow::anyhow!("non-utf8 payload"))?,
                    ),
                    None => {
                        return Err(anyhow::anyhow!(
                            "only {}/{} messages delivered in time",
                            payloads.len(),
                            count
                        ))
                    }
                }
            }
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            let expected: Vec<String> = (0..count).map(|i| format!("m-{i:02}")).collect();
            if payloads == expected {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "QoS 1 delivery order violated [MQTT-4.6.0-1]: \
                     expected {expected:?}, got {payloads:?}"
                ))
            }
        });
        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(60)
    }
}

/// G22 (record-type): the Response Topic is a client-side constraint
/// (MUST NOT contain wildcards when the client sends it); what the broker
/// does with a wildcard Response Topic is a MAY-level choice. This test
/// records the broker's actual behavior without asserting an outcome.
pub struct PublishV5ResponseTopicWildcardTest;

impl TestCase for PublishV5ResponseTopicWildcardTest {
    fn name(&self) -> &str {
        "publish_v5_response_topic_wildcard"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let observation: String = rt.block_on(async {
            let mut publisher = match crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "resptopic-obs",
                ctx.config.connect_timeout,
            )
            .await
            {
                Ok(c) => c,
                Err(e) => return format!("connect failed: {e}"),
            };

            let result = publisher
                .publish_with_properties(
                    "test/v5/resptopic",
                    b"wildcard-response-topic",
                    QoS::AtLeastOnce,
                    false,
                    None,
                    None,
                    Some("resp/+/1"), // wildcards in Response Topic
                    None,
                    None,
                    None,
                )
                .await;

            if let Err(e) = result {
                publisher.abort_connection().await.ok();
                return format!("send failed: {e}");
            }

            match publisher.recv_puback_reason(Duration::from_secs(3)).await {
                Some((_pid, code)) => {
                    format!("broker accepted the PUBLISH (PUBACK reason 0x{code:02X})")
                }
                None => {
                    if !publisher.is_connected() {
                        "broker closed the connection".to_string()
                    } else {
                        "no PUBACK within timeout, connection still open".to_string()
                    }
                }
            }
        });

        // Record-type: whatever happened, report the observation as Info.
        TestResult::passed_with_note(self.name(), "functional_v5", start.elapsed(), &observation)
    }

    fn expectation(&self) -> crate::framework::testcase::Expectation {
        crate::framework::testcase::Expectation::Info
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}
