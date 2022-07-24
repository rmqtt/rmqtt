use std::net::SocketAddr;
use std::str::FromStr;

use serde::de::{self};

use rmqtt::broker::types::NodeId;
use rmqtt::grpc::MessageType;
use rmqtt::Result;
use rmqtt::serde_json;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default = "PluginConfig::message_type_default")]
    pub message_type: MessageType,

    pub node_grpc_addrs: Vec<NodeGrpcAddr>,
}

impl PluginConfig {
    fn message_type_default() -> MessageType {
        98
    }

    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
}

#[derive(Clone, Serialize)]
pub struct NodeGrpcAddr {
    pub id: NodeId,
    pub addr: SocketAddr,
}

impl std::fmt::Debug for NodeGrpcAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{:?}", self.id, self.addr)
    }
}

impl<'de> de::Deserialize<'de> for NodeGrpcAddr {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
        where
            D: de::Deserializer<'de>,
    {
        let node_addr = String::deserialize(deserializer)?;
        let parts: Vec<&str> = node_addr.split('@').collect();
        if parts.len() < 2 {
            return Err(de::Error::custom(format!(
                "Plugin \"rmqtt-cluster-broadcast\" \"node_grpc_addrs\" config error, {}",
                node_addr
            )));
        }
        let id = NodeId::from_str(parts[0]).map_err(de::Error::custom)?;
        let addr = parts[1].parse().map_err(de::Error::custom)?;
        Ok(NodeGrpcAddr { id, addr })
    }
}
