use std::net::SocketAddr;
use std::time::Duration;

use rmqtt::serde_json;
use rmqtt::{
    grpc::MessageType,
    settings::{deserialize_addr, deserialize_duration},
    Result,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default = "PluginConfig::workers_default")]
    pub workers: usize,

    #[serde(default = "PluginConfig::max_row_limit_default")]
    pub max_row_limit: usize,

    #[serde(default = "PluginConfig::http_laddr_default", deserialize_with = "deserialize_addr")]
    pub http_laddr: SocketAddr,

    #[serde(
        default = "PluginConfig::metrics_sample_interval_default",
        deserialize_with = "deserialize_duration"
    )]
    pub metrics_sample_interval: Duration,

    #[serde(default = "PluginConfig::message_type_default")]
    pub message_type: MessageType,
}

impl PluginConfig {
    fn workers_default() -> usize {
        1
    }

    fn max_row_limit_default() -> usize {
        10_000
    }

    fn http_laddr_default() -> SocketAddr {
        "0.0.0.0:6060".parse::<std::net::SocketAddr>().unwrap()
    }

    fn metrics_sample_interval_default() -> Duration {
        Duration::from_secs(5)
    }

    fn message_type_default() -> MessageType {
        99
    }

    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }

    #[inline]
    pub fn changed(&self, other: &Self) -> bool {
        self.workers != other.workers
            || self.max_row_limit != other.max_row_limit
            || self.http_laddr != other.http_laddr
            || self.metrics_sample_interval != other.metrics_sample_interval
    }

    #[inline]
    pub fn restart_enable(&self, other: &Self) -> bool {
        self.workers != other.workers || self.http_laddr != other.http_laddr
    }
}
