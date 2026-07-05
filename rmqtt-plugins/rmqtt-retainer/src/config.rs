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
use rmqtt_storage::{CircuitBreakerConfig, CountBasedWindowConfig, TimeBasedWindowConfig, WindowConfig};

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

    // ─── Circuit breaker ─────────────────────────────────────────────────
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

    /// Per-operation timeout. Set to "0s" to disable. Default: "8s"
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
