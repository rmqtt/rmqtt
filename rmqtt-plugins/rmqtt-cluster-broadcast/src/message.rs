//! gRPC message types for the cluster broadcast plugin.
//!
//! Defines [`BroadcastGrpcMessage`] and [`BroadcastGrpcMessageReply`] for
//! custom inter-node communication (e.g., health status queries).

use serde::{Deserialize, Serialize};

use rmqtt::types::NodeHealthStatus;
use rmqtt::Result;

/// Custom gRPC messages for the cluster broadcast plugin.
#[derive(Serialize, Deserialize, Debug)]
pub enum BroadcastGrpcMessage {
    /// Request the health status of a remote node.
    GetNodeHealthStatus,
}

impl BroadcastGrpcMessage {
    /// Encodes this message into a byte vector using postcard.
    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        postcard::to_stdvec(self).map_err(anyhow::Error::new)
    }
    /// Decodes this message from a byte slice.
    #[inline]
    pub fn decode(data: &[u8]) -> Result<Self> {
        postcard::from_bytes::<Self>(data).map_err(anyhow::Error::new)
    }
}

/// Reply messages for [`BroadcastGrpcMessage`].
#[derive(Serialize, Deserialize, Debug)]
pub enum BroadcastGrpcMessageReply {
    /// Contains the health status of a remote node.
    GetNodeHealthStatus(NodeHealthStatus),
}

impl BroadcastGrpcMessageReply {
    /// Encodes this reply into a byte vector using postcard.
    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        postcard::to_stdvec(self).map_err(anyhow::Error::new)
    }
    /// Decodes this reply from a byte slice.
    #[inline]
    pub fn decode(data: &[u8]) -> Result<Self> {
        postcard::from_bytes::<Self>(data).map_err(anyhow::Error::new)
    }
}
