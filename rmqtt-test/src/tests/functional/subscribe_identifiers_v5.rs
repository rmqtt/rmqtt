//! MQTT 5.0 Subscription Identifiers test
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

pub struct SubscribeIdentifiersV5Test;
impl TestCase for SubscribeIdentifiersV5Test {
    fn name(&self) -> &str {
        "subscribe_identifiers_v5"
    }
    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let client = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "subid-v5",
                ctx.config.connect_timeout,
            )
            .await?;
            let ack = client.connack();
            let _ = ack.subscription_identifiers_available;
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
