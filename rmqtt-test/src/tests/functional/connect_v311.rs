//! MQTT v3.1.1 Connect/Disconnect functional tests

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Test basic MQTT 3.1.1 connect and disconnect
pub struct ConnectV311Test;

impl TestCase for ConnectV311Test {
    fn name(&self) -> &str {
        "connect_v311"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "connect-v311-test",
                ctx.config.connect_timeout,
            )
            .await?;
            assert!(client.is_connected());
            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }
}

/// Test connect with empty client ID (clean session required)
pub struct ConnectEmptyClientIdTest;

impl TestCase for ConnectEmptyClientIdTest {
    fn name(&self) -> &str {
        "connect_empty_client_id"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "",
                ctx.config.connect_timeout,
            )
            .await?; // should succeed with clean session
            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }
}

/// Test multiple concurrent connections
pub struct MultipleConnectionsTest {
    pub count: usize,
}

impl Default for MultipleConnectionsTest {
    fn default() -> Self {
        Self { count: 10 }
    }
}

impl TestCase for MultipleConnectionsTest {
    fn name(&self) -> &str {
        "multiple_connections"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let count = self.count;
        let addr = ctx.config.broker_addr.clone();
        let connect_timeout = ctx.config.connect_timeout;

        let result = rt.block_on(async {
            let mut clients = Vec::new();
            for i in 0..count {
                let client = crate::mqtt::v311::MqttV311Client::connect(
                    &addr,
                    &format!("multi-conn-{}", i),
                    connect_timeout,
                )
                .await?;
                clients.push(client);
            }
            // Verify all connected
            for client in &clients {
                assert!(client.is_connected());
            }
            // Disconnect all
            for client in clients {
                client.disconnect().await?;
            }
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
