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
#[cfg(feature = "circuit-breaker")]
use rmqtt_storage::{CircuitBreakerConfig, CountBasedWindowConfig, TimeBasedWindowConfig, WindowConfig};

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

    /// Optional circuit-breaker tuning.
    /// When `None` (default), the built-in `CircuitBreakerConfig::default()`
    /// is used verbatim. When `Some(...)`, the given fields override the
    /// corresponding defaults.
    #[serde(default)]
    pub circuit_breaker: CircuitBreakerPluginConfig,
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

    fn timeout_default() -> Duration {
        Duration::from_millis(5000)
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
            "timeout": format!("{:?}", self.timeout),
            "circuit_breaker": serde_json::json!(self.circuit_breaker),
        })
    }

    #[cfg(feature = "circuit-breaker")]
    #[inline]
    pub(crate) fn to_cb_config(&self) -> CircuitBreakerConfig {
        let cb = &self.circuit_breaker;
        let window = match cb.sliding_window_type.as_str() {
            "TimeBased" | "time_based" | "time" => {
                let dur = if cb.sliding_window_duration.is_zero() {
                    TimeBasedWindowConfig::default().sliding_window_duration
                } else {
                    cb.sliding_window_duration
                };
                WindowConfig::TimeBased(TimeBasedWindowConfig {
                    sliding_window_duration: dur,
                    sliding_window_size: cb.sliding_window_size,
                })
            }
            _ => WindowConfig::CountBased(CountBasedWindowConfig {
                sliding_window_size: cb.sliding_window_size,
            }),
        };
        CircuitBreakerConfig {
            failure_rate_threshold: cb.failure_rate_threshold,
            window,
            minimum_number_of_calls: cb.minimum_number_of_calls,
            wait_duration_in_open: cb.wait_duration_in_open,
            slow_call_duration_threshold: cb.slow_call_duration_threshold,
            slow_call_rate_threshold: cb.slow_call_rate_threshold,
            operation_timeout: if cb.operation_timeout.is_zero() { None } else { Some(cb.operation_timeout) },
            ..CircuitBreakerConfig::default()
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

/// User-facing circuit-breaker settings.
///
/// Each field carries its own `serde(default = ...)` so that users can
/// specify only the values they wish to override inside `[circuit_breaker]`.
///
/// Fields that are not set in TOML fall back to the values below, which
/// match `CircuitBreakerConfig::default()` where practical.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CircuitBreakerPluginConfig {
    /// Failure rate threshold (0.0 – 1.0). Default: 0.25
    #[serde(default = "CircuitBreakerPluginConfig::failure_rate_threshold_default")]
    pub failure_rate_threshold: f64,

    /// Sliding window type: "CountBased" or "TimeBased". Default: "TimeBased"
    #[serde(default = "CircuitBreakerPluginConfig::sliding_window_type_default")]
    pub sliding_window_type: String,

    /// Sliding window size (number of calls). Default: 20
    #[serde(default = "CircuitBreakerPluginConfig::sliding_window_size_default")]
    pub sliding_window_size: usize,

    /// Sliding window duration for TimeBased mode. Only used when sliding_window_type
    /// is "TimeBased". Calls older than this duration are excluded from failure rate
    /// calculation. Default: "45s"
    #[serde(
        default = "CircuitBreakerPluginConfig::sliding_window_duration_default",
        deserialize_with = "deserialize_duration"
    )]
    pub sliding_window_duration: Duration,

    /// Minimum calls before the breaker can trip. Default: 10
    #[serde(default = "CircuitBreakerPluginConfig::minimum_number_of_calls_default")]
    pub minimum_number_of_calls: usize,

    /// Duration in OPEN state before transitioning to HALF_OPEN. Default: "30s"
    #[serde(
        default = "CircuitBreakerPluginConfig::wait_duration_in_open_default",
        deserialize_with = "deserialize_duration"
    )]
    pub wait_duration_in_open: Duration,

    /// Slow call duration threshold. Default: "2s"
    #[serde(
        default = "CircuitBreakerPluginConfig::slow_call_duration_threshold_default",
        deserialize_with = "deserialize_duration"
    )]
    pub slow_call_duration_threshold: Duration,

    /// Slow call rate threshold (0.0 – 1.0). 1.0 = disabled. Default: 1.0
    #[serde(default = "CircuitBreakerPluginConfig::slow_call_rate_threshold_default")]
    pub slow_call_rate_threshold: f64,

    /// Per-operation timeout. Set to "0s" to disable. Default: "0s" (disabled)
    #[serde(
        default = "CircuitBreakerPluginConfig::operation_timeout_default",
        deserialize_with = "deserialize_duration"
    )]
    pub operation_timeout: Duration,
}

impl Default for CircuitBreakerPluginConfig {
    fn default() -> Self {
        Self {
            failure_rate_threshold: Self::failure_rate_threshold_default(),
            sliding_window_type: Self::sliding_window_type_default(),
            sliding_window_size: Self::sliding_window_size_default(),
            sliding_window_duration: Self::sliding_window_duration_default(),
            minimum_number_of_calls: Self::minimum_number_of_calls_default(),
            wait_duration_in_open: Self::wait_duration_in_open_default(),
            slow_call_duration_threshold: Self::slow_call_duration_threshold_default(),
            slow_call_rate_threshold: Self::slow_call_rate_threshold_default(),
            operation_timeout: Self::operation_timeout_default(),
        }
    }
}

impl CircuitBreakerPluginConfig {
    fn failure_rate_threshold_default() -> f64 {
        0.25
    }
    fn sliding_window_type_default() -> String {
        "TimeBased".to_string()
    }
    fn sliding_window_size_default() -> usize {
        20
    }
    fn sliding_window_duration_default() -> Duration {
        Duration::from_secs(45)
    }
    fn minimum_number_of_calls_default() -> usize {
        10
    }
    fn wait_duration_in_open_default() -> Duration {
        Duration::from_secs(30)
    }
    fn slow_call_duration_threshold_default() -> Duration {
        Duration::from_secs(2)
    }
    fn slow_call_rate_threshold_default() -> f64 {
        1.0
    }
    fn operation_timeout_default() -> Duration {
        Duration::from_secs(8)
    }
}
