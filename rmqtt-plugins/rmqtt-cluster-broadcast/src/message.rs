use serde::{Deserialize, Serialize};

use rmqtt::types::NodeHealthStatus;
use rmqtt::Result;

#[derive(Serialize, Deserialize, Debug)]
pub enum BroadcastGrpcMessage {
    GetNodeHealthStatus,
}

impl BroadcastGrpcMessage {
    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        bincode::serialize(self).map_err(anyhow::Error::new)
    }
    #[inline]
    pub fn decode(data: &[u8]) -> Result<Self> {
        bincode::deserialize::<Self>(data).map_err(anyhow::Error::new)
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum BroadcastGrpcMessageReply {
    GetNodeHealthStatus(NodeHealthStatus),
}

impl BroadcastGrpcMessageReply {
    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        bincode::serialize(self).map_err(anyhow::Error::new)
    }
    #[inline]
    pub fn decode(data: &[u8]) -> Result<Self> {
        bincode::deserialize::<Self>(data).map_err(anyhow::Error::new)
    }
}
