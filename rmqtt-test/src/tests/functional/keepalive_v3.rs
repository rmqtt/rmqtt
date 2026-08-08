//! MQTT v3.1 Keep Alive / PING functional tests
//!
//! Covers spec section 3.1 (Keep Alive timer) and 3.12-3.13 (PINGREQ/PINGRESP):
//! - PINGREQ is answered with PINGRESP
//! - keep_alive = 0 disables the timeout (no disconnect)
//! - a silent client is disconnected after the keep alive timeout
//!
//! NOTE: RMQTT applies the negotiated keep-alive adjustment from
//! `DefaultFitter::keep_alive()` (`fitter.rs`): the effective timeout window
//! is `keep_alive * keepalive_backoff * 2.0` (default backoff 0.75 → 1.5x),
//! and values < 6s are bumped to `keep_alive + 3`. The disconnect wait below
//! therefore uses a generous margin.

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Positive: PINGREQ is answered with PINGRESP and the connection stays open.
pub struct KeepAliveV3PingTest;

impl TestCase for KeepAliveV3PingTest {
    fn name(&self) -> &str {
        "keepalive_v3_ping"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                "v3-keepalive-ping",
                ctx.config.connect_timeout,
            )
            .await?;
            assert!(client.is_connected());

            // Send several PINGREQs; the reader loop consumes PINGRESP.
            for _ in 0..3 {
                client.ping().await?;
                tokio::time::sleep(Duration::from_millis(200)).await;
                assert!(client.is_connected(), "connection must stay alive after PINGREQ/PINGRESP exchange");
            }

            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Boundary: keep_alive = 0 means the server must not disconnect the client,
/// no matter how long it stays silent.
pub struct KeepAliveV3ZeroTest;

impl TestCase for KeepAliveV3ZeroTest {
    fn name(&self) -> &str {
        "keepalive_v3_zero"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v3::MqttV3Client::connect_with_options(
                &ctx.config.broker_addr,
                "v3-keepalive-zero",
                ctx.config.connect_timeout,
                true,
                0, // keep_alive = 0 → no timeout
                None,
                None,
                None,
            )
            .await?;
            assert!(client.is_connected());

            // Stay silent well beyond a typical keep-alive window.
            tokio::time::sleep(Duration::from_secs(5)).await;
            assert!(
                client.is_connected(),
                "keep_alive = 0 must disable the timeout, but the broker disconnected us"
            );

            // The connection must still be usable.
            client.ping().await?;
            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// Negative: a client that stays silent beyond the (adjusted) keep alive
/// window is disconnected by the broker.
pub struct KeepAliveV3TimeoutTest;

impl TestCase for KeepAliveV3TimeoutTest {
    fn name(&self) -> &str {
        "keepalive_v3_timeout"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            // keep_alive = 2s → effective window = 2 + 3 = 5s (value < 6 bumped
            // by +3 in DefaultFitter::keep_alive). Wait 8s to be safe.
            let client = crate::mqtt::v3::MqttV3Client::connect_with_options(
                &ctx.config.broker_addr,
                "v3-keepalive-timeout",
                ctx.config.connect_timeout,
                true,
                2,
                None,
                None,
                None,
            )
            .await?;
            assert!(client.is_connected());

            // Stay silent — the broker must drop us after the keep alive window.
            tokio::time::sleep(Duration::from_secs(8)).await;

            let connected = client.is_connected();
            let _ = client.disconnect().await;

            if connected {
                Err(anyhow::anyhow!("broker did not disconnect a keep-alive timeout client"))
            } else {
                Ok(())
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}
