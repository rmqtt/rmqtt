//! KeepAlive and PINGREQ/PINGRESP functional tests

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Test that sending PINGREQs keeps the connection alive with short keep_alive (v3.1.1)
pub struct KeepAliveV311Test;

impl TestCase for KeepAliveV311Test {
    fn name(&self) -> &str {
        "keepalive_v311_ping_keeps_alive"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "keepalive-v311",
                ctx.config.connect_timeout,
                true,
                5, // 5 second keep alive
                None,
                None,
                None,
            )
            .await?;

            // Send PINGREQs at 2s intervals for 15s (3x the keepalive)
            for _ in 0..6 {
                client.ping().await?;
                tokio::time::sleep(Duration::from_secs(2)).await;
            }

            // Should still be connected
            if !client.is_connected() {
                return Err(anyhow::anyhow!("client disconnected despite sending PINGREQs"));
            }

            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// Test that without PINGREQs, broker disconnects after keep_alive expires (v3.1.1)
pub struct KeepAliveTimeoutTest;

impl TestCase for KeepAliveTimeoutTest {
    fn name(&self) -> &str {
        "keepalive_v311_timeout"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "keepalive-timeout",
                ctx.config.connect_timeout,
                true,
                5, // 5 second keep alive
                None,
                None,
                None,
            )
            .await?;

            // Don't send any PINGREQs - wait for timeout
            // MQTT spec says broker should wait keepalive * 1.5 before disconnecting
            tokio::time::sleep(Duration::from_secs(15)).await;

            // After 15s with keepalive=5, should be disconnected
            if client.is_connected() {
                return Err(anyhow::anyhow!("client should have been disconnected by keepalive timeout"));
            }

            // Attempting to disconnect should not panic
            let _ = client.disconnect().await;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// Test PINGREQ/PINGRESP with MQTT 5.0 client
pub struct PingV5Test;

impl TestCase for PingV5Test {
    fn name(&self) -> &str {
        "ping_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "ping-v5-test",
                ctx.config.connect_timeout,
                true,
                10,
                None,
                None,
                None,
                None,
            )
            .await?;

            // Send multiple PINGREQs
            for _ in 0..5 {
                client.ping().await?;
                tokio::time::sleep(Duration::from_millis(200)).await;
            }

            assert!(client.is_connected());
            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
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
