use serde::{Deserialize, Serialize};

use rmqtt::types::{Id, NodeHealthStatus, NodeId, SubscriptionOptions};
use rmqtt::Result;

use super::Mailbox;

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
}

impl<'a> Message<'a> {
    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        bincode::serialize(self).map_err(anyhow::Error::new)
    }
    #[inline]
    pub fn _decode(data: &'a [u8]) -> Result<Self> {
        bincode::deserialize::<Self>(data).map_err(anyhow::Error::new)
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum MessageReply {
    Error(String),
    HandshakeTryLock(Option<Id>),
    Ping,
}

impl MessageReply {
    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        bincode::serialize(self).map_err(anyhow::Error::new)
    }
    #[inline]
    pub fn decode(data: &[u8]) -> Result<MessageReply> {
        bincode::deserialize::<MessageReply>(data).map_err(anyhow::Error::new)
    }
}

#[inline]
pub(crate) async fn get_client_node_id(raft_mailbox: Mailbox, client_id: &str) -> Result<Option<NodeId>> {
    let msg = Message::GetClientNodeId { client_id }.encode()?;
    let reply = raft_mailbox.query(msg).await.map_err(anyhow::Error::new)?;
    if !reply.is_empty() {
        Ok(bincode::deserialize(&reply).map_err(anyhow::Error::new)?)
    } else {
        Ok(None)
    }
}

#[allow(dead_code)]
#[inline]
pub async fn ping(raft_mailbox: &Mailbox) -> Result<Option<MessageReply>> {
    let msg = Message::Ping.encode()?;
    let reply = raft_mailbox.send_proposal(msg).await.map_err(anyhow::Error::new)?;
    if !reply.is_empty() {
        Ok(Some(MessageReply::decode(&reply)?))
    } else {
        Ok(None)
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum RaftGrpcMessage {
    GetNodeHealthStatus,
}

impl RaftGrpcMessage {
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
pub enum RaftGrpcMessageReply {
    GetNodeHealthStatus(NodeHealthStatus),
}

impl RaftGrpcMessageReply {
    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        bincode::serialize(self).map_err(anyhow::Error::new)
    }
    #[inline]
    pub fn decode(data: &[u8]) -> Result<Self> {
        bincode::deserialize::<Self>(data).map_err(anyhow::Error::new)
    }
}
