//! MQTT 5.0 no_local subscription option tests

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use rmqtt_codec::v5::RetainHandling;

/// Test that no_local prevents receiving own published messages (v5)
pub struct NoLocalV5Test;

impl TestCase for NoLocalV5Test {
    fn name(&self) -> &str {
        "no_local_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let topic = "test/v5/nolocal/messages";

            // Connect as both publisher and subscriber
            let mut client = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "nolocal-client",
                ctx.config.connect_timeout,
            )
            .await?;

            // Subscribe with no_local=true
            client
                .subscribe_with_options(topic, QoS::AtLeastOnce, true, false, RetainHandling::AtSubscribe)
                .await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Publish a message
            client.publish(topic, b"hello_nolocal", QoS::AtLeastOnce, false).await?;

            // Client should NOT receive its own message
            let msg = client.recv_message_timeout(Duration::from_secs(2)).await;
            client.disconnect().await?;

            if msg.is_some() {
                return Err(anyhow::anyhow!("received own message despite no_local=true"));
            }

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
