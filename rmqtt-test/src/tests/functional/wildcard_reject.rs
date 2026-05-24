//! Wildcard publish rejection tests

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Test that publishing to wildcard topics is handled (v3.1.1)
/// Per MQTT spec, clients MUST NOT publish to topics containing wildcards (# or +).
/// The broker may disconnect the client or silently reject such publishes.
pub struct PublishWildcardRejectTest;

impl TestCase for PublishWildcardRejectTest {
    fn name(&self) -> &str {
        "publish_wildcard_reject"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "wildcard-reject-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            // Subscribe to something to verify connectivity
            subscriber.subscribe("test/wildcard/monitor", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "wildcard-reject-pub",
                ctx.config.connect_timeout,
            )
            .await?;

            // Try publishing to wildcard topics with QoS 1
            publisher.publish("test/#", b"hash_wildcard", QoS::AtLeastOnce, false).await?;
            publisher.publish("test/+", b"plus_wildcard", QoS::AtLeastOnce, false).await?;

            tokio::time::sleep(Duration::from_millis(200)).await;

            // Check if still connected after wildcard publishes
            // If the broker disconnects us, is_connected() will return false
            if publisher.is_connected() {
                // Broker didn't disconnect us - acceptable behavior
                publisher.disconnect().await?;
            }

            // Verify subscriber didn't receive the wildcard messages
            let msg = subscriber.recv_message_timeout(Duration::from_secs(2)).await;
            subscriber.disconnect().await?;

            if msg.is_some() {
                return Err(anyhow::anyhow!(
                    "subscriber unexpectedly received a message from wildcard publish"
                ));
            }

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
