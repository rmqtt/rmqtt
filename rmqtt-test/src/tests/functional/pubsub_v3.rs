//! MQTT v3 PubSub functional tests (QoS 0 only)

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Test basic QoS 0 publish/subscribe with v3 client
pub struct PubSubV3Qos0Test;

impl TestCase for PubSubV3Qos0Test {
    fn name(&self) -> &str {
        "pubsub_v3_qos0"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                "v3-pub-qos0",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                "v3-sub-qos0",
                ctx.config.connect_timeout,
            )
            .await?;

            tracing::info!("publisher connect ok");
            tracing::info!("subscriber connect ok");

            let topic = "test/v3/pubsub/qos0";
            subscriber.subscribe(topic).await?;
            tracing::info!("subscriber subscribe ok");

            // Allow subscription to propagate
            tokio::time::sleep(Duration::from_millis(1000)).await;

            publisher.publish(topic, b"hello v3 qos0").await?;
            tracing::info!("publisher publish ok");
            let now = Instant::now();
            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            tracing::info!("recv_message_timeout: {:?}, cost time: {:?}", msg, now.elapsed());
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) => {
                    if m.payload.as_ref() == b"hello v3 qos0" && m.topic == topic {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("unexpected message: topic={}, payload={:?}", m.topic, m.payload))
                    }
                }
                None => Err(anyhow::anyhow!("no message received within timeout")),
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
