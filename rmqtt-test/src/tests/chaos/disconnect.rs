//! Client disconnect chaos tests (using v311 client)

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Test rapid connect/disconnect cycles
pub struct ConnectionChurnTest {
    pub cycles: usize,
}

impl Default for ConnectionChurnTest {
    fn default() -> Self {
        Self { cycles: 20 }
    }
}

impl TestCase for ConnectionChurnTest {
    fn name(&self) -> &str {
        "chaos_connection_churn"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cycles = self.cycles;

        let result = rt.block_on(async {
            for i in 0..cycles {
                let client = crate::mqtt::v311::MqttV311Client::connect(
                    &ctx.config.broker_addr,
                    &format!("churn-{}", i),
                    ctx.config.connect_timeout,
                )
                .await?;
                // Immediately disconnect
                client.disconnect().await?;
            }
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "chaos", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "chaos", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(60)
    }
}

/// Test reconnect storm - many clients connecting simultaneously
pub struct ReconnectStormTest {
    pub client_count: usize,
}

impl Default for ReconnectStormTest {
    fn default() -> Self {
        Self { client_count: 50 }
    }
}

impl TestCase for ReconnectStormTest {
    fn name(&self) -> &str {
        "chaos_reconnect_storm"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let count = self.client_count;

        let result = rt.block_on(async {
            let mut handles = Vec::new();
            let success = Arc::new(std::sync::atomic::AtomicU64::new(0));

            for i in 0..count {
                let addr = ctx.config.broker_addr.clone();
                let s = success.clone();
                let handle = tokio::spawn(async move {
                    if let Ok(client) = crate::mqtt::v311::MqttV311Client::connect(
                        &addr,
                        &format!("storm-{}", i),
                        Duration::from_secs(15),
                    )
                    .await
                    {
                        s.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        let _ = client.disconnect().await;
                    }
                });
                handles.push(handle);
            }

            for handle in handles {
                let _ = handle.await;
            }

            let successes = success.load(std::sync::atomic::Ordering::Relaxed);
            if successes as usize >= count * 80 / 100 {
                Ok(())
            } else {
                Err(anyhow::anyhow!("only {}/{} connections succeeded", successes, count))
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "chaos", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "chaos", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(60)
    }
}
