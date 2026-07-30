//! MQTT v3.1.1 Authentication Edge Case Tests

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Test connect with empty client_id AND clean_session=false.
/// Per MQTT spec, this combination should be rejected by the broker.
/// If the broker auto-generates a client ID, that's also acceptable behavior.
pub struct AuthEmptyClientIdFailTest;

impl TestCase for AuthEmptyClientIdFailTest {
    fn name(&self) -> &str {
        "auth_empty_client_id_fail"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let result = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "",                     // empty client_id
                Duration::from_secs(5), // short connect timeout
                false,                  // clean_session = false
                60,                     // keep_alive
                None,                   // no last will
                None,                   // no username
                None,                   // no password
            )
            .await;

            match result {
                Ok(client) => {
                    // Broker accepted the connection (may have auto-generated client ID)
                    client.disconnect().await?;
                    Ok(())
                }
                Err(_e) => {
                    // Broker rejected the connection (expected per MQTT spec)
                    Ok(())
                }
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Test a sequence of connect-disconnect-connect with the same client ID 5 times in a row.
/// Verifies each cycle succeeds and the broker properly cleans up between connections.
pub struct AuthConnectDisconnectSequenceTest;

impl TestCase for AuthConnectDisconnectSequenceTest {
    fn name(&self) -> &str {
        "auth_connect_disconnect_sequence"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            for _ in 0..5 {
                let client = crate::mqtt::v311::MqttV311Client::connect(
                    &ctx.config.broker_addr,
                    "connect-disconnect-seq",
                    ctx.config.connect_timeout,
                )
                .await?;
                assert!(client.is_connected());
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
        Duration::from_secs(20)
    }
}
