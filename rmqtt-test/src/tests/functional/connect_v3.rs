//! MQTT v3 Connect/Disconnect functional tests

use std::time::Instant;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Test basic MQTT 3.0 connect and disconnect
pub struct ConnectV3Test;

impl TestCase for ConnectV3Test {
    fn name(&self) -> &str { "connect_v3" }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                "connect-v3-test",
                ctx.config.connect_timeout,
            ).await?;
            assert!(client.is_connected());
            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }
}
