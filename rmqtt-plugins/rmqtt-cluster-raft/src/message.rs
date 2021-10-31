use rmqtt::broker::types::{NodeId, QoS, SharedGroup};
use rmqtt::Result;

#[derive(Serialize, Deserialize, Debug)]
pub enum Message<'a> {
    Connected {
        node_id: NodeId,
        client_id: &'a str,
    },
    Disconnected {
        client_id: &'a str,
    },
    SessionTerminated {
        client_id: &'a str,
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
    pub fn decode(data: &'a [u8]) -> Result<Self> {
        Ok(bincode::deserialize::<Self>(data).map_err(anyhow::Error::new)?)
    }
}
