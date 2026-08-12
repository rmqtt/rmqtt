//! Test Context - shared state available to all test cases

use std::sync::Arc;
use std::time::{Duration, Instant};

use bytestring::ByteString;
use parking_lot::Mutex;
use tracing::info;
use uuid::Uuid;

use crate::broker::BrokerProcess;
use crate::framework::testcase::TestResult;

/// Metrics collected during test execution
#[derive(Debug, Default, Clone)]
pub struct Metrics {
    pub messages_sent: u64,
    pub messages_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub errors: u64,
    pub connect_count: u64,
    pub subscribe_count: u64,
}

/// Test configuration
#[derive(Debug, Clone)]
pub struct TestConfig {
    pub broker_addr: String,
    pub connect_timeout: Duration,
    pub default_test_timeout: Duration,
    pub parallel_workers: usize,
    pub verbose: bool,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            broker_addr: "127.0.0.1:1883".to_string(),
            connect_timeout: Duration::from_secs(10),
            default_test_timeout: Duration::from_secs(60),
            parallel_workers: 4,
            verbose: false,
        }
    }
}

/// Shared test context
pub struct TestContext {
    pub config: TestConfig,
    pub metrics: Arc<Mutex<Metrics>>,
    broker: Option<Arc<Mutex<BrokerProcess>>>,
}

impl TestContext {
    /// Create a new test context
    pub fn new(config: TestConfig) -> Self {
        Self { config, metrics: Arc::new(Mutex::new(Metrics::default())), broker: None }
    }

    /// Create with a broker process
    pub fn with_broker(config: TestConfig, broker: BrokerProcess) -> Self {
        Self {
            config,
            metrics: Arc::new(Mutex::new(Metrics::default())),
            broker: Some(Arc::new(Mutex::new(broker))),
        }
    }

    /// Create an MQTT v3 client
    pub fn create_v3_client(&self, client_id: &str) -> crate::mqtt::v3::MqttV3Client {
        // Note: v3 client uses async connect, so we return a pre-connected client
        // This is a sync method, so tests need to call connect themselves
        // For compatibility with test infrastructure, we use a runtime
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        rt.block_on(crate::mqtt::v3::MqttV3Client::connect(
            &self.config.broker_addr,
            client_id,
            self.config.connect_timeout,
        ))
        .expect("failed to connect v3 client")
    }

    /// Create an MQTT v3.1.1 client
    pub fn create_v311_client(&self, client_id: &str) -> crate::mqtt::v311::MqttV311Client {
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        rt.block_on(crate::mqtt::v311::MqttV311Client::connect(
            &self.config.broker_addr,
            client_id,
            self.config.connect_timeout,
        ))
        .expect("failed to connect v311 client")
    }

    /// Create an MQTT v5 client
    pub fn create_v5_client(&self, client_id: &str) -> crate::mqtt::v5::MqttV5Client {
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        rt.block_on(crate::mqtt::v5::MqttV5Client::connect(
            &self.config.broker_addr,
            client_id,
            self.config.connect_timeout,
        ))
        .expect("failed to connect v5 client")
    }

    /// Record a message sent
    pub fn record_sent(&self, bytes: u64) {
        let mut m = self.metrics.lock();
        m.messages_sent += 1;
        m.bytes_sent += bytes;
    }

    /// Record a message received
    pub fn record_received(&self, bytes: u64) {
        let mut m = self.metrics.lock();
        m.messages_received += 1;
        m.bytes_received += bytes;
    }

    /// Record an error
    pub fn record_error(&self) {
        self.metrics.lock().errors += 1;
    }

    /// Whether this context manages a broker process.
    ///
    /// Returns `false` in `--no-broker` mode (external broker), in which case
    /// tests that depend on broker lifecycle management (restart/kill) should
    /// be skipped rather than failed.
    pub fn has_broker(&self) -> bool {
        self.broker.is_some()
    }

    /// Restart the broker (for chaos testing) - synchronous
    pub fn restart_broker(&self) -> Result<(), anyhow::Error> {
        if let Some(ref broker) = self.broker {
            let mut b = broker.lock();
            b.restart()
        } else {
            Err(anyhow::anyhow!("no broker managed by this context"))
        }
    }

    /// Ensure the broker runs with the given config file (synchronous,
    /// idempotent).
    ///
    /// If the broker is already running with `target` (compared against
    /// `BrokerProcess::config_path`), nothing happens. Otherwise the broker
    /// is restarted with the new config. Returns `Ok(())` in `--no-broker`
    /// mode (nothing to switch); the caller decides how to warn.
    pub fn ensure_broker_config(&self, target: &std::path::Path) -> Result<(), anyhow::Error> {
        if let Some(ref broker) = self.broker {
            let mut b = broker.lock();
            if b.config_path().map(|p| p.as_path()) == Some(target) {
                return Ok(());
            }
            info!(
                "switching broker config: {:?} -> {:?}",
                b.config_path().map(|p| p.display().to_string()),
                target.display()
            );
            b.restart_with_config(Some(target.to_path_buf()))
        } else {
            Ok(())
        }
    }

    /// Kill the broker (for chaos testing) - synchronous
    pub fn kill_broker(&self) -> Result<(), anyhow::Error> {
        if let Some(ref broker) = self.broker {
            let mut b = broker.lock();
            b.kill()
        } else {
            Err(anyhow::anyhow!("no broker managed by this context"))
        }
    }

    /// Start the broker - synchronous
    pub fn start_broker(&self) -> Result<(), anyhow::Error> {
        if let Some(ref broker) = self.broker {
            let mut b = broker.lock();
            b.start()
        } else {
            Err(anyhow::anyhow!("no broker managed by this context"))
        }
    }

    /// Check broker health - synchronous
    pub fn broker_healthy(&self) -> bool {
        if let Some(ref broker) = self.broker {
            let b = broker.lock();
            b.health_check()
        } else {
            false
        }
    }

    /// Probe whether the broker advertises `Retain Available = 1` in CONNACK,
    /// i.e. whether retained messages are enabled (the `rmqtt-retainer`
    /// plugin is loaded). Synchronous: creates its own tokio runtime,
    /// matching the style of `create_v5_client`.
    ///
    /// Uses a raw v5 CONNECT (the CONNACK `retain_available` property only
    /// exists in the v5 protocol); the capability is a broker-wide property
    /// independent of the client's protocol version.
    pub fn retain_available(&self) -> Result<bool, anyhow::Error> {
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        rt.block_on(async {
            let (mut reader, mut writer) =
                crate::transport::tcp_v5::connect(&self.config.broker_addr, self.config.connect_timeout)
                    .await?;
            let connect = rmqtt_codec::v5::Connect {
                clean_start: true,
                keep_alive: 60,
                client_id: ByteString::from(format!("retain-probe-{}", Uuid::new_v4().as_simple())),
                ..Default::default()
            };
            writer.send_packet(&rmqtt_codec::v5::Packet::Connect(Box::new(connect))).await?;
            let pkt = tokio::time::timeout(Duration::from_secs(5), reader.read_packet()).await.map_err(
                |_| anyhow::anyhow!("timed out waiting for CONNACK while probing retain availability"),
            )??;
            match pkt {
                rmqtt_codec::v5::Packet::ConnectAck(ack) => Ok(ack.retain_available),
                other => Err(anyhow::anyhow!(
                    "expected CONNACK while probing retain availability, got {:?}",
                    crate::transport::tcp_v5::packet_name_v5(&other)
                )),
            }
        })
    }

    /// Guard for retain-dependent tests.
    ///
    /// If the broker does not support retained messages (the `rmqtt-retainer`
    /// plugin is not enabled, CONNACK advertises `Retain Available = 0`), the
    /// test is not executed and is reported as **passed** with an explanatory
    /// note. Returns `None` when retained messages ARE available and the test
    /// should run normally.
    pub fn guard_retain_required(&self, name: &str, suite: &str, start: Instant) -> Option<TestResult> {
        match self.retain_available() {
            Ok(true) => None,
            Ok(false) => Some(TestResult::passed_with_note(
                name,
                suite,
                start.elapsed(),
                "skipped: 'rmqtt-retainer' plugin not enabled (Retain Available = 0)",
            )),
            Err(e) => Some(TestResult::failed(
                name,
                suite,
                start.elapsed(),
                format!("failed to probe retain availability: {e}"),
            )),
        }
    }
}
