//! Configuration for the message storage plugin.
//!
//! Defines [`PluginConfig`], [`Config`], and [`RamConfig`] for choosing
//! between in-memory (RAM) and external storage backends.

use serde::{
    de::{self, Deserializer},
    Deserialize, Serialize,
};

use std::time::Duration;

#[cfg(feature = "circuit-breaker")]
use rmqtt::context::CircuitBreakerConfig as GlobalCBConfig;
use rmqtt::utils::{deserialize_duration, Bytesize};
#[cfg(feature = "circuit-breaker")]
use rmqtt_storage::{
    CircuitBreakerConfig as StorageCircuitBreakerConfig,
    CountBasedWindowConfig as StorageCountBasedWindowConfig,
    TimeBasedWindowConfig as StorageTimeBasedWindowConfig, WindowConfig as StorageWindowConfig,
};

/// Top-level configuration for the message storage plugin.
#[derive(Debug, Clone, Deserialize)]
pub struct PluginConfig {
    #[serde(
        default = "PluginConfig::storage_default",
        deserialize_with = "PluginConfig::deserialize_storage"
    )]
    pub storage: Option<Config>,
    #[serde(default = "PluginConfig::cleanup_count_default")]
    pub cleanup_count: usize,
    /// Timeout for storage I/O operations and channel sends.
    /// Also used as the circuit breaker's per-operation timeout when
    /// the `circuit-breaker` feature is enabled.
    /// `0` = no timeout. Examples: `"5s"`, `"500ms"`. Default: `"15s"`.
    #[serde(default = "PluginConfig::backend_timeout_default", deserialize_with = "deserialize_duration")]
    pub backend_timeout: Duration,
}

impl PluginConfig {
    fn storage_default() -> Option<Config> {
        #[cfg(feature = "ram")]
        return Some(Config::Ram(RamConfig::default()));
        #[cfg(not(feature = "ram"))]
        None
    }

    fn cleanup_count_default() -> usize {
        5000
    }

    fn backend_timeout_default() -> Duration {
        Duration::from_secs(15)
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
                    Ok(ram) => Ok(Some(Config::Ram(ram))),
                }
            }
            #[cfg(any(feature = "redis", feature = "redis-cluster"))]
            Some("redis") | Some("redis-cluster") => {
                match serde_json::from_value::<rmqtt_storage::Config>(storage) {
                    Err(e) => Err(de::Error::custom(e.to_string())),
                    Ok(s_cfg) => Ok(Some(Config::Storage(s_cfg))),
                }
            }
            _ => Err(de::Error::custom(format!("Unsupported storage type, {typ:?}"))),
        }
    }

    /// Serializes the configuration to a JSON value.
    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
        let storage = match &self.storage {
            #[cfg(feature = "ram")]
            Some(Config::Ram(ram)) => serde_json::json!({
                "type": "ram",
                "ram": ram,
            }),
            #[cfg(any(feature = "redis", feature = "redis-cluster"))]
            Some(Config::Storage(s_cfg)) => serde_json::to_value(s_cfg).unwrap_or_default(),
            None => serde_json::Value::Null,
            #[allow(unreachable_patterns)]
            _ => serde_json::Value::Null,
        };
        serde_json::json!({
            "storage": storage,
            "cleanup_count": self.cleanup_count,
            "backend_timeout": format!("{:?}", self.backend_timeout),
        })
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
            name: "message".into(),
        }
    }
}

/// Storage backend selector (RAM or external storage).
#[derive(Debug, Clone)]
pub enum Config {
    #[cfg(feature = "ram")]
    Ram(RamConfig),
    #[cfg(any(feature = "redis", feature = "redis-cluster"))]
    Storage(rmqtt_storage::Config),
}

/// In-memory (RAM) storage configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RamConfig {
    pub cache_capacity: Bytesize,
    pub cache_max_count: usize,
    pub encode: bool,

    /// Maximum number of pending tasks in the TaskExecQueue.
    /// Beyond this, new tasks are rejected.
    /// Default: 300_000
    #[serde(default = "RamConfig::queue_max_default")]
    pub queue_max: usize,
}

impl Default for RamConfig {
    #[inline]
    fn default() -> Self {
        RamConfig {
            cache_capacity: Bytesize::from(1024 * 1024 * 1024 * 3),
            cache_max_count: usize::MAX,
            encode: false,
            queue_max: Self::queue_max_default(),
        }
    }
}

impl RamConfig {
    fn queue_max_default() -> usize {
        300_000
    }
}
