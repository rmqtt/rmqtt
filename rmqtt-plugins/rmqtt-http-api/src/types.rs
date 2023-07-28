use serde::de::{self, Deserialize};
use serde::ser::{self, Serialize};
use std::time::Duration;

use rmqtt::chrono::LocalResult;
use rmqtt::node::{BrokerInfo, NodeInfo, NodeStatus};
use rmqtt::plugin::PluginInfo;
use rmqtt::settings::{deserialize_datetime_option, serialize_datetime_option};
use rmqtt::Result;
use rmqtt::{anyhow, bincode, chrono, serde_json, HashMap, MqttError, QoS};
use rmqtt::{metrics::Metrics, stats::Stats};
use rmqtt::{ClientId, NodeId, Timestamp, TopicFilter, TopicName, UserName};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Message<'a> {
    BrokerInfo,
    NodeInfo,
    StatsInfo,
    MetricsInfo,
    ClientSearch(Box<ClientSearchParams>),
    ClientGet { clientid: &'a str },
    Subscribe(SubscribeParams),
    Unsubscribe(UnsubscribeParams),
    GetPlugins,
    GetPlugin { name: &'a str },
    GetPluginConfig { name: &'a str },
    ReloadPluginConfig { name: &'a str },
    LoadPlugin { name: &'a str },
    UnloadPlugin { name: &'a str },
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
    StatsInfo(NodeStatus, Box<Stats>),
    MetricsInfo(Metrics),
    ClientSearch(Vec<ClientSearchResult>),
    ClientGet(Option<ClientSearchResult>),
    Subscribe(HashMap<TopicFilter, (bool, Option<String>)>),
    Unsubscribe,
    GetPlugins(Vec<PluginInfo>),
    GetPlugin(Option<PluginInfo>),
    GetPluginConfig(Vec<u8>),
    ReloadPluginConfig,
    LoadPlugin,
    UnloadPlugin(bool),
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
    pub session_present: Option<bool>,
    pub proto_ver: Option<u8>,
    pub _like_clientid: Option<String>,
    //Substring fuzzy search
    pub _like_username: Option<String>,
    //Substring fuzzy search
    #[serde(
        default,
        deserialize_with = "deserialize_datetime_option",
        serialize_with = "serialize_datetime_option"
    )]
    pub _gte_created_at: Option<Duration>,
    //Greater than or equal search
    #[serde(
        default,
        deserialize_with = "deserialize_datetime_option",
        serialize_with = "serialize_datetime_option"
    )]
    pub _lte_created_at: Option<Duration>,
    //Less than or equal search
    #[serde(
        default,
        deserialize_with = "deserialize_datetime_option",
        serialize_with = "serialize_datetime_option"
    )]
    pub _gte_connected_at: Option<Duration>,
    //Greater than or equal search
    #[serde(
        default,
        deserialize_with = "deserialize_datetime_option",
        serialize_with = "serialize_datetime_option"
    )]
    pub _lte_connected_at: Option<Duration>,
    //Less than or equal search
    pub _gte_mqueue_len: Option<usize>,
    //Current length of message queue, Greater than or equal search
    pub _lte_mqueue_len: Option<usize>, //Current length of message queue, Less than or equal search
}

#[derive(Deserialize, Serialize, Debug, Default)]
pub struct ClientSearchResult {
    pub node_id: NodeId,
    pub clientid: ClientId,
    pub username: UserName,
    pub superuser: bool,
    pub proto_ver: u8,
    pub ip_address: Option<String>,
    pub port: Option<u16>,
    pub connected: bool,
    pub connected_at: Timestamp,
    pub disconnected_at: Timestamp,
    pub disconnected_reason: String,
    pub keepalive: u16,
    pub clean_start: bool,
    pub session_present: bool,
    pub expiry_interval: i64,
    pub created_at: Timestamp,
    pub subscriptions_cnt: usize,
    pub max_subscriptions: usize,
    pub extra_attrs: usize,
    #[serde(
        default,
        serialize_with = "ClientSearchResult::serialize_last_will",
        deserialize_with = "ClientSearchResult::deserialize_last_will"
    )]
    pub last_will: serde_json::Value,

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
    fn serialize_last_will<S>(last_will: &serde_json::Value, s: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        serde_json::to_vec(last_will).map_err(ser::Error::custom)?.serialize(s)
    }

    #[inline]
    pub fn deserialize_last_will<'de, D>(d: D) -> std::result::Result<serde_json::Value, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        serde_json::from_slice(&Vec::deserialize(d)?).map_err(de::Error::custom)
    }

    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
        let data = serde_json::json!({
            "node_id": self.node_id,
            "clientid": self.clientid,
            "username": self.username,
            "superuser": self.superuser,
            "proto_ver": self.proto_ver,
            "ip_address": self.ip_address,
            "port": self.port,
            "connected": self.connected,
            "connected_at": format_timestamp(self.connected_at),
            "disconnected_at": format_timestamp(self.disconnected_at),
            "disconnected_reason": self.disconnected_reason,
            "keepalive": self.keepalive,
            "clean_start": self.clean_start,
            "session_present": self.session_present,
            "expiry_interval": self.expiry_interval,
            "created_at": format_timestamp(self.created_at),
            "subscriptions_cnt": self.subscriptions_cnt,
            "max_subscriptions": self.max_subscriptions,
            "extra_attrs": self.extra_attrs,
            "last_will": self.last_will,

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

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct PublishParams {
    //For topic and topics, with at least one of them specified
    pub topic: Option<TopicName>,
    //Multiple topics separated by ,. This field is used to publish messages to multiple topics at the same time
    pub topics: Option<TopicName>,
    //Client identifier. Default:　system
    #[serde(default = "PublishParams::clientid_default")]
    pub clientid: ClientId,
    //Message body
    pub payload: String,
    //The encoding used in the message body. Currently only plain and base64 are supported. Default:　plain
    #[serde(default = "PublishParams::encoding_default")]
    pub encoding: String,
    //QoS level, Default: 0
    #[serde(default = "PublishParams::qos_default")]
    pub qos: u8,
    //Whether it is a retained message, Default: false
    #[serde(default = "PublishParams::retain_default")]
    pub retain: bool,
}

impl PublishParams {
    fn clientid_default() -> ClientId {
        "system".into()
    }

    fn encoding_default() -> String {
        "plain".into()
    }

    fn qos_default() -> u8 {
        0
    }

    fn retain_default() -> bool {
        false
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct SubscribeParams {
    //For topic and topics, with at least one of them specified
    pub topic: Option<TopicFilter>,
    //Multiple topics separated by,. This field is used to subscribe to multiple topics at the same time
    pub topics: Option<TopicFilter>,
    //Client identifier, Required
    pub clientid: ClientId,
    //QoS level, Default: 0
    #[serde(default = "SubscribeParams::qos_default")]
    pub qos: u8,
}

impl SubscribeParams {
    fn qos_default() -> u8 {
        0
    }

    #[inline]
    pub fn topics(&self) -> Result<Vec<TopicFilter>> {
        let mut topics = if let Some(topics) = &self.topics {
            topics.split(',').collect::<Vec<_>>().iter().map(|t| TopicName::from(t.trim())).collect()
        } else {
            Vec::new()
        };
        if let Some(topic) = &self.topic {
            topics.push(topic.clone());
        }
        if topics.is_empty() {
            return Err(MqttError::Msg("topics or topic is empty".into()));
        }
        Ok(topics)
    }

    #[inline]
    pub fn qos(&self) -> Result<QoS> {
        Ok(QoS::try_from(self.qos).map_err(|e| anyhow::Error::msg(e.to_string()))?)
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct UnsubscribeParams {
    pub topic: TopicFilter,
    pub clientid: ClientId,
}

#[inline]
fn format_timestamp(t: i64) -> String {
    if t <= 0 {
        "".into()
    } else {
        use chrono::TimeZone;
        if let LocalResult::Single(t) = chrono::Local.timestamp_opt(t, 0) {
            t.format("%Y-%m-%d %H:%M:%S").to_string()
        } else {
            "".into()
        }
    }
}
