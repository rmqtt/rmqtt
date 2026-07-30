//! MQTT 5.0 Disconnect Reason Code tests

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Test V5 disconnect with various reason codes
pub struct DisconnectReasonV5Test;

impl TestCase for DisconnectReasonV5Test {
    fn name(&self) -> &str {
        "disconnect_reason_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            // Connect and disconnect with NormalDisconnection (0)
            let client = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "disconnect-reason1",
                ctx.config.connect_timeout,
            )
            .await?;
            client.disconnect_with_reason(Some(0)).await?;

            tokio::time::sleep(Duration::from_millis(200)).await;

            // Connect again and disconnect with DisconnectWithWillMessage (4)
            let client2 = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "disconnect-reason2",
                ctx.config.connect_timeout,
            )
            .await?;
            client2.disconnect_with_reason(Some(4)).await?;

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
