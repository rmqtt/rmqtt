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
        postcard::to_stdvec(self).map_err(anyhow::Error::new)
    }
    #[inline]
    pub fn decode(data: &[u8]) -> Result<Self> {
        postcard::from_bytes::<Self>(data).map_err(anyhow::Error::new)
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum BroadcastGrpcMessageReply {
    GetNodeHealthStatus(NodeHealthStatus),
}

impl BroadcastGrpcMessageReply {
    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        postcard::to_stdvec(self).map_err(anyhow::Error::new)
    }
    #[inline]
    pub fn decode(data: &[u8]) -> Result<Self> {
        postcard::from_bytes::<Self>(data).map_err(anyhow::Error::new)
    }
}
