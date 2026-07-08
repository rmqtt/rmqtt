//! Configuration for the session storage plugin.
//!
//! Defines [`PluginConfig`] wrapping an `rmqtt_storage::Config` for
//! persisting session state, plus a `backend_timeout` for circuit-breaker
//! per-operation timeout tuning.

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[cfg(feature = "circuit-breaker")]
use rmqtt::context::CircuitBreakerConfig as GlobalCBConfig;
use rmqtt::utils::deserialize_duration;
use rmqtt_storage::Config;
#[cfg(feature = "circuit-breaker")]
use rmqtt_storage::{
    CircuitBreakerConfig as StorageCircuitBreakerConfig,
    CountBasedWindowConfig as StorageCountBasedWindowConfig,
    TimeBasedWindowConfig as StorageTimeBasedWindowConfig, WindowConfig as StorageWindowConfig,
};

/// Top-level configuration for the session storage plugin.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    pub storage: Config,

    /// Backend storage operation timeout (circuit breaker).
    ///
    /// Each storage call (Sled/Redis get/set/delete) that exceeds this
    /// duration is aborted and counted as a failure by the circuit breaker.
    /// Set to `"0s"` to disable. Default: `"15s"`.
    #[serde(default = "PluginConfig::backend_timeout_default", deserialize_with = "deserialize_duration")]
    pub backend_timeout: Duration,
}

impl PluginConfig {
    fn backend_timeout_default() -> Duration {
        Duration::from_secs(15)
    }

    /// Serializes the configuration to a JSON value.
    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!(self)
    }

    /// Build a storage `CircuitBreakerConfig` by merging the global circuit-breaker
    /// config with this plugin's local `backend_timeout` override.
    #[cfg(feature = "circuit-breaker")]
    #[inline]
    pub(crate) fn to_cb_config(&self, global: &GlobalCBConfig) -> StorageCircuitBreakerConfig {
        let window = match &global.window {
            rmqtt::context::WindowConfig::CountBased(w) => {
                StorageWindowConfig::CountBased(StorageCountBasedWindowConfig {
                    sliding_window_size: w.sliding_window_size,
                })
            }
            rmqtt::context::WindowConfig::TimeBased(w) => {
                StorageWindowConfig::TimeBased(StorageTimeBasedWindowConfig {
                    sliding_window_duration: w.sliding_window_duration,
                    sliding_window_size: w.sliding_window_size,
                })
            }
        };
        let operation_timeout =
            if self.backend_timeout.is_zero() { None } else { Some(self.backend_timeout) };
        StorageCircuitBreakerConfig {
            failure_rate_threshold: global.failure_rate_threshold,
            window,
            minimum_number_of_calls: global.minimum_number_of_calls,
            wait_duration_in_open: global.wait_duration_in_open,
            slow_call_duration_threshold: global.slow_call_duration_threshold,
            slow_call_rate_threshold: global.slow_call_rate_threshold,
            operation_timeout,
            name: "session".into(),
        }
    }
}
