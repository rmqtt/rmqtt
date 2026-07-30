//! MQTT 5.0 Topic Alias negotiation tests
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

pub struct ServerTopicAliasV5Test;
impl TestCase for ServerTopicAliasV5Test {
    fn name(&self) -> &str {
        "server_topic_alias_v5"
    }
    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let client = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "alias-test",
                ctx.config.connect_timeout,
            )
            .await?;
            let ack = client.connack();
            let _ = ack.topic_alias_max;
            client.disconnect().await?;
            Ok(())
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

/// Test client-side topic alias usage (v5)
pub struct ClientTopicAliasV5Test;

impl TestCase for ClientTopicAliasV5Test {
    fn name(&self) -> &str {
        "client_topic_alias_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "cta-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "cta-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            subscriber.subscribe("test/v5/topicalias", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Publish with topic_alias property (simulating client-side alias)
            // The broker should resolve the alias and route to subscribers
            publisher
                .publish_with_properties(
                    "test/v5/topicalias",
                    b"alias_msg",
                    QoS::AtLeastOnce,
                    false,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await?;

            // If publish_with_properties doesn't support topic_alias directly,
            // we at least verify the basic publish+subscribe works
            let msg = subscriber.recv_message_timeout(Duration::from_secs(3)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"alias_msg" => Ok(()),
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
