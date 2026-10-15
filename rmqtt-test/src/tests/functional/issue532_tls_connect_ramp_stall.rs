//! Issue #532 — the connect ramp on a **TLS** listener (single node).
//!
//! Companion of [`super::issue532_connect_ramp_stall`]: that module documents
//! the incident, the detector and why the fixture is shaped the way it is. This
//! case runs the same experiment with the ramp arriving over TLS — the
//! transport the reporter's deployment used — and shares that module's ramp,
//! watchdog, rate profile and verdict, so a difference between the two runs is
//! attributable to the transport alone.
//!
//! ## Why the transport is not a detail
//!
//! `rmqtt/src/server.rs::listen_tls` performs the TLS handshake inside a
//! per-connection `tokio::spawn`, i.e. **outside** the per-port
//! `TaskExecQueue` that `max_handshaking_limit` sizes; only the MQTT handshake
//! that follows goes through that queue. TLS therefore adds CPU work (rustls +
//! certificate verification) at unbounded concurrency immediately beside the
//! queue's dispatcher task — the one variable this case isolates. The plain-TCP
//! case is its control.
//!
//! ## Fixture
//!
//! `configs/issue532-ramp-tls/` — MQTT/TLS `127.0.0.1:1908`, gRPC 5385, HTTP
//! 6072, and otherwise the same shape as `configs/issue532-ramp/`: a two-worker
//! handshake pool, `max_connections` far above the ramp's concurrency, a 1 s
//! stats snapshot and file logging. Certificates are the repository's own test
//! pair (`rmqtt-bin/rmqtt.pem` + `rmqtt.key`, CA `rmqtt-bin/root.pem`,
//! server-auth only), so the fixture needs no certificate files of its own.
//!
//! ## Running it
//!
//! ```text
//! cargo build -p rmqttd -p rmqtt-test
//! target/debug/mqtt_harness --binary target/debug/rmqttd --workspace . \
//!   --suites functional_v5 --workers 1 --test issue532_tls
//! ```
//!
//! The broker is harness-managed (no `--no-broker`); its log lands in
//! `target/rmqtt-test-logs/issue532-ramp-tls.log`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{Expectation, TestCase, TestResult};
use crate::tests::functional::issue532_connect_ramp_stall::{
    run_ramp, tls_client_config, verdict, Counters, Endpoint, Transport, CA_PEM,
};

/// MQTT/TLS address of `configs/issue532-ramp-tls/rmqtt.toml`. The fixture owns
/// its ports instead of sharing the harness-wide `--addr`, so this case
/// advertises it through `broker_addr()` — the harness health-probes 1908 while
/// the sub-suite runs — and connects here itself. Must match
/// `listener.tls.external.addr` in that file.
pub(crate) const TLS_ADDR: &str = "127.0.0.1:1908";
/// HTTP port of `configs/issue532-ramp-tls/plugins/rmqtt-http-api.toml`. The
/// harness knows nothing about the plugin's own port, so it is pinned here and
/// must match that file.
const HTTP_ADDR: &str = "127.0.0.1:6072";
/// Log sink of the fixture (relative to the repository root, where the harness
/// and the broker run). Read after a stall: a frozen mtime is the "its log
/// ends" symptom of the incident.
const LOG_PATH: &str = "target/rmqtt-test-logs/issue532-ramp-tls.log";
/// Suite the case is reported under; the scheduler splits it into
/// `functional_v5@issue532-ramp-tls` because the case declares its own config.
const SUITE: &str = "functional_v5";

/// The connect-ramp stall of issue #532, reproduced on a single node with the
/// ramp arriving over TLS.
pub struct Issue532TlsConnectRampStallTest;

impl TestCase for Issue532TlsConnectRampStallTest {
    fn name(&self) -> &str {
        "issue532_tls_connect_ramp_stall"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        Some(crate::tests::config_path("issue532-ramp-tls"))
    }

    fn broker_addr(&self) -> Option<&'static str> {
        Some(TLS_ADDR)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        // One client config for the whole case: a fresh `ClientConfig` per
        // connection would add certificate parsing to every attempt and blur
        // the very cost this case is measuring.
        let transport = match tls_client_config(CA_PEM) {
            Ok(cfg) => Transport::Tls(cfg),
            Err(e) => {
                return TestResult::error(
                    self.name(),
                    SUITE,
                    start.elapsed(),
                    format!("could not build the TLS client config from {CA_PEM}: {e}"),
                )
            }
        };
        let ep = Arc::new(Endpoint {
            addr: TLS_ADDR,
            http: HTTP_ADDR,
            log_path: LOG_PATH,
            client_prefix: "i532t",
            fixture: "configs/issue532-ramp-tls",
            transport,
        });

        let rt = tokio::runtime::Runtime::new().unwrap();
        let counters = Arc::new(Counters::default());
        let (stall, profile) = rt.block_on(run_ramp(counters.clone(), ep.clone()));

        // Collected *after* the ramp, on purpose: the incident is that the
        // TCP-level check stays green while handshakes stop completing.
        let tcp_alive = ctx.broker_healthy();
        verdict(self.name(), SUITE, &ep, &counters, stall, &profile, start.elapsed(), tcp_alive)
    }

    fn timeout(&self) -> Duration {
        // The plain case's budget (RAMP_DURATION + WARMUP + drain + evidence
        // probes) plus headroom for TLS handshake latency under the ramp.
        Duration::from_secs(180)
    }

    /// Assertive on purpose, exactly like its plain-TCP sibling: the case
    /// exists to reproduce issue #532, so a reproduced stall must fail the
    /// suite. Switch to `Expectation::ExpectedFail` if the suite is ever
    /// required to stay green while the defect is open.
    fn expectation(&self) -> Expectation {
        Expectation::Normal
    }
}
