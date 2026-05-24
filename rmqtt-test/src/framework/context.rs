//! Test Context - shared state available to all test cases

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;

use crate::broker::BrokerProcess;

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
        Self {
            config,
            metrics: Arc::new(Mutex::new(Metrics::default())),
            broker: None,
        }
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
        rt.block_on(
            crate::mqtt::v3::MqttV3Client::connect(
                &self.config.broker_addr,
                client_id,
                self.config.connect_timeout,
            )
        ).expect("failed to connect v3 client")
    }

    /// Create an MQTT v3.1.1 client
    pub fn create_v311_client(&self, client_id: &str) -> crate::mqtt::v311::MqttV311Client {
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        rt.block_on(
            crate::mqtt::v311::MqttV311Client::connect(
                &self.config.broker_addr,
                client_id,
                self.config.connect_timeout,
            )
        ).expect("failed to connect v311 client")
    }

    /// Create an MQTT v5 client
    pub fn create_v5_client(&self, client_id: &str) -> crate::mqtt::v5::MqttV5Client {
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        rt.block_on(
            crate::mqtt::v5::MqttV5Client::connect(
                &self.config.broker_addr,
                client_id,
                self.config.connect_timeout,
            )
        ).expect("failed to connect v5 client")
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

    /// Restart the broker (for chaos testing) - synchronous
    pub fn restart_broker(&self) -> Result<(), anyhow::Error> {
        if let Some(ref broker) = self.broker {
            let mut b = broker.lock();
            b.restart()
        } else {
            Err(anyhow::anyhow!("no broker managed by this context"))
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
}
