//! Configuration for the retained messages plugin.
//!
//! Defines [`PluginConfig`], [`Config`], and [`RamConfig`] for configuring
//! storage backend (RAM or persistent), message limits, payload size caps,
//! and TTL for retained messages.

use std::time::Duration;

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

use rmqtt::utils::{deserialize_duration, deserialize_duration_option, Bytesize};
use rmqtt::Result;

#[cfg(feature = "circuit-breaker")]
use rmqtt::context::CircuitBreakerConfig as GlobalCBConfig;
#[cfg(feature = "circuit-breaker")]
use rmqtt_storage::{
    CircuitBreakerConfig as StorageCircuitBreakerConfig,
    CountBasedWindowConfig as StorageCountBasedWindowConfig,
    TimeBasedWindowConfig as StorageTimeBasedWindowConfig, WindowConfig as StorageWindowConfig,
};

/// Top-level configuration for the retained messages plugin.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(
        default = "PluginConfig::storage_default",
        deserialize_with = "PluginConfig::deserialize_storage"
    )]
    pub storage: Option<Config>,

    // The maximum number of retained messages, where 0 indicates no limit. After the number of reserved messages exceeds
    // the maximum limit, existing reserved messages can be replaced, but reserved messages cannot be stored for new topics.
    #[serde(default = "PluginConfig::max_retained_messages_default")]
    pub max_retained_messages: isize, // = 0

    // The maximum Payload value for retaining messages. After the Payload size exceeds the maximum value, the RMQTT
    // message server will process the received reserved message as a regular message.
    #[serde(default = "PluginConfig::max_payload_size_default")]
    pub max_payload_size: Bytesize, // = "1MB"

    // TTL for retained messages. Set to 0 for no expiration.
    // If not specified, the message expiration time will be used by default.
    #[serde(default, deserialize_with = "deserialize_duration_option")]
    pub retained_message_ttl: Option<Duration>,

    // Maximum number of messages per batch (default: 500).
    #[serde(default = "PluginConfig::batch_messages_limit_default")]
    pub batch_messages_limit: usize,

    // ─── Backend timeout (circuit breaker) ───────────────────────────────
    /// Backend storage operation timeout (circuit breaker).
    ///
    /// Each storage call (Sled/Redis get/set/delete) that exceeds this
    /// duration is aborted and counted as a failure by the circuit breaker.
    /// Set to `"0s"` to disable. Default: `"8s"`.
    #[serde(default = "PluginConfig::backend_timeout_default", deserialize_with = "deserialize_duration")]
    pub backend_timeout: Duration,
}

impl PluginConfig {
    fn storage_default() -> Option<Config> {
        #[cfg(feature = "ram")]
        return Some(Config::Ram);
        #[cfg(not(feature = "ram"))]
        None
    }

    fn max_retained_messages_default() -> isize {
        0
    }

    fn max_payload_size_default() -> Bytesize {
        Bytesize::from(1024 * 1024)
    }

    fn batch_messages_limit_default() -> usize {
        500
    }

    fn backend_timeout_default() -> Duration {
        Duration::from_secs(8)
    }

    #[inline]
    fn deserialize_storage<'de, D>(deserializer: D) -> std::result::Result<Option<Config>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let storage = serde_json::Value::deserialize(deserializer)?;
        let typ = storage.as_object().and_then(|obj| obj.get("type").and_then(|typ| typ.as_str()));
        match typ {
            #[cfg(feature = "ram")]
            Some("ram") => {
                match storage
                    .as_object()
                    .and_then(|obj| {
                        obj.get("ram").map(|ram| serde_json::from_value::<RamConfig>(ram.clone()))
                    })
                    .unwrap_or_else(|| Ok(RamConfig::default()))
                {
                    Err(e) => Err(de::Error::custom(e.to_string())),
                    Ok(_) => Ok(Some(Config::Ram)),
                }
            }
            #[cfg(any(feature = "sled", feature = "redis"))]
            Some("sled") | Some("redis") => match serde_json::from_value::<rmqtt_storage::Config>(storage) {
                Err(e) => Err(de::Error::custom(e.to_string())),
                Ok(s_cfg) => Ok(Some(Config::Storage(s_cfg))),
            },
            _ => Err(de::Error::custom(format!("Unsupported storage type, {typ:?}"))),
        }
    }

    /// Serializes the configuration to a JSON value.
    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
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
            name: "retainer".into(),
        }
    }
}

/// Storage backend selector for retained messages.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum Config {
    #[cfg(feature = "ram")]
    Ram,
    #[cfg(any(feature = "sled", feature = "redis"))]
    Storage(rmqtt_storage::Config),
}

/// RAM-only retainer configuration (no additional options).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RamConfig {}
