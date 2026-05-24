//! MQTT v5 Connect functional tests

use std::time::Instant;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use rmqtt_codec::v5::ConnectAckReason;

/// Test basic MQTT 5.0 connect with session expiry
pub struct ConnectV5Test;

impl TestCase for ConnectV5Test {
    fn name(&self) -> &str {
        "connect_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "connect-v5-test",
                ctx.config.connect_timeout,
                true,
                60,
                None,
                None,
                None,
                Some(3600), // session_expiry_interval
                None,
            )
            .await?;
            assert!(client.is_connected());

            // Verify connack has v5 fields
            let ack = client.connack();
            if ack.reason_code != ConnectAckReason::Success {
                return Err(anyhow::anyhow!("CONNACK failed: {:?}", ack.reason_code));
            }

            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }
}

/// Test MQTT 5.0 reason code validation
pub struct ConnectV5ReasonCodeTest;

impl TestCase for ConnectV5ReasonCodeTest {
    fn name(&self) -> &str {
        "connect_v5_reason_codes"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "v5-reason-code-test",
                ctx.config.connect_timeout,
            )
            .await?;

            // CONNACK should have reason code 0 (Success)
            let ack = client.connack();
            if ack.reason_code != ConnectAckReason::Success {
                return Err(anyhow::anyhow!("CONNACK reason code not success: {:?}", ack.reason_code));
            }

            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }
}
