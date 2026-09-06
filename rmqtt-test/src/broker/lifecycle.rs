//! Broker process lifecycle management
//!
//! Starts rmqttd as a child process, manages its lifecycle,
//! and provides health checking. All operations are synchronous
//! to avoid nested runtime issues when called from within tests.

use std::path::PathBuf;
use std::process::Child;
use std::time::Duration;

use tracing::{info, warn};

use super::healthcheck::{health_check_sync, port_free_sync, wait_port_free_sync};

/// Default broker TCP address
const DEFAULT_BROKER_ADDR: &str = "127.0.0.1:1883";

/// Default (self-contained) broker config, relative to the workspace root
const DEFAULT_CONFIG_REL: &str = "rmqtt-test/configs/default/rmqtt.toml";

/// Broker process manager (synchronous)
pub struct BrokerProcess {
    /// Path to the broker binary
    binary_path: PathBuf,
    /// Broker listen address for health check
    addr: String,
    /// Config file path (optional)
    config_path: Option<PathBuf>,
    /// The running child process
    child: Option<Child>,
}

impl BrokerProcess {
    /// Create a new broker process manager
    ///
    /// Searches for the broker binary in target/release and target/debug,
    /// and uses the self-contained `rmqtt-test/configs/default/rmqtt.toml`
    /// as the default config (never the repository-root rmqtt.toml).
    pub fn new(workspace_root: Option<PathBuf>) -> Self {
        let binary_path = Self::find_binary(workspace_root.as_deref());
        let config_path =
            workspace_root.as_ref().map(|root| root.join(DEFAULT_CONFIG_REL)).filter(|p| p.exists());
        Self { binary_path, addr: DEFAULT_BROKER_ADDR.to_string(), config_path, child: None }
    }

    /// Create with a specific binary path and address
    pub fn with_config(binary_path: PathBuf, addr: String, config_path: Option<PathBuf>) -> Self {
        Self { binary_path, addr, config_path, child: None }
    }

    /// Find the broker binary
    pub fn find_binary(workspace_root: Option<&std::path::Path>) -> PathBuf {
        if let Some(root) = workspace_root {
            for dir in &["target/release", "target/debug"] {
                let path = root.join(dir).join("rmqttd.exe");
                if path.exists() {
                    info!("Found broker binary: {:?}", path);
                    return path;
                }
                // Also try without .exe (Linux/macOS)
                let path = root.join(dir).join("rmqttd");
                if path.exists() {
                    info!("Found broker binary: {:?}", path);
                    return path;
                }
            }
        }

        // Fallback: try current directory
        PathBuf::from("rmqttd")
    }

    /// Start the broker process (synchronous)
    pub fn start(&mut self) -> Result<(), anyhow::Error> {
        if self.child.is_some() {
            warn!("Broker already running, skipping start");
            return Ok(());
        }

        if !self.binary_path.exists() {
            return Err(anyhow::anyhow!("broker binary not found at {:?}", self.binary_path));
        }

        info!("Starting broker: {:?}", self.binary_path);

        // Pre-flight check: fail fast when the port is already occupied by a
        // residual broker or any other process, instead of waiting the full
        // health-check timeout for a broker that can never bind.
        if !port_free_sync(&self.addr) {
            return Err(anyhow::anyhow!(
                "port {} is already in use by another process; a residual broker may \
                 still be running (check: netstat -ano | findstr {})",
                self.addr,
                self.addr.rsplit(':').next().unwrap_or("?")
            ));
        }

        let mut cmd = std::process::Command::new(&self.binary_path);
        cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());

        if let Some(ref config) = self.config_path {
            // rmqttd accepts `-f`/`--config` (config filename); `-c` is rejected.
            cmd.arg("-f").arg(config);
        }

        let child = cmd.spawn()?;
        self.child = Some(child);

        // Wait for broker to become healthy. The timeout is generous (60s):
        // sled-backed configs (session-sled / *-stress) can take a while to
        // load offline sessions on startup when the sled has grown large.
        let healthy = self.wait_healthy(Duration::from_secs(60));
        if healthy {
            info!("Broker is healthy at {}", self.addr);
            Ok(())
        } else {
            let _ = self.kill();
            Err(anyhow::anyhow!("broker failed to become healthy within timeout"))
        }
    }

    /// Wait for the broker to become healthy (synchronous polling)
    pub fn wait_healthy(&self, timeout: Duration) -> bool {
        let start = std::time::Instant::now();
        let check_interval = Duration::from_millis(200);

        while start.elapsed() < timeout {
            if health_check_sync(&self.addr, Duration::from_secs(2)) {
                return true;
            }
            std::thread::sleep(check_interval);
        }
        false
    }

    /// Check if the broker is healthy (synchronous)
    pub fn health_check(&self) -> bool {
        health_check_sync(&self.addr, Duration::from_secs(2))
    }

    /// Stop the broker gracefully
    pub fn stop(&mut self) -> Result<(), anyhow::Error> {
        if let Some(mut child) = self.child.take() {
            info!("Stopping broker (PID: {:?})", child.id());
            let _ = child.kill();
            let _ = child.wait();
            // The process is reaped, but on some platforms the listening
            // socket can linger briefly; wait until the port is actually
            // released before returning (a follow-up restart would otherwise
            // race into WSAEADDRINUSE / EADDRINUSE).
            if !wait_port_free_sync(&self.addr, Duration::from_secs(5)) {
                warn!("port {} still in use 5s after broker stop", self.addr);
            }
            info!("Broker stopped");
        }
        Ok(())
    }

    /// Restart the broker
    pub fn restart(&mut self) -> Result<(), anyhow::Error> {
        self.stop()?;
        self.start()
    }

    /// Update the config file path (without restarting)
    pub fn set_config(&mut self, config: Option<PathBuf>) {
        self.config_path = config;
    }

    /// Get the currently configured config file path
    pub fn config_path(&self) -> Option<&PathBuf> {
        self.config_path.as_ref()
    }

    /// Stop the broker, switch to the given config file and start again
    pub fn restart_with_config(&mut self, config: Option<PathBuf>) -> Result<(), anyhow::Error> {
        self.stop()?;
        self.config_path = config;
        self.start()
    }

    /// Kill the broker immediately
    pub fn kill(&mut self) -> Result<(), anyhow::Error> {
        if let Some(mut child) = self.child.take() {
            info!("Killing broker (PID: {:?})", child.id());
            let _ = child.kill();
            let _ = child.wait();
            // Same rationale as `stop`: wait for the port to be released so a
            // subsequent start (e.g. chaos restart) cannot race the teardown.
            if !wait_port_free_sync(&self.addr, Duration::from_secs(5)) {
                warn!("port {} still in use 5s after broker kill", self.addr);
            }
        }
        Ok(())
    }

    /// Get the broker address
    pub fn addr(&self) -> &str {
        &self.addr
    }

    /// Check if the broker process is running
    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }
}

impl Drop for BrokerProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // Best effort cleanup
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
