pub mod client;
pub mod server;

#[allow(dead_code)]
pub(crate) mod pb {
    include!(concat!(env!("OUT_DIR"), "/pb.rs"));
}

use crate::broker::types::{From, Publish};
use crate::Result;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Message {
    Forward(From, Publish),
    Forwards(Vec<(From, Publish)>),
    Data(Vec<u8>),
}

impl Message {
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(bincode::serialize(self).map_err(anyhow::Error::new)?)
    }
    pub fn decode(data: &[u8]) -> Result<Message> {
        Ok(bincode::deserialize::<Message>(data).map_err(anyhow::Error::new)?)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum MessageReply {
    Data(Vec<u8>),
    Success,
}

impl MessageReply {
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(bincode::serialize(self).map_err(anyhow::Error::new)?)
    }
    pub fn decode(data: &[u8]) -> Result<MessageReply> {
        Ok(bincode::deserialize::<MessageReply>(data).map_err(anyhow::Error::new)?)
    }
}
