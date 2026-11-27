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
        // Skip (as passed) when retained messages are unavailable.
        if let Some(result) = ctx.guard_retain_required(self.name(), "functional_v311", start) {
            return result;
        }
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

            let verdict = match msg {
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
            };

            // Cleanup: delete the retained message so it doesn't leak into
            // other tests (e.g. `#` subscriptions in wildcard tests).
            if let Ok(client) = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "v311-retain-msg-cleanup",
                ctx.config.connect_timeout,
            )
            .await
            {
                let _ = client.publish(topic, b"", QoS::AtLeastOnce, true).await;
                let _ = client.disconnect().await;
            }

            verdict
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

/// QoS negotiation: the QoS delivered to a subscriber is the minimum of the
/// publish QoS and the subscription QoS. [MQTT-4.3.3-1] [MQTT-4.5.0-1]
pub struct QosDowngradeV311Test;

impl TestCase for QosDowngradeV311Test {
    fn name(&self) -> &str {
        "qos_downgrade_v311"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();

            // (publish QoS, subscription QoS, expected delivered QoS)
            let scenarios: [(QoS, QoS, QoS, &str); 3] = [
                (QoS::AtLeastOnce, QoS::AtMostOnce, QoS::AtMostOnce, "qos1-sub0"),
                (QoS::ExactlyOnce, QoS::AtLeastOnce, QoS::AtLeastOnce, "qos2-sub1"),
                (QoS::ExactlyOnce, QoS::AtMostOnce, QoS::AtMostOnce, "qos2-sub0"),
            ];

            for (pub_qos, sub_qos, expected_qos, label) in scenarios {
                let topic = format!("test/downgrade/{label}/{uid}");
                let pub_cid = format!("downgrade-pub-{label}-{uid}");
                let sub_cid = format!("downgrade-sub-{label}-{uid}");

                let publisher = crate::mqtt::v311::MqttV311Client::connect(
                    &ctx.config.broker_addr,
                    &pub_cid,
                    ctx.config.connect_timeout,
                )
                .await?;
                let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                    &ctx.config.broker_addr,
                    &sub_cid,
                    ctx.config.connect_timeout,
                )
                .await?;

                subscriber.subscribe(&topic, sub_qos).await?;
                tokio::time::sleep(Duration::from_millis(100)).await;

                if pub_qos == QoS::ExactlyOnce {
                    // Complete the QoS 2 publisher-side handshake so the broker
                    // does not hold a dangling in-flight state.
                    let pid = std::num::NonZeroU16::new(1).expect("1 is non-zero");
                    publisher
                        .publish_with_packet_id(&topic, b"downgrade", pub_qos, false, false, pid)
                        .await?;
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    let _ = publisher.send_pubrel(pid).await;
                } else {
                    publisher.publish(&topic, b"downgrade", pub_qos, false).await?;
                }

                let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;

                let _ = publisher.disconnect().await;
                let _ = subscriber.disconnect().await;

                match msg {
                    Some(m) if m.qos == expected_qos => {}
                    Some(m) => {
                        return Err(anyhow::anyhow!(
                            "{label}: expected delivered QoS {:?}, got {:?}",
                            expected_qos,
                            m.qos
                        ))
                    }
                    None => return Err(anyhow::anyhow!("{label}: no message received")),
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
        Duration::from_secs(30)
    }
}
