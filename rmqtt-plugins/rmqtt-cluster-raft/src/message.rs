use rmqtt::broker::types::{Id, NodeId, QoS, SharedGroup};
use rmqtt::Result;

#[derive(Serialize, Deserialize, Debug)]
pub enum Message<'a> {
    HandshakeTryLock{
        id: Id,
    },
    Connected {
        id: Id,
    },
    Disconnected {
        id: Id,
    },
    SessionTerminated {
        id: Id,
    },
    Add {
        topic_filter: &'a str,
        node_id: NodeId,
        client_id: &'a str,
        qos: QoS,
        shared_group: Option<SharedGroup>,
    },
    Remove {
        topic_filter: &'a str,
        node_id: NodeId,
        client_id: &'a str,
    },
    //get client node id
    GetClientNodeId {
        client_id: &'a str,
    },
}

impl<'a> Message<'a> {
    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(bincode::serialize(self).map_err(anyhow::Error::new)?)
    }
    #[inline]
    pub fn _decode(data: &'a [u8]) -> Result<Self> {
        Ok(bincode::deserialize::<Self>(data).map_err(anyhow::Error::new)?)
    }
}


#[derive(Serialize, Deserialize, Debug)]
pub enum MessageReply {
    // Success,
    Error(String),
    HandshakeTryLock(Option<Id>),
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