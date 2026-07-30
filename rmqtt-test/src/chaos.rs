//! Chaos/Fault injection module
//!
//! Provides hooks for broker restart, client disconnect simulation,
//! packet loss, slow consumer, and network delay injection.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use rand::Rng;
use tracing::{info, warn};

use crate::framework::context::TestContext;

/// Chaos controller for fault injection
pub struct ChaosController {
    ctx: Arc<Mutex<TestContext>>,
    packet_loss_rate: f64,
    delay_range: (Duration, Duration),
    disconnect_rate: f64,
    enabled: bool,
}

impl ChaosController {
    /// Create a new chaos controller
    pub fn new(ctx: Arc<Mutex<TestContext>>) -> Self {
        Self {
            ctx,
            packet_loss_rate: 0.0,
            delay_range: (Duration::ZERO, Duration::ZERO),
            disconnect_rate: 0.0,
            enabled: false,
        }
    }

    /// Enable chaos with specified parameters
    pub fn enable(&mut self, packet_loss: f64, delay_min: Duration, delay_max: Duration, disconnect: f64) {
        self.packet_loss_rate = packet_loss;
        self.delay_range = (delay_min, delay_max);
        self.disconnect_rate = disconnect;
        self.enabled = true;
        info!(
            "Chaos enabled: loss={:.1}%, delay={:?}-{:?}, disconnect={:.1}%",
            packet_loss * 100.0,
            delay_min,
            delay_max,
            disconnect * 100.0
        );
    }

    /// Disable chaos
    pub fn disable(&mut self) {
        self.enabled = false;
        info!("Chaos disabled");
    }

    /// Should we drop this packet?
    pub fn should_drop_packet(&self) -> bool {
        if !self.enabled || self.packet_loss_rate == 0.0 {
            return false;
        }
        let mut rng = rand::rng();
        rng.random::<f64>() < self.packet_loss_rate
    }

    /// Should we disconnect this client?
    pub fn should_disconnect(&self) -> bool {
        if !self.enabled || self.disconnect_rate == 0.0 {
            return false;
        }
        let mut rng = rand::rng();
        rng.random::<f64>() < self.disconnect_rate
    }

    /// Get a random delay to inject
    pub fn get_delay(&self) -> Duration {
        if !self.enabled || self.delay_range.0.is_zero() {
            return Duration::ZERO;
        }
        let mut rng = rand::rng();
        let min_ms = self.delay_range.0.as_millis() as u64;
        let max_ms = self.delay_range.1.as_millis() as u64;
        let delay_ms = rng.random_range(min_ms..=max_ms);
        Duration::from_millis(delay_ms)
    }

    /// Restart the broker (chaos action) - synchronous
    pub fn restart_broker(&self) -> Result<(), anyhow::Error> {
        info!("Chaos: restarting broker");
        let ctx = self.ctx.lock();
        ctx.restart_broker()
    }

    /// Kill the broker (chaos action) - synchronous
    pub fn kill_broker(&self) -> Result<(), anyhow::Error> {
        warn!("Chaos: killing broker");
        let ctx = self.ctx.lock();
        ctx.kill_broker()
    }

    /// Check if broker is healthy - synchronous
    pub fn broker_healthy(&self) -> bool {
        let ctx = self.ctx.lock();
        ctx.broker_healthy()
    }
}
