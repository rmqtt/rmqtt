//! Configuration for the HTTP API plugin.
//!
//! Defines [`PluginConfig`] with HTTP server settings, message expiry,
//! metrics sampling, Prometheus cache intervals, and history flush.

use std::net::SocketAddr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use rmqtt::{
    grpc::MessageType,
    utils::{deserialize_addr, deserialize_duration},
    Result,
};

/// Top-level configuration for the HTTP API plugin.
///
/// Specifies the HTTP listen address, bearer token, message type for gRPC,
/// metrics/Prometheus settings, request logging options, and optional
/// history flush configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default = "PluginConfig::max_row_limit_default")]
    pub max_row_limit: usize,

    #[serde(default = "PluginConfig::http_laddr_default", deserialize_with = "deserialize_addr")]
    pub http_laddr: SocketAddr,

    #[serde(
        default = "PluginConfig::metrics_sample_interval_default",
        deserialize_with = "deserialize_duration"
    )]
    pub metrics_sample_interval: Duration,

    pub http_bearer_token: Option<String>,

    #[serde(default = "PluginConfig::message_type_default")]
    pub message_type: MessageType,

    #[serde(default = "PluginConfig::http_reuseaddr_default")]
    pub http_reuseaddr: bool,

    #[serde(default = "PluginConfig::http_reuseport_default")]
    pub http_reuseport: bool,

    #[serde(default = "PluginConfig::http_request_log_default")]
    pub http_request_log: bool,

    #[serde(
        default = "PluginConfig::message_expiry_interval_default",
        deserialize_with = "deserialize_duration"
    )]
    pub message_expiry_interval: Duration,

    #[serde(
        default = "PluginConfig::prometheus_metrics_cache_interval_default",
        deserialize_with = "deserialize_duration"
    )]
    pub prometheus_metrics_cache_interval: Duration,

    /// Optional static directory for the Dashboard SPA.
    /// If set, http-api serves the contents at `/dashboard/`.
    #[serde(default)]
    pub dashboard_static_dir: Option<String>,

    // ── History flush configuration ──────────────────────────────────────
    /// Stats/Metrics history storage config.
    /// When `None`, the history flush and query APIs are disabled.
    #[serde(default)]
    pub storage: Option<rmqtt_storage::Config>,

    /// Interval between history flush writes.
    /// Default: 5 seconds.
    #[serde(default = "PluginConfig::flush_interval_default", deserialize_with = "deserialize_duration")]
    pub flush_interval: Duration,

    /// TTL for each history data point.
    /// After this duration, the storage backend automatically evicts it.
    /// Default: 7 days.
    #[serde(default = "PluginConfig::history_retention_default", deserialize_with = "deserialize_duration")]
    pub history_retention: Duration,
}

impl PluginConfig {
    #[inline]
    fn max_row_limit_default() -> usize {
        10_000
    }

    #[inline]
    fn http_laddr_default() -> SocketAddr {
        ([0, 0, 0, 0], 6060).into()
    }

    #[inline]
    fn metrics_sample_interval_default() -> Duration {
        Duration::from_secs(5)
    }

    #[inline]
    fn message_type_default() -> MessageType {
        99
    }

    #[inline]
    fn http_reuseaddr_default() -> bool {
        true
    }

    #[inline]
    fn http_reuseport_default() -> bool {
        false
    }

    #[inline]
    fn http_request_log_default() -> bool {
        false
    }

    #[inline]
    fn message_expiry_interval_default() -> Duration {
        Duration::from_secs(300)
    }

    #[inline]
    fn prometheus_metrics_cache_interval_default() -> Duration {
        Duration::from_secs(5)
    }

    #[inline]
    fn flush_interval_default() -> Duration {
        Duration::from_secs(5)
    }

    #[inline]
    fn history_retention_default() -> Duration {
        Duration::from_secs(7 * 24 * 60 * 60)
    }

    /// Serializes the configuration to a JSON value.
    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }

    /// Returns `true` if any config values that require a hot-reload
    /// (without restart) have changed.
    #[inline]
    pub fn changed(&self, other: &Self) -> bool {
        self.max_row_limit != other.max_row_limit
            || self.http_laddr != other.http_laddr
            || self.metrics_sample_interval != other.metrics_sample_interval
            || self.http_request_log != other.http_request_log
            || self.prometheus_metrics_cache_interval != other.prometheus_metrics_cache_interval
            || self.dashboard_static_dir != other.dashboard_static_dir
    }

    /// Returns `true` if a full server restart is required (listen address
    /// changed).
    #[inline]
    pub fn restart_enable(&self, other: &Self) -> bool {
        self.http_laddr != other.http_laddr || self.dashboard_static_dir != other.dashboard_static_dir
    }
}
