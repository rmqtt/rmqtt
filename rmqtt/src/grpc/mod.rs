pub mod client;
pub mod server;

#[allow(dead_code)]
pub(crate) mod pb {
    include!(concat!(env!("OUT_DIR"), "/pb.rs"));
}

use crate::broker::session::SessionOfflineInfo;
use crate::broker::types::{From, Id, Publish, Retain, Topic};
use crate::Result;
use bytes::Bytes;

///Reserved within 1000
pub type MessageType = u64;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Message {
    Forward(From, Publish),
    Forwards(Vec<(From, Publish)>),
    Kick(Id),
    GetRetains(Topic),
    NumberOfClients,
    NumberOfSessions,
    Bytes(Bytes),
}

impl Message {
    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(bincode::serialize(self).map_err(anyhow::Error::new)?)
    }
    #[inline]
    pub fn decode(data: &[u8]) -> Result<Message> {
        Ok(bincode::deserialize::<Message>(data).map_err(anyhow::Error::new)?)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum MessageReply {
    Success,
    Error(String),
    Kick(Option<SessionOfflineInfo>),
    GetRetains(Vec<(Topic, Retain)>),
    NumberOfClients(usize),
    NumberOfSessions(usize),
    Bytes(Vec<u8>),
}

impl MessageReply {
    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(bincode::serialize(self).map_err(anyhow::Error::new)?)
    }
    #[inline]
    pub fn decode(data: &[u8]) -> Result<MessageReply> {
        Ok(bincode::deserialize::<MessageReply>(data).map_err(anyhow::Error::new)?)
    }
}
