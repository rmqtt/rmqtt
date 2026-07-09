//! Message types for Raft-based cluster communication.
//!
//! Defines [`Message`] / [`MessageReply`] for session and subscription
//! Raft proposals, and [`RaftGrpcMessage`] / [`RaftGrpcMessageReply`] for
//! gRPC-level queries (e.g., health status).

use serde::{Deserialize, Serialize};

use rmqtt::types::{Addr, Id, NodeHealthStatus, NodeId, SubscriptionOptions};
use rmqtt::Result;

use super::Mailbox;

/// Raft-proposal messages for managing client state and subscriptions.
#[derive(Serialize, Deserialize, Debug)]
pub enum Message<'a> {
    HandshakeTryLock { id: Id },
    Connected { id: Id },
    Disconnected { id: Id },
    SessionTerminated { id: Id },
    Add { topic_filter: &'a str, id: Id, opts: SubscriptionOptions },
    Remove { topic_filter: &'a str, id: Id },
    //get client node id
    GetClientNodeId { client_id: &'a str },
    Ping,
    NodeUp { node: ClusterNode },
    NodeDown { id: NodeId },
    NodeRemove { id: NodeId },
}

/// Business-layer cluster node metadata replicated through Raft.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClusterNode {
    pub id: NodeId,
    pub grpc_addr: Addr,
    pub raft_addr: Addr,
}

impl<'a> Message<'a> {
    /// Encodes this message into a byte vector using postcard.
    /// Encodes this message into a byte vector using postcard.
    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        postcard::to_stdvec(self).map_err(anyhow::Error::new)
    }
    /// Decodes this message from a byte slice.
    #[inline]
    pub fn _decode(data: &'a [u8]) -> Result<Self> {
        postcard::from_bytes::<Self>(data).map_err(anyhow::Error::new)
    }
}

/// Reply messages for Raft [`Message`] proposals.
#[derive(Serialize, Deserialize, Debug)]
pub enum MessageReply {
    Error(String),
    HandshakeTryLock(Option<Id>),
    Ping,
}

impl MessageReply {
    /// Encodes this reply into a byte vector using postcard.
    /// Encodes this reply into a byte vector using postcard.
    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        postcard::to_stdvec(self).map_err(anyhow::Error::new)
    }
    /// Decodes this reply from a byte slice.
    #[inline]
    pub fn decode(data: &[u8]) -> Result<MessageReply> {
        postcard::from_bytes::<MessageReply>(data).map_err(anyhow::Error::new)
    }
}

#[inline]
pub(crate) async fn get_client_node_id(raft_mailbox: Mailbox, client_id: &str) -> Result<Option<NodeId>> {
    let msg = Message::GetClientNodeId { client_id }.encode()?;
    let reply = raft_mailbox.query(msg).await.map_err(anyhow::Error::new)?;
    if !reply.is_empty() {
        Ok(postcard::from_bytes(&reply).map_err(anyhow::Error::new)?)
    } else {
        Ok(None)
    }
}

#[allow(dead_code)]
#[inline]
/// Sends a Ping proposal to the Raft cluster and returns the reply.
pub async fn ping(raft_mailbox: &Mailbox) -> Result<Option<MessageReply>> {
    let msg = Message::Ping.encode()?;
    let reply = raft_mailbox.send_proposal(msg).await.map_err(anyhow::Error::new)?;
    if !reply.is_empty() {
        Ok(Some(MessageReply::decode(&reply)?))
    } else {
        Ok(None)
    }
}

/// gRPC-level messages for the Raft plugin (e.g., health status queries).
#[derive(Serialize, Deserialize, Debug)]
pub enum RaftGrpcMessage {
    GetNodeHealthStatus,
}

impl RaftGrpcMessage {
    /// Encodes this gRPC message into a byte vector using postcard.
    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        postcard::to_stdvec(self).map_err(anyhow::Error::new)
    }
    /// Decodes this gRPC message from a byte slice.
    #[inline]
    pub fn decode(data: &[u8]) -> Result<Self> {
        postcard::from_bytes::<Self>(data).map_err(anyhow::Error::new)
    }
}

/// Reply messages for [`RaftGrpcMessage`].
#[derive(Serialize, Deserialize, Debug)]
pub enum RaftGrpcMessageReply {
    GetNodeHealthStatus(NodeHealthStatus),
}

impl RaftGrpcMessageReply {
    /// Encodes this reply into a byte vector using postcard.
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
