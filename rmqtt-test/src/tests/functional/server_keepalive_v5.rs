//! MQTT 5.0 Server Keep Alive test
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

pub struct ServerKeepAliveV5Test;
impl TestCase for ServerKeepAliveV5Test {
    fn name(&self) -> &str {
        "server_keepalive_v5"
    }
    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let client = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "sk-v5",
                ctx.config.connect_timeout,
                true,
                10,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await?;
            let ack = client.connack();
            if let Some(sk) = ack.server_keepalive_sec {
                assert!(sk > 0, "server keepalive should be positive");
            }
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
