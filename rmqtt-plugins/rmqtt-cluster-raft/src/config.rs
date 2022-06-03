use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;
use serde::de::{self};

use rmqtt::broker::types::NodeId;
use rmqtt::grpc::MessageType;
use rmqtt::Result;
use rmqtt::settings::deserialize_duration;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    pub message_type: MessageType,
    pub node_grpc_addrs: Vec<NodeAddr>,
    pub raft_peer_addrs: Vec<NodeAddr>,
    #[serde(default = "PluginConfig::try_lock_timeout_default", deserialize_with = "deserialize_duration")]
    pub try_lock_timeout: Duration,  //Message::HandshakeTryLock
}

impl PluginConfig {
    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }

    fn try_lock_timeout_default() -> Duration{
        Duration::from_secs(15)
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
