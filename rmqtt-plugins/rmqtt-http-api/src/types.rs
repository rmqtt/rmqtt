use std::time::Duration;

use rmqtt::{chrono, serde_json, anyhow, bincode};

use rmqtt::{NodeId, ClientId, UserName, Timestamp};
use rmqtt::settings::{deserialize_duration_option, serialize_duration_option};
use rmqtt::node::{BrokerInfo, NodeInfo, NodeStatus};
use rmqtt::{metrics::Metrics, stats::Stats};
use rmqtt::Result;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Message<'a> {
    BrokerInfo,
    NodeInfo,
    StateInfo,
    MetricsInfo,
    ClientSearch(ClientSearchParams),
    ClientGet {clientid: &'a str},
}

impl<'a> Message<'a> {
    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(bincode::serialize(self).map_err(anyhow::Error::new)?)
    }
    #[inline]
    pub fn decode(data: &[u8]) -> Result<Message> {
        Ok(bincode::deserialize::<Message>(data).map_err(anyhow::Error::new)?)
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum MessageReply {
    BrokerInfo(BrokerInfo),
    NodeInfo(NodeInfo),
    StateInfo(NodeStatus, Box<Stats>),
    MetricsInfo(Metrics),
    ClientSearch(Vec<ClientSearchResult>),
    ClientGet(Option<ClientSearchResult>),
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

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct ClientSearchParams {
    #[serde(default)]
    pub _limit: usize,
    pub clientid: Option<String>,
    pub username: Option<String>,
    pub ip_address: Option<String>,
    pub connected: Option<bool>,
    pub clean_start: Option<bool>,
    pub proto_ver: Option<u8>,
    pub _like_clientid: Option<String>, //Substring fuzzy search
    pub _like_username: Option<String>, //Substring fuzzy search
    #[serde(default, deserialize_with = "deserialize_duration_option", serialize_with = "serialize_duration_option")]
    pub _gte_created_at: Option<Duration>, //Greater than or equal search
    #[serde(default, deserialize_with = "deserialize_duration_option", serialize_with = "serialize_duration_option")]
    pub _lte_created_at: Option<Duration>, //Less than or equal search
    #[serde(default, deserialize_with = "deserialize_duration_option", serialize_with = "serialize_duration_option")]
    pub _gte_connected_at: Option<Duration>, //Greater than or equal search
    #[serde(default, deserialize_with = "deserialize_duration_option", serialize_with = "serialize_duration_option")]
    pub _lte_connected_at: Option<Duration>, //Less than or equal search
    pub _gte_mqueue_len: Option<usize>,   //Current length of message queue, Greater than or equal search
    pub _lte_mqueue_len: Option<usize>,   //Current length of message queue, Less than or equal search
}

#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ClientSearchResult{
    pub node_id: NodeId,
    pub clientid: ClientId,
    pub username: UserName,
    pub proto_ver: u8,
    pub ip_address: Option<String>,
    pub port: Option<u16>,
    pub connected: bool,
    pub connected_at: Timestamp,
    pub disconnected_at: Timestamp,
    pub keepalive: u16,
    pub clean_start: bool,
    pub expiry_interval: i64,
    pub created_at: Timestamp,
    pub subscriptions_cnt: usize,
    pub max_subscriptions: usize,

    pub inflight: usize,
    pub max_inflight: usize,
//    pub inflight_dropped: usize,

    pub mqueue_len: usize,
    pub max_mqueue: usize,
//     pub mqueue_dropped: usize,

//    pub awaiting_rel:0,
//    pub max_awaiting_rel:s.listen_cfg.max_awaiting_rel,
//    pub awaiting_rel_dropped:0,

//     pub recv_msg:0,	//Number of received PUBLISH packets
//     pub send_msg:0,	//Number of sent PUBLISH packets
//     pub resend_msg:0, //Resent message data
//     pub ackeds:0,  //Number of Acked received
}

impl ClientSearchResult {

    #[inline]
    pub fn to_json(&self) -> serde_json::Value{
        let data = serde_json::json!({
        "node_id": self.node_id,
        "clientid": self.clientid,
        "username": self.username,
        "proto_ver": self.proto_ver,
        "ip_address": self.ip_address,
        "port": self.port,
        "connected": self.connected,
        "connected_at": format_timestamp(self.connected_at),
        "disconnected_at": format_timestamp(self.disconnected_at),
        "keepalive": self.keepalive,
        "clean_start": self.clean_start,
        "expiry_interval": self.expiry_interval,
        "created_at": format_timestamp(self.created_at),
        "subscriptions_cnt": self.subscriptions_cnt,
        "max_subscriptions": self.max_subscriptions,

        "inflight": self.inflight,
        "max_inflight": self.max_inflight,
        //"inflight_dropped": 0,

        "mqueue_len": self.mqueue_len,
        "max_mqueue": self.max_mqueue,
        // "mqueue_dropped": 0,

        //"awaiting_rel": 0,
        //"max_awaiting_rel": s.listen_cfg.max_awaiting_rel,
        //"awaiting_rel_dropped": 0,

        // "recv_msg": 0,	//Number of received PUBLISH packets
        // "send_msg": 0,	//Number of sent PUBLISH packets
        // "resend_msg": 0, //Resent message data
        // "ackeds": 0,  //Number of Acked received

    });
        data
    }
}


#[inline]
fn format_timestamp(t: i64) -> String {
    if t <= 0 {
        "".into()
    } else {
        use chrono::TimeZone;
        chrono::Local
            .timestamp(t, 0)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    }
}