//! Configuration for the message storage plugin.
//!
//! Defines [`PluginConfig`], [`Config`], and [`RamConfig`] for choosing
//! between in-memory (RAM) and external storage backends.

use serde::{
    de::{self, Deserializer},
    Deserialize, Serialize,
};

use std::time::Duration;

use rmqtt::utils::{deserialize_duration, Bytesize};

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
    /// Timeout for storage I/O operations. 0 = no timeout. Examples: "5s", "500ms".
    #[serde(default = "PluginConfig::timeout_default", deserialize_with = "deserialize_duration")]
    pub timeout: Duration,

    // ─── Circuit breaker ─────────────────────────────────────────────────────
    /// Enable circuit breaker for Redis storage operations.
    /// When enabled and the circuit is OPEN, store/mark_forwarded/get
    /// return immediately without touching Redis.
    #[serde(default = "PluginConfig::circuit_breaker_enabled_default")]
    pub circuit_breaker_enabled: bool,

    /// Consecutive failures before tripping to OPEN.
    #[serde(default = "PluginConfig::circuit_failure_threshold_default")]
    pub circuit_failure_threshold: usize,

    /// Duration in OPEN state before transitioning to HALF_OPEN (probe).
    /// Example: "30s", "1m".
    #[serde(
        default = "PluginConfig::circuit_reset_timeout_default",
        deserialize_with = "deserialize_duration"
    )]
    pub circuit_reset_timeout: Duration,

    /// Consecutive probe successes in HALF_OPEN before closing the circuit.
    #[serde(default = "PluginConfig::circuit_half_open_success_threshold_default")]
    pub circuit_half_open_success_threshold: usize,
}

impl PluginConfig {
    fn storage_default() -> Option<Config> {
        #[cfg(feature = "ram")]
        return Some(Config::Ram(RamConfig::default()));
        #[cfg(not(feature = "ram"))]
        None
    }

    fn cleanup_count_default() -> usize {
        2000
    }

    fn timeout_default() -> Duration {
        Duration::from_millis(5000)
    }

    fn circuit_breaker_enabled_default() -> bool {
        true
    }

    fn circuit_failure_threshold_default() -> usize {
        10
    }

    fn circuit_reset_timeout_default() -> Duration {
        Duration::from_secs(15)
    }

    fn circuit_half_open_success_threshold_default() -> usize {
        3
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
                let backend_key = match typ {
                    Some("redis") => "redis",
                    _ => "redis-cluster",
                };
                let merge_on_read = storage
                    .as_object()
                    .and_then(|obj| obj.get(backend_key))
                    .and_then(|v| v.get("merge_on_read"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                match serde_json::from_value::<rmqtt_storage::Config>(storage) {
                    Err(e) => Err(de::Error::custom(e.to_string())),
                    Ok(s_cfg) => Ok(Some(Config::Storage(s_cfg, merge_on_read))),
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
            Some(Config::Storage(s_cfg, merge_on_read)) => {
                let mut map = serde_json::to_value(s_cfg).unwrap_or_default();
                if let Some(obj) = map.as_object_mut() {
                    let backend_key = if obj.contains_key("redis") { "redis" } else { "redis-cluster" };
                    if let Some(backend) = obj.get_mut(backend_key).and_then(|v| v.as_object_mut()) {
                        backend.insert("merge_on_read".to_string(), serde_json::json!(*merge_on_read));
                    }
                }
                map
            }
            None => serde_json::Value::Null,
            #[allow(unreachable_patterns)]
            _ => serde_json::Value::Null,
        };
        serde_json::json!({
            "storage": storage,
            "cleanup_count": self.cleanup_count,
            "timeout": format!("{:?}", self.timeout),
            "circuit_breaker_enabled": self.circuit_breaker_enabled,
            "circuit_failure_threshold": self.circuit_failure_threshold,
            "circuit_reset_timeout": format!("{:?}", self.circuit_reset_timeout),
            "circuit_half_open_success_threshold": self.circuit_half_open_success_threshold,
        })
    }
}

/// Storage backend selector (RAM or external storage).
#[derive(Debug, Clone)]
pub enum Config {
    #[cfg(feature = "ram")]
    Ram(RamConfig),
    #[cfg(any(feature = "redis", feature = "redis-cluster"))]
    Storage(rmqtt_storage::Config, bool),
}

/// In-memory (RAM) storage configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RamConfig {
    pub cache_capacity: Bytesize,
    pub cache_max_count: usize,
    pub encode: bool,
    #[serde(default = "RamConfig::merge_on_read_default")]
    pub merge_on_read: bool,

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
            cache_capacity: Bytesize::from(1024 * 1024 * 1024 * 2),
            cache_max_count: usize::MAX,
            encode: false,
            merge_on_read: true,
            queue_max: Self::queue_max_default(),
        }
    }
}

impl RamConfig {
    fn merge_on_read_default() -> bool {
        true
    }

    fn queue_max_default() -> usize {
        300_000
    }
}
