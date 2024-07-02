use anyhow::anyhow;
use std::any::Any;
use std::convert::From as _f;
use std::fmt;
use std::fmt::Display;
use std::hash::Hash;
use std::mem::{size_of, size_of_val};
use std::net::SocketAddr;
use std::num::{NonZeroU16, NonZeroU32};
use std::ops::Deref;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{Serialize, SerializeStruct, Serializer};
use tokio::sync::{oneshot, RwLock};

use base64::prelude::{Engine, BASE64_STANDARD};
use bitflags::*;
use bytestring::ByteString;
use get_size::GetSize;
use itertools::Itertools;

use ntex::util::Bytes;

use ntex_mqtt::error::SendPacketError;
pub use ntex_mqtt::types::{Protocol, MQTT_LEVEL_31, MQTT_LEVEL_311, MQTT_LEVEL_5};
pub use ntex_mqtt::v3::{
    self, codec::Connect as ConnectV3, codec::ConnectAckReason as ConnectAckReasonV3,
    codec::LastWill as LastWillV3, codec::Packet as PacketV3,
    codec::SubscribeReturnCode as SubscribeReturnCodeV3, HandshakeAck as HandshakeAckV3,
    MqttSink as MqttSinkV3,
};
use ntex_mqtt::v5::codec::{PublishAckReason, RetainHandling};
pub use ntex_mqtt::v5::{
    self, codec::Connect as ConnectV5, codec::ConnectAckReason as ConnectAckReasonV5,
    codec::Disconnect as DisconnectV5, codec::DisconnectReasonCode, codec::LastWill as LastWillV5,
    codec::Packet as PacketV5, codec::PublishAck2, codec::PublishAck2Reason,
    codec::PublishProperties as PublishPropertiesV5, codec::Subscribe as SubscribeV5,
    codec::SubscribeAck as SubscribeAckV5, codec::SubscribeAckReason,
    codec::SubscriptionOptions as SubscriptionOptionsV5, codec::Unsubscribe as UnsubscribeV5,
    codec::UnsubscribeAck as UnsubscribeAckV5, codec::UserProperties, codec::UserProperty,
    HandshakeAck as HandshakeAckV5, MqttSink as MqttSinkV5,
};
use ntex_mqtt::TopicLevel;

use crate::broker::fitter::Fitter;
use crate::broker::inflight::Inflight;
use crate::broker::queue::{Queue, Sender};
use crate::{MqttError, Result, Runtime};

pub type NodeId = u64;
pub type NodeName = String;
pub type RemoteSocketAddr = SocketAddr;
pub type LocalSocketAddr = SocketAddr;
pub type Addr = bytestring::ByteString;
pub type ClientId = bytestring::ByteString;
pub type UserName = bytestring::ByteString;
pub type Superuser = bool;
pub type Password = bytes::Bytes;
pub type PacketId = u16;
///topic name or topic filter
pub type TopicName = bytestring::ByteString;
pub type Topic = ntex_mqtt::Topic;
///topic filter
pub type TopicFilter = bytestring::ByteString;
pub type SharedGroup = bytestring::ByteString;
pub type IsDisconnect = bool;
pub type MessageExpiry = bool;
pub type TimestampMillis = i64;
pub type Timestamp = i64;
pub type IsOnline = bool;
pub type IsAdmin = bool;
pub type LimiterName = u16;
pub type CleanStart = bool;

pub type IsPing = bool;

pub type Tx = SessionTx; //futures::channel::mpsc::UnboundedSender<Message>;
pub type Rx = futures::channel::mpsc::UnboundedReceiver<Message>;

pub type DashSet<V> = dashmap::DashSet<V, ahash::RandomState>;
pub type DashMap<K, V> = dashmap::DashMap<K, V, ahash::RandomState>;
pub type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;
pub type QoS = ntex_mqtt::types::QoS;
pub type PublishReceiveTime = TimestampMillis;
pub type Subscriptions = Vec<(TopicFilter, SubscriptionOptions)>;
pub type TopicFilters = Vec<TopicFilter>;
pub type SubscriptionClientIds = Option<Vec<(ClientId, Option<(TopicFilter, SharedGroup)>)>>;
pub type SubscriptionIdentifier = NonZeroU32;

pub type HookSubscribeResult = Vec<Option<TopicFilter>>;
pub type HookUnsubscribeResult = Vec<Option<TopicFilter>>;

pub type MessageSender = Sender<(From, Publish)>;
pub type MessageQueue = Queue<(From, Publish)>;
pub type MessageQueueType = Arc<MessageQueue>;
pub type InflightType = Arc<RwLock<Inflight>>;

pub type ConnectInfoType = Arc<ConnectInfo>;
pub type FitterType = Arc<dyn Fitter>;

pub(crate) const UNDEFINED: &str = "undefined";

#[derive(Clone)]
pub struct SessionTx {
    tx: futures::channel::mpsc::UnboundedSender<Message>,
}

impl SessionTx {
    pub fn new(tx: futures::channel::mpsc::UnboundedSender<Message>) -> Self {
        Self { tx }
    }

    #[inline]
    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }

    #[inline]
    pub fn unbounded_send(
        &self,
        msg: Message,
    ) -> std::result::Result<(), futures::channel::mpsc::TrySendError<Message>> {
        match self.tx.unbounded_send(msg) {
            Ok(()) => {
                #[cfg(feature = "debug")]
                Runtime::instance().stats.debug_session_channels.inc();
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
pub enum ConnectInfo {
    V3(Id, ConnectV3),
    V5(Id, Box<ConnectV5>),
}

impl std::convert::From<Id> for ConnectInfo {
    fn from(id: Id) -> Self {
        ConnectInfo::V3(id, ConnectV3::default())
    }
}

impl ConnectInfo {
    #[inline]
    pub fn id(&self) -> &Id {
        match self {
            ConnectInfo::V3(id, _) => id,
            ConnectInfo::V5(id, _) => id,
        }
    }

    #[inline]
    pub fn client_id(&self) -> &ClientId {
        match self {
            ConnectInfo::V3(id, _) => &id.client_id,
            ConnectInfo::V5(id, _) => &id.client_id,
        }
    }

    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            ConnectInfo::V3(id, c) => {
                json!({
                    "node": id.node(),
                    "ipaddress": id.remote_addr,
                    "clientid": id.client_id,
                    "username": id.username_ref(),
                    "keepalive": c.keep_alive,
                    "proto_ver": c.protocol.level(),
                    "clean_session": c.clean_session,
                    "last_will": self.last_will().map(|lw|lw.to_json())
                })
            }
            ConnectInfo::V5(id, c) => {
                json!({
                    "node": id.node(),
                    "ipaddress": id.remote_addr,
                    "clientid": id.client_id,
                    "username": id.username_ref(),
                    "keepalive": c.keep_alive,
                    "proto_ver": ntex_mqtt::types::MQTT_LEVEL_5,
                    "clean_start": c.clean_start,
                    "last_will": self.last_will().map(|lw|lw.to_json()),

                    "session_expiry_interval_secs": c.session_expiry_interval_secs,
                    "auth_method": c.auth_method,
                    "auth_data": c.auth_data,
                    "request_problem_info": c.request_problem_info,
                    "request_response_info": c.request_response_info,
                    "receive_max": c.receive_max,
                    "topic_alias_max": c.topic_alias_max,
                    "user_properties": c.user_properties,
                    "max_packet_size": c.max_packet_size,
                })
            }
        }
    }

    #[inline]
    pub fn to_hook_body(&self) -> serde_json::Value {
        match self {
            ConnectInfo::V3(id, c) => {
                json!({
                    "node": id.node(),
                    "ipaddress": id.remote_addr,
                    "clientid": id.client_id,
                    "username": id.username_ref(),
                    "keepalive": c.keep_alive,
                    "proto_ver": c.protocol.level(),
                    "clean_session": c.clean_session,
                })
            }
            ConnectInfo::V5(id, c) => {
                json!({
                    "node": id.node(),
                    "ipaddress": id.remote_addr,
                    "clientid": id.client_id,
                    "username": id.username_ref(),
                    "keepalive": c.keep_alive,
                    "proto_ver": ntex_mqtt::types::MQTT_LEVEL_5,
                    "clean_start": c.clean_start,
                })
            }
        }
    }

    #[inline]
    pub fn last_will(&self) -> Option<LastWill> {
        match self {
            ConnectInfo::V3(_, conn_info) => conn_info.last_will.as_ref().map(LastWill::V3),
            ConnectInfo::V5(_, conn_info) => conn_info.last_will.as_ref().map(LastWill::V5),
        }
    }

    #[inline]
    pub fn keep_alive(&self) -> u16 {
        match self {
            ConnectInfo::V3(_, conn_info) => conn_info.keep_alive,
            ConnectInfo::V5(_, conn_info) => conn_info.keep_alive,
        }
    }

    #[inline]
    pub fn username(&self) -> Option<&UserName> {
        match self {
            ConnectInfo::V3(_, conn_info) => conn_info.username.as_ref(),
            ConnectInfo::V5(_, conn_info) => conn_info.username.as_ref(),
        }
    }

    #[inline]
    pub fn password(&self) -> Option<&Password> {
        match self {
            ConnectInfo::V3(_, conn_info) => conn_info.password.as_ref(),
            ConnectInfo::V5(_, conn_info) => conn_info.password.as_ref(),
        }
    }

    #[inline]
    pub fn clean_start(&self) -> bool {
        match self {
            ConnectInfo::V3(_, conn_info) => conn_info.clean_session,
            ConnectInfo::V5(_, conn_info) => conn_info.clean_start,
        }
    }

    #[inline]
    pub fn proto_ver(&self) -> u8 {
        match self {
            ConnectInfo::V3(_, conn_info) => conn_info.protocol.level(),
            ConnectInfo::V5(_, _) => MQTT_LEVEL_5,
        }
    }

    ///client max packet size, S(Max Limit) -> C
    #[inline]
    pub fn max_packet_size(&self) -> Option<NonZeroU32> {
        if let ConnectInfo::V5(_, connect) = self {
            connect.max_packet_size
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum Disconnect {
    V3,
    V5(DisconnectV5),
}

impl Disconnect {
    #[inline]
    pub fn reason_code(&self) -> Option<DisconnectReasonCode> {
        match self {
            Disconnect::V3 => None,
            Disconnect::V5(d) => Some(d.reason_code),
        }
    }

    #[inline]
    pub fn reason(&self) -> Reason {
        match self {
            Disconnect::V3 => Reason::ConnectDisconnect(None),
            Disconnect::V5(d) => Reason::ConnectDisconnect(d.reason_string.as_ref().cloned()),
        }
    }
}

pub trait QoSEx {
    fn value(&self) -> u8;
    fn less_value(&self, qos: QoS) -> QoS;
}

impl QoSEx for QoS {
    #[inline]
    fn value(&self) -> u8 {
        match self {
            QoS::AtMostOnce => 0,
            QoS::AtLeastOnce => 1,
            QoS::ExactlyOnce => 2,
        }
    }

    #[inline]
    fn less_value(&self, qos: QoS) -> QoS {
        if self.value() < qos.value() {
            *self
        } else {
            qos
        }
    }
}

pub type SubscribeAclResult = SubscribeReturn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishAclResult {
    Allow,
    Rejected(IsDisconnect),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthResult {
    Allow(Superuser),
    ///User is not found
    NotFound,
    BadUsernameOrPassword,
    NotAuthorized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageExpiryCheckResult {
    Expiry,
    Remaining(Option<NonZeroU32>),
}

impl MessageExpiryCheckResult {
    #[inline]
    pub fn is_expiry(&self) -> bool {
        matches!(self, Self::Expiry)
    }

    #[inline]
    pub fn message_expiry_interval(&self) -> Option<NonZeroU32> {
        match self {
            Self::Expiry => None,
            Self::Remaining(i) => *i,
        }
    }
}

//key is TopicFilter
pub type SharedSubRelations = HashMap<TopicFilter, Vec<(SharedGroup, NodeId, ClientId, QoS, IsOnline)>>;
//In other nodes
pub type OtherSubRelations = HashMap<NodeId, Vec<TopicFilter>>;
pub type ClearSubscriptions = bool;

pub type SharedGroupType = (SharedGroup, IsOnline, Vec<ClientId>);

pub type SubRelation = (
    TopicFilter,
    ClientId,
    SubscriptionOptions,
    Option<Vec<SubscriptionIdentifier>>,
    Option<SharedGroupType>,
);
pub type SubRelations = Vec<SubRelation>;
pub type SubRelationsMap = HashMap<NodeId, SubRelations>;

impl std::convert::From<SubscriptioRelationsCollector> for SubRelations {
    #[inline]
    fn from(collector: SubscriptioRelationsCollector) -> Self {
        let mut subs = collector.v3_rels;
        subs.extend(collector.v5_rels.into_iter().map(|(clientid, (topic_filter, opts, sub_ids, group))| {
            (topic_filter, clientid, opts, sub_ids, group)
        }));
        subs
    }
}

pub type SubscriptioRelationsCollectorMap = HashMap<NodeId, SubscriptioRelationsCollector>;

#[allow(clippy::type_complexity)]
#[derive(Debug, Default)]
pub struct SubscriptioRelationsCollector {
    v3_rels: Vec<SubRelation>,
    v5_rels: HashMap<
        ClientId,
        (TopicFilter, SubscriptionOptions, Option<Vec<SubscriptionIdentifier>>, Option<SharedGroupType>),
    >,
}

impl SubscriptioRelationsCollector {
    #[inline]
    pub fn add(
        &mut self,
        topic_filter: &TopicFilter,
        client_id: ClientId,
        opts: SubscriptionOptions,
        group: Option<SharedGroupType>,
    ) {
        if opts.is_v3() {
            self.v3_rels.push((topic_filter.clone(), client_id, opts, None, group));
        } else {
            //MQTT V5: Subscription Identifiers
            self.v5_rels
                .entry(client_id)
                .and_modify(|(_, _, sub_ids, _)| {
                    if let Some(sub_id) = opts.subscription_identifier() {
                        if let Some(sub_ids) = sub_ids {
                            sub_ids.push(sub_id)
                        } else {
                            *sub_ids = Some(vec![sub_id]);
                        }
                    }
                })
                .or_insert_with(|| {
                    let sub_ids = opts.subscription_identifier().map(|id| vec![id]);
                    (topic_filter.clone(), opts, sub_ids, group)
                });
        }
    }
}

#[inline]
pub fn parse_topic_filter(
    topic_filter: &ByteString,
    shared_subscription_supported: bool,
) -> Result<(TopicFilter, Option<SharedGroup>)> {
    let mut shared_group = None;
    let err = MqttError::TopicError("Illegal topic filter".into());
    //$share/abc/
    let topic = if shared_subscription_supported {
        let mut levels = topic_filter.splitn(3, '/').collect::<Vec<_>>();
        let is_share = levels.first().map(|f| *f == "$share").unwrap_or(false);
        if is_share {
            if levels.len() < 3 {
                return Err(err);
            }
            levels.remove(0);
            shared_group = Some(SharedGroup::from(levels.remove(0)));
            ByteString::from(levels.remove(0))
        } else {
            topic_filter.clone()
        }
    } else {
        topic_filter.clone()
    };
    if topic.is_empty() {
        return Err(err);
    }
    Ok((topic, shared_group))
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum SubscriptionOptions {
    V3(SubOptionsV3),
    V5(SubOptionsV5),
}

impl Default for SubscriptionOptions {
    fn default() -> Self {
        SubscriptionOptions::V3(SubOptionsV3 { qos: QoS::AtMostOnce, shared_group: None })
    }
}

impl SubscriptionOptions {
    #[inline]
    pub fn no_local(&self) -> Option<bool> {
        match self {
            SubscriptionOptions::V3(_) => None,
            SubscriptionOptions::V5(opts) => Some(opts.no_local),
        }
    }

    #[inline]
    pub fn retain_as_published(&self) -> Option<bool> {
        match self {
            SubscriptionOptions::V3(_) => None,
            SubscriptionOptions::V5(opts) => Some(opts.retain_as_published),
        }
    }

    #[inline]
    pub fn retain_handling(&self) -> Option<RetainHandling> {
        match self {
            SubscriptionOptions::V3(_) => None,
            SubscriptionOptions::V5(opts) => Some(opts.retain_handling),
        }
    }

    #[inline]
    pub fn subscription_identifier(&self) -> Option<NonZeroU32> {
        match self {
            SubscriptionOptions::V3(_) => None,
            SubscriptionOptions::V5(opts) => opts.id,
        }
    }

    #[inline]
    pub fn qos(&self) -> QoS {
        match self {
            SubscriptionOptions::V3(opts) => opts.qos,
            SubscriptionOptions::V5(opts) => opts.qos,
        }
    }

    #[inline]
    pub fn qos_value(&self) -> u8 {
        match self {
            SubscriptionOptions::V3(opts) => opts.qos.value(),
            SubscriptionOptions::V5(opts) => opts.qos.value(),
        }
    }

    #[inline]
    pub fn set_qos(&mut self, qos: QoS) {
        match self {
            SubscriptionOptions::V3(opts) => opts.qos = qos,
            SubscriptionOptions::V5(opts) => opts.qos = qos,
        }
    }

    #[inline]
    pub fn shared_group(&self) -> Option<&SharedGroup> {
        match self {
            SubscriptionOptions::V3(opts) => opts.shared_group.as_ref(),
            SubscriptionOptions::V5(opts) => opts.shared_group.as_ref(),
        }
    }

    #[inline]
    pub fn has_shared_group(&self) -> bool {
        match self {
            SubscriptionOptions::V3(opts) => opts.shared_group.is_some(),
            SubscriptionOptions::V5(opts) => opts.shared_group.is_some(),
        }
    }

    #[inline]
    pub fn is_v3(&self) -> bool {
        matches!(self, SubscriptionOptions::V3(_))
    }

    #[inline]
    pub fn is_v5(&self) -> bool {
        matches!(self, SubscriptionOptions::V5(_))
    }

    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            SubscriptionOptions::V3(opts) => opts.to_json(),
            SubscriptionOptions::V5(opts) => opts.to_json(),
        }
    }

    #[inline]
    pub fn deserialize_qos<'de, D>(deserializer: D) -> Result<QoS, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = u8::deserialize(deserializer)?;
        Ok(match v {
            0 => QoS::AtMostOnce,
            1 => QoS::AtLeastOnce,
            2 => QoS::ExactlyOnce,
            _ => return Err(de::Error::custom(format!("invalid QoS value, {}", v))),
        })
    }

    #[inline]
    pub fn serialize_qos<S>(qos: &QoS, s: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        qos.value().serialize(s)
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct SubOptionsV3 {
    #[serde(
        serialize_with = "SubscriptionOptions::serialize_qos",
        deserialize_with = "SubscriptionOptions::deserialize_qos"
    )]
    pub qos: QoS,
    pub shared_group: Option<SharedGroup>,
}

impl SubOptionsV3 {
    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
        let mut obj = json!({
            "qos": self.qos.value(),
        });
        if let Some(g) = &self.shared_group {
            if let Some(obj) = obj.as_object_mut() {
                obj.insert("group".into(), serde_json::Value::String(g.to_string()));
            }
        }
        obj
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct SubOptionsV5 {
    #[serde(
        serialize_with = "SubscriptionOptions::serialize_qos",
        deserialize_with = "SubscriptionOptions::deserialize_qos"
    )]
    pub qos: QoS,
    pub shared_group: Option<SharedGroup>,
    pub no_local: bool,
    pub retain_as_published: bool,
    #[serde(
        serialize_with = "SubOptionsV5::serialize_retain_handling",
        deserialize_with = "SubOptionsV5::deserialize_retain_handling"
    )]
    pub retain_handling: RetainHandling,
    //Subscription Identifier
    pub id: Option<SubscriptionIdentifier>,
}

impl SubOptionsV5 {
    #[inline]
    pub fn retain_handling_value(&self) -> u8 {
        match self.retain_handling {
            RetainHandling::AtSubscribe => 0u8,
            RetainHandling::AtSubscribeNew => 1u8,
            RetainHandling::NoAtSubscribe => 2u8,
        }
    }

    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
        let mut obj = json!({
            "qos": self.qos.value(),
            "no_local": self.no_local,
            "retain_as_published": self.retain_as_published,
            "retain_handling": self.retain_handling_value(),
        });
        if let Some(obj) = obj.as_object_mut() {
            if let Some(g) = &self.shared_group {
                obj.insert("group".into(), serde_json::Value::String(g.to_string()));
            }
            if let Some(id) = &self.id {
                obj.insert("id".into(), serde_json::Value::Number(serde_json::Number::from(id.get())));
            }
        }
        obj
    }

    #[inline]
    pub fn deserialize_retain_handling<'de, D>(deserializer: D) -> Result<RetainHandling, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = u8::deserialize(deserializer)?;
        Ok(match v {
            0 => RetainHandling::AtSubscribe,
            1 => RetainHandling::AtSubscribeNew,
            2 => RetainHandling::NoAtSubscribe,
            _ => return Err(de::Error::custom(format!("invalid RetainHandling value, {}", v))),
        })
    }

    #[inline]
    pub fn serialize_retain_handling<S>(rh: &RetainHandling, s: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let v = match rh {
            RetainHandling::AtSubscribe => 0u8,
            RetainHandling::AtSubscribeNew => 1u8,
            RetainHandling::NoAtSubscribe => 2u8,
        };
        v.serialize(s)
    }
}

impl std::convert::From<(QoS, Option<SharedGroup>)> for SubscriptionOptions {
    #[inline]
    fn from(opts: (QoS, Option<SharedGroup>)) -> Self {
        SubscriptionOptions::V3(SubOptionsV3 { qos: opts.0, shared_group: opts.1 })
    }
}

impl std::convert::From<(&SubscriptionOptionsV5, Option<SharedGroup>, Option<NonZeroU32>)>
    for SubscriptionOptions
{
    #[inline]
    fn from(opts: (&SubscriptionOptionsV5, Option<SharedGroup>, Option<NonZeroU32>)) -> Self {
        SubscriptionOptions::V5(SubOptionsV5 {
            qos: opts.0.qos,
            shared_group: opts.1,
            no_local: opts.0.no_local,
            retain_as_published: opts.0.retain_as_published,
            retain_handling: opts.0.retain_handling,
            id: opts.2,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Subscribe {
    pub topic_filter: TopicFilter,
    pub opts: SubscriptionOptions,
}

impl Subscribe {
    #[inline]
    pub fn from_v3(topic_filter: &ByteString, qos: QoS, shared_subscription_supported: bool) -> Result<Self> {
        let (topic_filter, shared_group) = parse_topic_filter(topic_filter, shared_subscription_supported)?;
        let opts = (qos, shared_group).into();
        Ok(Subscribe { topic_filter, opts })
    }

    #[inline]
    pub fn from_v5(
        topic_filter: &ByteString,
        opts: &SubscriptionOptionsV5,
        shared_subscription_supported: bool,
        sub_id: Option<NonZeroU32>,
    ) -> Result<Self> {
        let (topic_filter, shared_group) = parse_topic_filter(topic_filter, shared_subscription_supported)?;
        let opts = (opts, shared_group, sub_id).into();
        Ok(Subscribe { topic_filter, opts })
    }

    #[inline]
    pub fn is_shared(&self) -> bool {
        self.opts.has_shared_group()
    }
}

#[derive(Clone, Debug)]
pub struct SubscribeReturn {
    pub ack_reason: SubscribeAckReason,
    pub prev_opts: Option<SubscriptionOptions>,
}

impl SubscribeReturn {
    #[inline]
    pub fn new_success(qos: QoS, prev_opts: Option<SubscriptionOptions>) -> Self {
        let ack_reason = match qos {
            QoS::AtMostOnce => SubscribeAckReason::GrantedQos0,
            QoS::AtLeastOnce => SubscribeAckReason::GrantedQos1,
            QoS::ExactlyOnce => SubscribeAckReason::GrantedQos2,
        };
        Self { ack_reason, prev_opts }
    }

    #[inline]
    pub fn new_failure(ack_reason: SubscribeAckReason) -> Self {
        Self { ack_reason, prev_opts: None }
    }

    #[inline]
    pub fn success(&self) -> Option<QoS> {
        match self.ack_reason {
            SubscribeAckReason::GrantedQos0 => Some(QoS::AtMostOnce),
            SubscribeAckReason::GrantedQos1 => Some(QoS::AtLeastOnce),
            SubscribeAckReason::GrantedQos2 => Some(QoS::ExactlyOnce),
            _ => None,
        }
    }

    #[inline]
    pub fn failure(&self) -> bool {
        !matches!(
            self.ack_reason,
            SubscribeAckReason::GrantedQos0
                | SubscribeAckReason::GrantedQos1
                | SubscribeAckReason::GrantedQos2
        )
    }

    #[inline]
    pub fn into_inner(self) -> SubscribeAckReason {
        self.ack_reason
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct SubscribedV5 {
    /// Packet Identifier
    pub packet_id: NonZeroU16,
    /// Subscription Identifier
    pub id: Option<NonZeroU32>,
    pub user_properties: UserProperties,
    /// the list of Topic Filters and QoS to which the Client wants to subscribe.
    pub topic_filter: (ByteString, SubscriptionOptionsV5),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectAckReason {
    V3(ConnectAckReasonV3),
    V5(ConnectAckReasonV5),
}

impl ConnectAckReason {
    #[inline]
    pub fn success(&self) -> bool {
        matches!(
            *self,
            ConnectAckReason::V3(ConnectAckReasonV3::ConnectionAccepted)
                | ConnectAckReason::V5(ConnectAckReasonV5::Success)
        )
    }

    #[inline]
    pub fn not_authorized(&self) -> bool {
        matches!(
            *self,
            ConnectAckReason::V3(ConnectAckReasonV3::NotAuthorized)
                | ConnectAckReason::V3(ConnectAckReasonV3::BadUserNameOrPassword)
                | ConnectAckReason::V5(ConnectAckReasonV5::NotAuthorized)
                | ConnectAckReason::V5(ConnectAckReasonV5::BadUserNameOrPassword)
        )
    }

    #[inline]
    pub fn success_or_auth_error(&self) -> (bool, bool) {
        match *self {
            ConnectAckReason::V3(ConnectAckReasonV3::ConnectionAccepted)
            | ConnectAckReason::V5(ConnectAckReasonV5::Success) => (true, false),
            ConnectAckReason::V3(ConnectAckReasonV3::NotAuthorized)
            | ConnectAckReason::V3(ConnectAckReasonV3::BadUserNameOrPassword)
            | ConnectAckReason::V5(ConnectAckReasonV5::NotAuthorized)
            | ConnectAckReason::V5(ConnectAckReasonV5::BadUserNameOrPassword) => (false, true),
            _ => (false, false),
        }
    }

    #[inline]
    pub fn v3_error_ack<Io, St>(&self, handshake: v3::Handshake<Io>) -> HandshakeAckV3<Io, St> {
        match *self {
            ConnectAckReason::V3(ConnectAckReasonV3::UnacceptableProtocolVersion) => {
                handshake.service_unavailable()
            }
            ConnectAckReason::V3(ConnectAckReasonV3::IdentifierRejected) => handshake.identifier_rejected(),
            ConnectAckReason::V3(ConnectAckReasonV3::ServiceUnavailable) => handshake.service_unavailable(),
            ConnectAckReason::V3(ConnectAckReasonV3::BadUserNameOrPassword) => {
                handshake.bad_username_or_pwd()
            }
            ConnectAckReason::V3(ConnectAckReasonV3::NotAuthorized) => handshake.not_authorized(),
            ConnectAckReason::V3(ConnectAckReasonV3::Reserved) => handshake.service_unavailable(),
            _ => panic!("invalid value"),
        }
    }

    #[inline]
    pub fn v5_error_ack<Io, St>(&self, handshake: v5::Handshake<Io>) -> HandshakeAckV5<Io, St> {
        match *self {
            ConnectAckReason::V5(ack_reason) => handshake.failed(ack_reason),
            _ => panic!("invalid value"),
        }
    }

    #[inline]
    pub fn reason(&self) -> &'static str {
        match *self {
            ConnectAckReason::V3(r) => r.reason(),
            ConnectAckReason::V5(r) => r.reason(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Unsubscribe {
    pub topic_filter: TopicFilter,
    pub shared_group: Option<SharedGroup>,
}

impl Unsubscribe {
    #[inline]
    pub fn from(topic_filter: &ByteString, shared_subscription_supported: bool) -> Result<Self> {
        let (topic_filter, shared_group) = parse_topic_filter(topic_filter, shared_subscription_supported)?;
        Ok(Unsubscribe { topic_filter, shared_group })
    }

    #[inline]
    pub fn is_shared(&self) -> bool {
        self.shared_group.is_some()
    }
}

#[derive(Clone, Debug)]
pub enum UnsubscribeAck {
    V3,
    V5(UnsubscribeAckV5),
}

#[derive(Clone)]
pub enum LastWill<'a> {
    V3(&'a LastWillV3),
    V5(&'a LastWillV5),
}

impl<'a> fmt::Debug for LastWill<'a> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LastWill::V3(lw) => f
                .debug_struct("LastWill")
                .field("topic", &lw.topic)
                .field("retain", &lw.retain)
                .field("qos", &lw.qos.value())
                .field("message", &"<REDACTED>")
                .finish(),
            LastWill::V5(lw) => f
                .debug_struct("LastWill")
                .field("topic", &lw.topic)
                .field("retain", &lw.retain)
                .field("qos", &lw.qos.value())
                .field("message", &"<REDACTED>")
                .field("will_delay_interval_sec", &lw.will_delay_interval_sec)
                .field("correlation_data", &lw.correlation_data)
                .field("message_expiry_interval", &lw.message_expiry_interval)
                .field("content_type", &lw.content_type)
                .field("user_properties", &lw.user_properties)
                .field("is_utf8_payload", &lw.is_utf8_payload)
                .field("response_topic", &lw.response_topic)
                .finish(),
        }
    }
}

impl<'a> LastWill<'a> {
    #[inline]
    pub fn will_delay_interval(&self) -> Option<Duration> {
        match self {
            LastWill::V3(_) => None,
            LastWill::V5(lw) => lw.will_delay_interval_sec.map(|i| Duration::from_secs(i as u64)),
        }
    }

    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            LastWill::V3(lw) => {
                json!({
                    "qos": lw.qos.value(),
                    "retain": lw.retain,
                    "topic": lw.topic,
                    "message": BASE64_STANDARD.encode(lw.message.as_ref()),
                })
            }
            LastWill::V5(lw) => {
                json!({
                    "qos": lw.qos.value(),
                    "retain": lw.retain,
                    "topic": lw.topic,
                    "message": BASE64_STANDARD.encode(lw.message.as_ref()),

                    "will_delay_interval_sec": lw.will_delay_interval_sec,
                    "correlation_data": lw.correlation_data,
                    "message_expiry_interval": lw.message_expiry_interval,
                    "content_type": lw.content_type,
                    "user_properties": lw.user_properties,
                    "is_utf8_payload": lw.is_utf8_payload,
                    "response_topic": lw.response_topic,
                })
            }
        }
    }
}

impl<'a> Serialize for LastWill<'a> {
    #[inline]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            LastWill::V3(lw) => {
                let mut s = serializer.serialize_struct("LastWill", 4)?;
                s.serialize_field("qos", &lw.qos.value())?;
                s.serialize_field("retain", &lw.retain)?;
                s.serialize_field("topic", &lw.topic)?;
                s.serialize_field("message", &lw.message)?;
                s.end()
            }
            LastWill::V5(lw) => {
                let mut s = serializer.serialize_struct("LastWill", 11)?;
                s.serialize_field("qos", &lw.qos.value())?;
                s.serialize_field("retain", &lw.retain)?;
                s.serialize_field("topic", &lw.topic)?;
                s.serialize_field("message", &lw.message)?;

                s.serialize_field("will_delay_interval_sec", &lw.will_delay_interval_sec)?;
                s.serialize_field("correlation_data", &lw.correlation_data)?;
                s.serialize_field("message_expiry_interval", &lw.message_expiry_interval)?;
                s.serialize_field("content_type", &lw.content_type)?;
                s.serialize_field("user_properties", &lw.user_properties)?;
                s.serialize_field("is_utf8_payload", &lw.is_utf8_payload)?;
                s.serialize_field("response_topic", &lw.response_topic)?;

                s.end()
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum Sink {
    V3(MqttSinkV3),
    V5(MqttSinkV5),
}

impl Sink {
    #[inline]
    pub(crate) fn close(&self) {
        match self {
            Sink::V3(s) => {
                s.close();
            }
            Sink::V5(s) => s.close(),
        }
    }

    #[inline]
    pub(crate) async fn publish(
        &self,
        p: &Publish,
        message_expiry_interval: Option<NonZeroU32>,
        server_topic_aliases: Option<&Rc<ServerTopicAliases>>,
    ) -> Result<()> {
        let pkt = match self {
            Sink::V3(_) => p.into_v3(),
            Sink::V5(_) => p.into_v5(message_expiry_interval, server_topic_aliases).await,
        };
        self.send(pkt)
    }

    #[inline]
    pub(crate) fn send(&self, p: Packet) -> Result<()> {
        match self {
            Sink::V3(s) => {
                if let Packet::V3(p) = p {
                    s.send(p)?;
                }
            }
            Sink::V5(s) => {
                if s.is_open() {
                    if let Packet::V5(p) = p {
                        s.send(p)?;
                    }
                } else {
                    return Err(MqttError::from(SendPacketError::Disconnected));
                }
            }
        }
        Ok(())
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Packet {
    V3(PacketV3),
    V5(PacketV5),
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Default)]
pub struct PublishProperties {
    pub topic_alias: Option<NonZeroU16>,
    pub correlation_data: Option<Bytes>,
    pub message_expiry_interval: Option<NonZeroU32>,
    pub content_type: Option<ByteString>,
    pub user_properties: UserProperties,
    pub is_utf8_payload: Option<bool>,
    pub response_topic: Option<ByteString>,
    pub subscription_ids: Option<Vec<NonZeroU32>>,
}

impl std::convert::From<UserProperties> for PublishProperties {
    fn from(props: UserProperties) -> Self {
        PublishProperties {
            topic_alias: None,
            correlation_data: None,
            message_expiry_interval: None,
            content_type: None,
            user_properties: props,
            is_utf8_payload: None,
            response_topic: None,
            subscription_ids: None,
        }
    }
}

impl std::convert::From<PublishPropertiesV5> for PublishProperties {
    fn from(props: PublishPropertiesV5) -> Self {
        PublishProperties {
            topic_alias: props.topic_alias,
            correlation_data: props.correlation_data,
            message_expiry_interval: props.message_expiry_interval,
            content_type: props.content_type,
            user_properties: props.user_properties,
            is_utf8_payload: props.is_utf8_payload,
            response_topic: props.response_topic,
            subscription_ids: props.subscription_ids,
        }
    }
}

impl std::convert::From<PublishProperties> for PublishPropertiesV5 {
    fn from(props: PublishProperties) -> Self {
        PublishPropertiesV5 {
            topic_alias: props.topic_alias,
            correlation_data: props.correlation_data,
            message_expiry_interval: props.message_expiry_interval,
            content_type: props.content_type,
            user_properties: props.user_properties,
            is_utf8_payload: props.is_utf8_payload,
            response_topic: props.response_topic,
            subscription_ids: props.subscription_ids,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Publish {
    /// this might be re-delivery of an earlier attempt to send the Packet.
    pub dup: bool,
    pub retain: bool,
    /// the level of assurance for delivery of an Application Message.
    pub qos: QoS,
    /// the information channel to which payload data is published.
    pub topic: TopicName,
    /// only present in PUBLISH Packets where the QoS level is 1 or 2.
    pub packet_id: Option<NonZeroU16>,
    /// the Application Message that is being published.
    pub payload: Bytes,

    pub properties: PublishProperties,
    pub create_time: TimestampMillis,
}

impl<'a> std::convert::TryFrom<LastWill<'a>> for Publish {
    type Error = MqttError;

    #[inline]
    fn try_from(lw: LastWill<'a>) -> std::result::Result<Self, Self::Error> {
        let (retain, qos, topic, payload, props) = match lw {
            LastWill::V3(lw) => {
                let (topic, user_properties) = if let Some(pos) = lw.topic.find('?') {
                    let topic = lw.topic.clone();
                    let query = lw.topic.as_bytes().slice(pos + 1..lw.topic.len());
                    let user_props = url::form_urlencoded::parse(query.as_ref())
                        .into_owned()
                        .map(|(key, val)| (ByteString::from(key), ByteString::from(val)))
                        .collect::<UserProperties>();
                    (topic, user_props)
                } else {
                    let topic = lw.topic.clone();
                    (topic, UserProperties::default())
                };
                let props = PublishProperties { user_properties, ..Default::default() };
                (lw.retain, lw.qos, topic, lw.message.clone(), props)
            }
            LastWill::V5(lw) => {
                let topic = lw.topic.clone();
                let props = PublishProperties {
                    correlation_data: lw.correlation_data.clone(),
                    message_expiry_interval: lw.message_expiry_interval,
                    content_type: lw.content_type.clone(),
                    user_properties: lw.user_properties.clone(),
                    is_utf8_payload: lw.is_utf8_payload,
                    response_topic: lw.response_topic.clone(),
                    ..Default::default()
                };
                (lw.retain, lw.qos, topic, lw.message.clone(), props)
            }
        };

        Ok(Self {
            dup: false,
            retain,
            qos,
            topic,
            packet_id: None,
            payload,

            properties: props,
            create_time: chrono::Local::now().timestamp_millis(),
        })
    }
}

impl std::convert::From<&v3::Publish> for Publish {
    #[inline]
    fn from(p: &v3::Publish) -> Self {
        let query = p.query();
        let p_props = if !query.is_empty() {
            let user_props = url::form_urlencoded::parse(query.as_bytes())
                .into_owned()
                .map(|(key, val)| (ByteString::from(key), ByteString::from(val)))
                .collect::<UserProperties>();
            PublishProperties::from(user_props)
        } else {
            PublishProperties::default()
        };

        Self {
            dup: p.dup(),
            retain: p.retain(),
            qos: p.qos(),
            topic: TopicName::from(p.topic().path()),
            packet_id: p.id(),
            payload: p.take_payload(),

            properties: p_props,
            create_time: chrono::Local::now().timestamp_millis(),
        }
    }
}

impl std::convert::From<&v5::Publish> for Publish {
    #[inline]
    fn from(p: &v5::Publish) -> Self {
        Self {
            dup: p.dup(),
            retain: p.retain(),
            qos: p.qos(),
            topic: TopicName::from(p.topic().path()),
            packet_id: p.id(),
            payload: p.take_payload(),

            properties: PublishProperties::from(p.packet().properties.clone()),
            create_time: chrono::Local::now().timestamp_millis(),
        }
    }
}

impl Publish {
    #[inline]
    pub fn into_v3(&self) -> Packet {
        let p = v3::codec::Publish {
            dup: self.dup,
            retain: self.retain,
            qos: self.qos,
            topic: self.topic.clone(),
            packet_id: self.packet_id,
            payload: self.payload.clone(),
        };
        Packet::V3(v3::codec::Packet::Publish(p))
    }

    #[inline]
    pub async fn into_v5(
        &self,
        message_expiry_interval: Option<NonZeroU32>,
        server_topic_aliases: Option<&Rc<ServerTopicAliases>>,
    ) -> Packet {
        let (topic, alias) = {
            if let Some(server_topic_aliases) = server_topic_aliases {
                server_topic_aliases.get(self.topic.clone()).await
            } else {
                (Some(self.topic.clone()), None)
            }
        };
        log::debug!("topic: {:?}, alias: {:?}", topic, alias);
        let mut p = v5::codec::Publish {
            dup: self.dup,
            retain: self.retain,
            qos: self.qos,
            topic: topic.unwrap_or_default(),
            packet_id: self.packet_id,
            payload: self.payload.clone(),
            properties: self.properties.clone().into(),
        };
        p.properties.message_expiry_interval = message_expiry_interval;
        p.properties.topic_alias = alias;
        log::debug!("p.properties: {:?}", p.properties);
        Packet::V5(v5::codec::Packet::Publish(p))
    }

    #[inline]
    pub fn payload(&self) -> &Bytes {
        &self.payload
    }

    #[inline]
    pub fn retain(&self) -> bool {
        self.retain
    }

    #[inline]
    pub fn topic(&self) -> &TopicName {
        &self.topic
    }

    #[inline]
    pub fn topic_mut(&mut self) -> &mut TopicName {
        &mut self.topic
    }

    #[inline]
    pub fn dup(&self) -> bool {
        self.dup
    }

    #[inline]
    pub fn set_dup(&mut self, b: bool) {
        self.dup = b
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }

    #[inline]
    pub fn qos(&self) -> QoS {
        self.qos
    }

    #[inline]
    pub fn create_time(&self) -> TimestampMillis {
        self.create_time
    }

    #[inline]
    pub fn packet_id(&self) -> Option<PacketId> {
        self.packet_id.map(|id| id.get())
    }

    #[inline]
    pub fn packet_id_mut(&mut self) -> &mut Option<NonZeroU16> {
        &mut self.packet_id
    }

    #[inline]
    pub fn packet_id_is_none(&self) -> bool {
        self.packet_id.is_none()
    }

    #[inline]
    pub fn set_packet_id(&mut self, packet_id: PacketId) {
        self.packet_id = NonZeroU16::new(packet_id)
    }
}

#[derive(GetSize, Debug, Clone, Copy, Deserialize, Serialize)]
pub enum FromType {
    Custom,
    Admin,
    System,
    LastWill,
    Bridge,
}

impl std::fmt::Display for FromType {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let typ = match self {
            FromType::Custom => "custom",
            FromType::Admin => "admin",
            FromType::System => "system",
            FromType::LastWill => "lastwill",
            FromType::Bridge => "bridge",
        };
        write!(f, "{}", typ)
    }
}

#[derive(GetSize, Clone, Deserialize, Serialize)]
pub struct From {
    typ: FromType,
    pub id: Id,
}

impl From {
    #[inline]
    pub fn from_custom(id: Id) -> From {
        From { typ: FromType::Custom, id }
    }

    #[inline]
    pub fn from_admin(id: Id) -> From {
        From { typ: FromType::Admin, id }
    }

    #[inline]
    pub fn from_bridge(id: Id) -> From {
        From { typ: FromType::Bridge, id }
    }

    #[inline]
    pub fn from_system(id: Id) -> From {
        From { typ: FromType::System, id }
    }

    #[inline]
    pub fn from_lastwill(id: Id) -> From {
        From { typ: FromType::LastWill, id }
    }

    #[inline]
    pub fn typ(&self) -> FromType {
        self.typ
    }

    #[inline]
    pub fn is_system(&self) -> bool {
        matches!(self.typ, FromType::System)
    }

    #[inline]
    pub fn is_custom(&self) -> bool {
        matches!(self.typ, FromType::Custom)
    }

    #[inline]
    pub fn to_from_json(&self, json: serde_json::Value) -> serde_json::Value {
        let mut json = self.id.to_from_json(json);
        if let Some(obj) = json.as_object_mut() {
            obj.insert("from_type".into(), serde_json::Value::String(self.typ.to_string()));
        }
        json
    }
}

impl Deref for From {
    type Target = Id;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.id
    }
}

impl std::fmt::Debug for From {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}-{:?}", self.id, self.typ)
    }
}

//pub type From = Id;
pub type To = Id;

#[derive(Clone)]
pub struct Id(Arc<_Id>);

impl get_size::GetSize for Id {
    fn get_heap_size(&self) -> usize {
        self.0.get_heap_size()
    }
}

impl Id {
    #[inline]
    pub fn new(
        node_id: NodeId,
        local_addr: Option<SocketAddr>,
        remote_addr: Option<SocketAddr>,
        client_id: ClientId,
        username: Option<UserName>,
    ) -> Self {
        Self(Arc::new(_Id {
            node_id,
            local_addr,
            remote_addr,
            client_id,
            username,
            create_time: chrono::Local::now().timestamp_millis(),
        }))
    }

    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "node": self.node(),
            "ipaddress": self.remote_addr,
            "clientid": self.client_id,
            "username": self.username_ref(),
            "create_time": self.create_time,
        })
    }

    #[inline]
    pub fn to_from_json(&self, mut json: serde_json::Value) -> serde_json::Value {
        if let Some(obj) = json.as_object_mut() {
            obj.insert("from_node".into(), serde_json::Value::Number(serde_json::Number::from(self.node())));
            obj.insert(
                "from_ipaddress".into(),
                self.remote_addr
                    .map(|a| serde_json::Value::String(a.to_string()))
                    .unwrap_or(serde_json::Value::Null),
            );
            obj.insert("from_clientid".into(), serde_json::Value::String(self.client_id.to_string()));
            obj.insert("from_username".into(), serde_json::Value::String(self.username_ref().into()));
        }
        json
    }

    #[inline]
    pub fn to_to_json(&self, mut json: serde_json::Value) -> serde_json::Value {
        if let Some(obj) = json.as_object_mut() {
            obj.insert("node".into(), serde_json::Value::Number(serde_json::Number::from(self.node())));
            obj.insert(
                "ipaddress".into(),
                self.remote_addr
                    .map(|a| serde_json::Value::String(a.to_string()))
                    .unwrap_or(serde_json::Value::Null),
            );
            obj.insert("clientid".into(), serde_json::Value::String(self.client_id.to_string()));
            obj.insert("username".into(), serde_json::Value::String(self.username_ref().into()));
        }
        json
    }

    #[inline]
    pub fn from(node_id: NodeId, client_id: ClientId) -> Self {
        Self::new(node_id, None, None, client_id, None)
    }

    #[inline]
    pub fn node(&self) -> NodeId {
        self.node_id
    }

    #[inline]
    pub fn username(&self) -> UserName {
        self.username.clone().unwrap_or_else(|| UserName::from_static(UNDEFINED))
    }

    #[inline]
    pub fn username_ref(&self) -> &str {
        self.username.as_ref().map(<UserName as AsRef<str>>::as_ref).unwrap_or_else(|| UNDEFINED)
    }
}

impl Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}@{}/{}/{}/{}/{}",
            self.node_id,
            self.local_addr.map(|addr| addr.to_string()).unwrap_or_default(),
            self.remote_addr.map(|addr| addr.to_string()).unwrap_or_default(),
            self.client_id,
            self.username_ref(),
            self.create_time
        )
    }
}

impl std::fmt::Debug for Id {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self)
    }
}

impl PartialEq<Id> for Id {
    #[inline]
    fn eq(&self, o: &Id) -> bool {
        self.node_id == o.node_id
            && self.client_id == o.client_id
            && self.local_addr == o.local_addr
            && self.remote_addr == o.remote_addr
            && self.username == o.username
            && self.create_time == o.create_time
    }
}

impl Eq for Id {}

impl std::hash::Hash for Id {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.node_id.hash(state);
        self.local_addr.hash(state);
        self.remote_addr.hash(state);
        self.client_id.hash(state);
        self.username.hash(state);
        self.create_time.hash(state);
    }
}

impl Deref for Id {
    type Target = _Id;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Serialize for Id {
    #[inline]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        _Id::serialize(self.0.as_ref(), serializer)
    }
}

impl<'de> Deserialize<'de> for Id {
    #[inline]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Id(Arc::new(_Id::deserialize(deserializer)?)))
    }
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, GetSize, Deserialize, Serialize)]
pub struct _Id {
    pub node_id: NodeId,
    #[get_size(size_fn = get_option_addr_size_helper)]
    pub local_addr: Option<SocketAddr>,
    #[get_size(size_fn = get_option_addr_size_helper)]
    pub remote_addr: Option<SocketAddr>,
    #[get_size(size_fn = get_bytestring_size_helper)]
    pub client_id: ClientId,
    #[get_size(size_fn = get_option_bytestring_size_helper)]
    pub username: Option<UserName>,
    pub create_time: TimestampMillis,
}

fn get_bytestring_size_helper(s: &ByteString) -> usize {
    s.len()
}

fn get_option_bytestring_size_helper(s: &Option<ByteString>) -> usize {
    if let Some(s) = s {
        s.len()
    } else {
        0
    }
}

fn get_option_addr_size_helper(s: &Option<SocketAddr>) -> usize {
    if let Some(s) = s {
        match s {
            SocketAddr::V4(s) => size_of_val(s),
            SocketAddr::V6(s) => size_of_val(s),
        }
    } else {
        0
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Retain {
    pub msg_id: Option<MsgID>,
    pub from: From,
    pub publish: Publish,
}

pub type MsgID = usize;

#[derive(Debug, Clone, Deserialize, Serialize, GetSize)]
pub struct StoredMessage {
    pub msg_id: MsgID,
    pub from: From,
    #[get_size(size_fn = get_publish_size_helper)]
    pub publish: Publish,
    pub expiry_time_at: TimestampMillis,
}

fn get_bytes_size_helper(s: &Bytes) -> usize {
    s.len()
}

fn get_properties_size_helper(s: &PublishProperties) -> usize {
    s.content_type.as_ref().map(|ct| ct.len()).unwrap_or_default()
        + s.correlation_data.as_ref().map(|cd| cd.len()).unwrap_or_default()
        + s.user_properties.len() * size_of::<UserProperty>()
        + s.response_topic.as_ref().map(|rt| rt.len()).unwrap_or_default()
        + s.subscription_ids.as_ref().map(|si| si.len() * size_of::<NonZeroU32>()).unwrap_or_default()
}

fn get_publish_size_helper(p: &Publish) -> usize {
    // p.create_time.get_heap_size()
    p.packet_id.get_heap_size()
        // + p.dup.get_heap_size()
        + get_bytes_size_helper(&p.payload)
        // + p.retain.get_heap_size()
        + get_bytestring_size_helper(&p.topic)
        + size_of_val(&p.qos)
        + get_properties_size_helper(&p.properties)
}

impl StoredMessage {
    #[inline]
    pub fn is_expiry(&self) -> bool {
        self.expiry_time_at < timestamp_millis()
    }

    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(bincode::serialize(&self).map_err(|e| anyhow!(e))?)
    }

    #[inline]
    pub fn decode(data: &[u8]) -> Result<Self> {
        Ok(bincode::deserialize(data).map_err(|e| anyhow!(e))?)
    }
}

#[derive(Debug)]
pub enum Message {
    Forward(From, Publish),
    Kick(oneshot::Sender<()>, Id, CleanStart, IsAdmin),
    Disconnect(Disconnect),
    Closed(Reason),
    Keepalive(IsPing),
    Subscribe(Subscribe, oneshot::Sender<Result<SubscribeReturn>>),
    Unsubscribe(Unsubscribe, oneshot::Sender<Result<()>>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionStatus {
    pub id: Id,
    pub online: IsOnline,
    pub handshaking: bool,
}

#[derive(Deserialize, Serialize, Debug, Default, Clone)]
pub struct SubsSearchParams {
    #[serde(default)]
    pub _limit: usize,
    pub clientid: Option<String>,
    pub topic: Option<String>,
    //value is 0,1,2
    pub qos: Option<u8>,
    pub share: Option<SharedGroup>,
    pub _match_topic: Option<String>,
}

#[derive(Deserialize, Serialize, Debug, Default)]
pub struct SubsSearchResult {
    pub node_id: NodeId,
    pub clientid: ClientId,
    pub client_addr: Option<SocketAddr>,
    pub topic: TopicFilter,
    pub opts: SubscriptionOptions,
}

impl SubsSearchResult {
    #[inline]
    pub fn to_json(self) -> serde_json::Value {
        json!({
            "node_id": self.node_id,
            "clientid": self.clientid,
            "client_addr": self.client_addr,
            "topic": self.topic,
            "opts": self.opts.to_json(),
        })
    }
}

#[derive(Deserialize, Serialize, Debug, Default, PartialEq, Eq, Hash, Clone)]
pub struct Route {
    pub node_id: NodeId,
    pub topic: TopicFilter,
}

pub type SessionSubMap = HashMap<TopicFilter, SubscriptionOptions>;
#[derive(Clone)]
pub struct SessionSubs {
    subs: Arc<RwLock<SessionSubMap>>,
}

impl Deref for SessionSubs {
    type Target = Arc<RwLock<SessionSubMap>>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.subs
    }
}

impl Default for SessionSubs {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionSubs {
    #[inline]
    pub fn new() -> Self {
        Self::from(SessionSubMap::default())
    }

    #[inline]
    #[allow(clippy::mutable_key_type)]
    pub fn from(subs: SessionSubMap) -> Self {
        Self { subs: Arc::new(RwLock::new(subs)) }
    }

    #[inline]
    pub async fn _add(
        &self,
        topic_filter: TopicFilter,
        opts: SubscriptionOptions,
    ) -> Option<SubscriptionOptions> {
        let is_shared = opts.has_shared_group();

        let prev = {
            let mut subs = self.subs.write().await;
            let prev = subs.insert(topic_filter, opts);
            subs.shrink_to_fit();
            prev
        };

        if let Some(prev_opts) = &prev {
            match (prev_opts.has_shared_group(), is_shared) {
                (true, false) => {
                    Runtime::instance().stats.subscriptions_shared.dec();
                }
                (false, true) => {
                    Runtime::instance().stats.subscriptions_shared.inc();
                }
                (false, false) => {}
                (true, true) => {}
            }
        } else {
            Runtime::instance().stats.subscriptions.inc();
            if is_shared {
                Runtime::instance().stats.subscriptions_shared.inc();
            }
        }

        prev
    }

    #[inline]
    pub async fn _remove(&self, topic_filter: &str) -> Option<(TopicFilter, SubscriptionOptions)> {
        let removed = {
            let mut subs = self.subs.write().await;
            let removed = subs.remove_entry(topic_filter);
            subs.shrink_to_fit();
            removed
        };

        if let Some((_, opts)) = &removed {
            Runtime::instance().stats.subscriptions.dec();
            if opts.has_shared_group() {
                Runtime::instance().stats.subscriptions_shared.dec();
            }
        }

        removed
    }

    #[inline]
    pub async fn _drain(&self) -> Subscriptions {
        let topic_filters = self.subs.read().await.iter().map(|(key, _)| key.clone()).collect::<Vec<_>>();
        let mut subs = Vec::new();
        for tf in topic_filters {
            if let Some(sub) = self._remove(&tf).await {
                subs.push(sub);
            }
        }
        subs
    }

    #[inline]
    pub async fn _extend(&self, subs: Subscriptions) {
        for (topic_filter, opts) in subs {
            self._add(topic_filter, opts).await;
        }
    }

    #[inline]
    pub async fn _clear(&self) {
        {
            let subs = self.subs.read().await;
            for (_, opts) in subs.iter() {
                Runtime::instance().stats.subscriptions.dec();
                if opts.has_shared_group() {
                    Runtime::instance().stats.subscriptions_shared.dec();
                }
            }
        }
        let mut subs = self.subs.write().await;
        subs.clear();
        subs.shrink_to_fit();
    }

    #[inline]
    pub async fn len(&self) -> usize {
        self.subs.read().await.len()
    }

    #[inline]
    pub async fn shared_len(&self) -> usize {
        self.subs.read().await.iter().filter(|(_, opts)| opts.has_shared_group()).count()
    }

    #[inline]
    pub async fn is_empty(&self) -> bool {
        self.subs.read().await.is_empty()
    }

    #[inline]
    pub async fn to_topic_filters(&self) -> TopicFilters {
        self.subs.read().await.iter().map(|(key, _)| key.clone()).collect()
    }
}

pub struct ExtraData<K, T> {
    attrs: Arc<rust_box::std_ext::RwLock<HashMap<K, T>>>,
}

impl<K, T> Deref for ExtraData<K, T> {
    type Target = Arc<rust_box::std_ext::RwLock<HashMap<K, T>>>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.attrs
    }
}

impl<K, T> Serialize for ExtraData<K, T>
where
    K: serde::Serialize,
    T: serde::Serialize,
{
    #[inline]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.attrs.read().deref().serialize(serializer)
    }
}

impl<'de, K, T> Deserialize<'de> for ExtraData<K, T>
where
    K: Eq + Hash,
    K: serde::de::DeserializeOwned,
    T: serde::de::DeserializeOwned,
{
    #[inline]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = HashMap::deserialize(deserializer)?;
        Ok(Self { attrs: Arc::new(rust_box::std_ext::RwLock::new(v)) })
    }
}

impl<K, T> Default for ExtraData<K, T>
where
    K: Eq + Hash,
{
    fn default() -> Self {
        Self::new()
    }
}
impl<K, T> ExtraData<K, T>
where
    K: Eq + Hash,
{
    #[inline]
    pub fn new() -> Self {
        Self { attrs: Arc::new(rust_box::std_ext::RwLock::new(HashMap::default())) }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.attrs.read().len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.attrs.read().is_empty()
    }

    #[inline]
    pub fn clear(&self) {
        self.attrs.write().clear()
    }

    #[inline]
    pub fn insert(&self, key: K, value: T) {
        self.attrs.write().insert(key, value);
    }
}

pub struct ExtraAttrs {
    attrs: HashMap<String, Box<dyn Any + Sync + Send>>,
}

impl Default for ExtraAttrs {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtraAttrs {
    #[inline]
    pub fn new() -> Self {
        Self { attrs: HashMap::default() }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.attrs.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.attrs.is_empty()
    }

    #[inline]
    pub fn clear(&mut self) {
        self.attrs.clear()
    }

    #[inline]
    pub fn insert<T: Any + Sync + Send>(&mut self, key: String, value: T) {
        self.attrs.insert(key, Box::new(value));
    }

    #[inline]
    pub fn get<T: Any + Sync + Send>(&self, key: &str) -> Option<&T> {
        self.attrs.get(key).and_then(|v| v.downcast_ref::<T>())
    }

    #[inline]
    pub fn get_mut<T: Any + Sync + Send>(&mut self, key: &str) -> Option<&mut T> {
        self.attrs.get_mut(key).and_then(|v| v.downcast_mut::<T>())
    }

    #[inline]
    pub fn get_default_mut<T: Any + Sync + Send, F: Fn() -> T>(
        &mut self,
        key: String,
        def_fn: F,
    ) -> Option<&mut T> {
        self.attrs.entry(key).or_insert_with(|| Box::new(def_fn())).downcast_mut::<T>()
    }

    #[inline]
    pub fn serialize_key<S, T>(&self, key: &str, serializer: S) -> Result<S::Ok, S::Error>
    where
        T: Any + Sync + Send + serde::ser::Serialize,
        S: Serializer,
    {
        self.get::<T>(key).serialize(serializer)
    }
}

#[derive(Clone, Debug)]
pub struct TimedValue<V>(V, Option<Instant>);

impl<V> TimedValue<V> {
    pub fn new(value: V, timeout_duration: Option<Duration>) -> Self {
        TimedValue(value, timeout_duration.map(|t| Instant::now() + t))
    }

    pub fn value(&self) -> &V {
        &self.0
    }

    pub fn value_mut(&mut self) -> &mut V {
        &mut self.0
    }

    pub fn into_value(self) -> V {
        self.0
    }

    pub fn is_expired(&self) -> bool {
        self.1.map(|e| Instant::now() >= e).unwrap_or(false)
    }
}

impl<V> PartialEq for TimedValue<V>
where
    V: PartialEq,
{
    fn eq(&self, other: &TimedValue<V>) -> bool {
        self.value() == other.value()
    }
}

//impl<V> Eq for TimedValue<V> where V: Eq {}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct StateFlags: u8 {
        const Kicked = 0b00000001;
        const ByAdminKick = 0b00000010;
        const DisconnectReceived = 0b00000100;
        const CleanStart = 0b00001000;
        const Ping = 0b00010000;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct SessionStateFlags: u8 {
        const SessionPresent = 0b00000001;
        const Superuser = 0b00000010;
        const Connected = 0b00000100;
    }
}

impl Serialize for SessionStateFlags {
    #[inline]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let v: u8 = self.0 .0;
        v.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SessionStateFlags {
    #[inline]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = u8::deserialize(deserializer)?;
        Ok(SessionStateFlags::from_bits_retain(v))
    }
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub enum Reason {
    ConnectDisconnect(Option<ByteString>),
    ConnectReadWriteTimeout,
    ConnectReadWriteError,
    ConnectRemoteClose,
    ConnectKeepaliveTimeout,
    ConnectKicked(IsAdmin),
    SessionExpiration,
    SubscribeFailed(Option<ByteString>),
    UnsubscribeFailed(Option<ByteString>),
    SubscribeRefused,
    PublishRefused,
    MessageExpiration,
    MessageQueueFull,
    PublishFailed(ByteString),
    ProtocolError(ByteString),
    Error(ByteString),
    Reasons(Vec<Reason>),
    #[default]
    Unknown,
}

impl Reason {
    #[inline]
    pub fn from_static(r: &'static str) -> Self {
        Reason::Error(ByteString::from_static(r))
    }

    #[inline]
    pub fn is_kicked(&self, admin_opt: IsAdmin) -> bool {
        match self {
            Reason::ConnectKicked(_admin_opt) => *_admin_opt == admin_opt,
            _ => false,
        }
    }

    #[inline]
    pub fn is_kicked_by_admin(&self) -> bool {
        matches!(self, Reason::ConnectKicked(true))
    }
}

impl std::convert::From<&str> for Reason {
    #[inline]
    fn from(r: &str) -> Self {
        Reason::Error(ByteString::from(r))
    }
}

impl std::convert::From<String> for Reason {
    #[inline]
    fn from(r: String) -> Self {
        Reason::Error(ByteString::from(r))
    }
}

impl std::convert::From<MqttError> for Reason {
    #[inline]
    fn from(e: MqttError) -> Self {
        match e {
            MqttError::Reason(r) => r,
            MqttError::SendPacketError(_) | MqttError::IoError(_) => Reason::ConnectReadWriteError,
            MqttError::Timeout(_) => Reason::ConnectReadWriteTimeout,
            MqttError::None => Reason::Unknown,
            _ => Reason::Error(ByteString::from(e.to_string())),
        }
    }
}

impl Display for Reason {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let r = match self {
            Reason::ConnectDisconnect(r) => {
                //Disconnect message received
                match r {
                    Some(r) => return write!(f, "Disconnect({})", r),
                    None => "Disconnect",
                }
            }
            Reason::ConnectReadWriteTimeout => {
                "ReadWriteTimeout" //read/write timeout
            }
            Reason::ConnectReadWriteError => {
                "ReadWriteError" //read/write error
            }
            Reason::ConnectRemoteClose => {
                "RemoteClose" //"connection close by remote client"
            }
            Reason::ConnectKeepaliveTimeout => {
                "KeepaliveTimeout" //keepalive timeout
            }
            Reason::ConnectKicked(admin_opt) => {
                if *admin_opt {
                    "ByAdminKick" //kicked by administrator
                } else {
                    "Kicked" //kicked
                }
            }
            Reason::SessionExpiration => {
                "SessionExpiration" //session expiration
            }
            Reason::SubscribeFailed(r) => {
                //subscribe failed
                match r {
                    Some(r) => return write!(f, "SubscribeFailed({})", r),
                    None => "SubscribeFailed",
                }
            }
            Reason::UnsubscribeFailed(r) => {
                //unsubscribe failed
                match r {
                    Some(r) => return write!(f, "UnsubscribeFailed({})", r),
                    None => "UnsubscribeFailed",
                }
            }
            Reason::SubscribeRefused => {
                "SubscribeRefused" //subscribe refused
            }
            Reason::PublishRefused => {
                "PublishRefused" //publish refused
            }
            Reason::MessageExpiration => {
                "MessageExpiration" //message expiration
            }
            Reason::MessageQueueFull => {
                "MessageQueueFull" //message deliver queue is full
            }
            Reason::PublishFailed(r) => return write!(f, "PublishFailed({})", r),
            Reason::Error(r) => r,
            Reason::ProtocolError(r) => return write!(f, "ProtocolError({})", r),
            Reason::Reasons(reasons) => match reasons.len() {
                0 => "",
                1 => return write!(f, "{}", reasons.first().map(|r| r.to_string()).unwrap_or_default()),
                _ => return write!(f, "{}", reasons.iter().map(|r| r.to_string()).join(",")),
            },
            Reason::Unknown => {
                "Unknown" //unknown
            }
        };
        write!(f, "{}", r)
    }
}

#[derive(Debug)]
pub struct ServerTopicAliases {
    max_topic_aliases: usize,
    aliases: RwLock<HashMap<TopicName, NonZeroU16>>,
}

impl ServerTopicAliases {
    #[inline]
    pub fn new(max_topic_aliases: usize) -> Self {
        ServerTopicAliases { max_topic_aliases, aliases: RwLock::new(HashMap::default()) }
    }

    #[inline]
    pub async fn get(&self, topic: TopicName) -> (Option<TopicName>, Option<NonZeroU16>) {
        if self.max_topic_aliases == 0 {
            return (Some(topic), None);
        }
        let alias = {
            let aliases = self.aliases.read().await;
            if let Some(alias) = aliases.get(&topic) {
                return (None, Some(*alias));
            }
            let len = aliases.len();
            if len >= self.max_topic_aliases {
                return (Some(topic), None);
            }

            match NonZeroU16::try_from((len + 1) as u16) {
                Ok(alias) => alias,
                Err(_) => {
                    unreachable!()
                }
            }
        };
        self.aliases.write().await.insert(topic.clone(), alias);
        (Some(topic), Some(alias))
    }
}

#[derive(Debug)]
pub struct ClientTopicAliases {
    max_topic_aliases: usize,
    aliases: RwLock<HashMap<NonZeroU16, TopicName>>,
}

impl ClientTopicAliases {
    #[inline]
    pub fn new(max_topic_aliases: usize) -> Self {
        ClientTopicAliases { max_topic_aliases, aliases: RwLock::new(HashMap::default()) }
    }

    #[inline]
    pub async fn set_and_get(&self, alias: Option<NonZeroU16>, topic: TopicName) -> Result<TopicName> {
        match (alias, topic.len()) {
            (Some(alias), 0) => {
                self.aliases.read().await.get(&alias).ok_or_else(|| {
                    MqttError::PublishAckReason(
                        PublishAckReason::ImplementationSpecificError,
                        ByteString::from(
                            "implementation specific error, the ‘topic‘ associated with the ‘alias‘ was not found",
                        ),
                    )
                }).cloned()
            }
            (Some(alias), _) => {
                let mut aliases = self.aliases.write().await;
                let len = aliases.len();
                if let Some(topic_mut) = aliases.get_mut(&alias) {
                    *topic_mut = topic.clone()
                }else{
                    if len >= self.max_topic_aliases {
                        return Err(MqttError::PublishAckReason(
                            PublishAckReason::ImplementationSpecificError,
                            ByteString::from(
                                format!("implementation specific error, the number of topic aliases exceeds the limit ({})", self.max_topic_aliases),
                            ),
                        ))
                    }
                    aliases.insert(alias, topic.clone());
                }
                Ok(topic)
            }
            (None, 0) => Err(MqttError::PublishAckReason(
                PublishAckReason::ImplementationSpecificError,
                ByteString::from("implementation specific error, ‘alias’ and ‘topic’ are both empty"),
            )),
            (None, _) => Ok(topic),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct DisconnectInfo {
    pub disconnected_at: TimestampMillis,
    pub reasons: Vec<Reason>,
    pub mqtt_disconnect: Option<Disconnect>, //MQTT Disconnect
}

impl DisconnectInfo {
    #[inline]
    pub fn new(disconnected_at: TimestampMillis) -> Self {
        Self { disconnected_at, reasons: Vec::new(), mqtt_disconnect: None }
    }

    #[inline]
    pub fn is_disconnected(&self) -> bool {
        self.disconnected_at != 0
    }
}

#[inline]
pub fn topic_size(topic: &Topic) -> usize {
    topic
        .iter()
        .map(|l| {
            let data_len = match l {
                TopicLevel::Normal(s) => s.len(),
                TopicLevel::Metadata(s) => s.len(),
                _ => 0,
            };
            size_of::<TopicLevel>() + data_len
        })
        .sum::<usize>()
}

#[inline]
pub fn timestamp() -> Duration {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_else(|_| {
        let now = chrono::Local::now();
        Duration::new(now.timestamp() as u64, now.timestamp_subsec_nanos())
    })
}

#[inline]
pub fn timestamp_secs() -> Timestamp {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|t| t.as_secs() as i64)
        .unwrap_or_else(|_| chrono::Local::now().timestamp())
}

#[inline]
pub fn timestamp_millis() -> TimestampMillis {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|t| t.as_millis() as i64)
        .unwrap_or_else(|_| chrono::Local::now().timestamp_millis())
}

#[inline]
pub fn format_timestamp(t: Timestamp) -> String {
    if t <= 0 {
        "".into()
    } else {
        use chrono::TimeZone;
        if let chrono::LocalResult::Single(t) = chrono::Local.timestamp_opt(t, 0) {
            t.format("%Y-%m-%d %H:%M:%S").to_string()
        } else {
            "".into()
        }
    }
}

#[inline]
pub fn format_timestamp_millis(t: TimestampMillis) -> String {
    if t <= 0 {
        "".into()
    } else {
        use chrono::TimeZone;
        if let chrono::LocalResult::Single(t) = chrono::Local.timestamp_millis_opt(t) {
            t.format("%Y-%m-%d %H:%M:%S%.3f").to_string()
        } else {
            "".into()
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum StatsMergeMode {
    None,    // Represents no merging;
    Sum,     // Represents summing the data;
    Average, // Represents averaging the data;
    Max,     // Represents taking the maximum value of the data;
    Min,     // Represents taking the minimum value of the data;
}

#[test]
fn test_reason() {
    assert!(Reason::ConnectKicked(false).is_kicked(false));
    assert!(!Reason::ConnectKicked(false).is_kicked(true));
    assert!(Reason::ConnectKicked(true).is_kicked(true));
    assert!(!Reason::ConnectKicked(true).is_kicked(false));
    assert!(Reason::ConnectKicked(true).is_kicked_by_admin());
    assert!(!Reason::ConnectKicked(false).is_kicked_by_admin());
    assert!(!Reason::ConnectDisconnect(None).is_kicked(false));
    assert!(!Reason::ConnectDisconnect(None).is_kicked_by_admin());

    let reasons = Reason::Reasons(vec![
        Reason::PublishRefused,
        Reason::ConnectKicked(false),
        Reason::MessageExpiration,
    ]);
    assert_eq!(reasons.to_string(), "PublishRefused,Kicked,MessageExpiration");
}
