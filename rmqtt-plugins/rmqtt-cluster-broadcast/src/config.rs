use serde::{Deserialize, Serialize};
use std::time::Duration;

use rmqtt::grpc::MessageType;
use rmqtt::utils::{deserialize_duration, NodeAddr};
use rmqtt::Result;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default = "PluginConfig::message_type_default")]
    pub message_type: MessageType,

    pub node_grpc_addrs: Vec<NodeAddr>,

    #[serde(default = "PluginConfig::grpc_client_concurrency_limit_default")]
    pub node_grpc_client_concurrency_limit: usize,

    #[serde(
        default = "PluginConfig::grpc_client_timeout_default",
        deserialize_with = "deserialize_duration"
    )]
    pub node_grpc_client_timeout: Duration,

    //#Maximum number of messages sent in batch
    #[serde(default = "PluginConfig::grpc_batch_size_default")]
    pub node_grpc_batch_size: usize,
}

impl PluginConfig {
    fn message_type_default() -> MessageType {
        98
    }

    fn grpc_client_concurrency_limit_default() -> usize {
        128
    }
    fn grpc_client_timeout_default() -> Duration {
        Duration::from_secs(60)
    }

    fn grpc_batch_size_default() -> usize {
        128
    }

    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
}
