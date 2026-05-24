//! MQTT 5.0 Flow Control (receive_max) tests

use std::num::NonZeroU16;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Test V5 flow control with receive_max=2 - rapid publishes should not overflow
pub struct FlowControlV5Test;

impl TestCase for FlowControlV5Test {
    fn name(&self) -> &str {
        "flow_control_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            // Connect with receive_max=2
            let publisher = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "fc-pub-v5",
                ctx.config.connect_timeout,
                true,
                60,
                None,
                None,
                None,
                None,
                NonZeroU16::new(2),
            )
            .await?;
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "fc-sub-v5",
                ctx.config.connect_timeout,
                true,
                60,
                None,
                None,
                None,
                None,
                NonZeroU16::new(2),
            )
            .await?;

            subscriber.subscribe("test/flow/control", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Publish rapidly - should not overflow despite receive_max=2
            for i in 0..10 {
                publisher
                    .publish("test/flow/control", format!("msg{}", i).as_bytes(), QoS::AtLeastOnce, false)
                    .await?;
            }

            // Verify at least some messages were delivered
            let mut received = 0;
            for _ in 0..10 {
                if subscriber.recv_message_timeout(Duration::from_secs(3)).await.is_some() {
                    received += 1;
                }
            }

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            if received >= 1 {
                Ok(())
            } else {
                Err(anyhow::anyhow!("no messages received with flow control"))
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}
