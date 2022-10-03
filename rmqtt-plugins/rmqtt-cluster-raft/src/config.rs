use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

pub(crate) use backoff::{ExponentialBackoff, ExponentialBackoffBuilder};
pub(crate) use backoff::future::retry;
use serde::de::{self};

use rmqtt::{lazy_static, serde_json};
use rmqtt::broker::types::NodeId;
use rmqtt::grpc::MessageType;
use rmqtt::Result;
use rmqtt::settings::deserialize_duration;

lazy_static::lazy_static! {
    pub static ref BACKOFF_STRATEGY: ExponentialBackoff = ExponentialBackoffBuilder::new()
        .with_max_elapsed_time(Some(Duration::from_secs(60)))
        .with_multiplier(2.5).build();
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default = "PluginConfig::message_type_default")]
    pub message_type: MessageType,
    pub node_grpc_addrs: Vec<NodeAddr>,
    pub raft_peer_addrs: Vec<NodeAddr>,
    #[serde(default = "PluginConfig::try_lock_timeout_default", deserialize_with = "deserialize_duration")]
    pub try_lock_timeout: Duration, //Message::HandshakeTryLock

    #[serde(default = "PluginConfig::executor_workers_default")]
    pub executor_workers: usize,
    #[serde(default = "PluginConfig::executor_queue_max_default")]
    pub executor_queue_max: usize,
}

impl PluginConfig {
    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }

    fn message_type_default() -> MessageType {
        198
    }

    fn try_lock_timeout_default() -> Duration {
        Duration::from_secs(10)
    }

    fn executor_workers_default() -> usize {
        500
    }

    fn executor_queue_max_default() -> usize {
        100_000
    }
}

#[derive(Clone, Serialize)]
pub struct NodeAddr {
    pub id: NodeId,
    pub addr: SocketAddr,
}

impl std::fmt::Debug for NodeAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{:?}", self.id, self.addr)
    }
}

impl<'de> de::Deserialize<'de> for NodeAddr {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
        where
            D: de::Deserializer<'de>,
    {
        let node_addr = String::deserialize(deserializer)?;
        let parts: Vec<&str> = node_addr.split('@').collect();
        if parts.len() < 2 {
            return Err(de::Error::custom(format!(
                "Plugin \"rmqtt-cluster-raft\" \"node_grpc_addrs\" config error, {}",
                node_addr
            )));
        }
        let id = NodeId::from_str(parts[0]).map_err(de::Error::custom)?;
        let addr = parts[1].parse().map_err(de::Error::custom)?;
        Ok(NodeAddr { id, addr })
    }
}
