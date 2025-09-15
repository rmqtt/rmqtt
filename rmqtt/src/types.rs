//! Some commonly used type definitions

use std::convert::From as _f;
use std::fmt;
use std::fmt::Display;
use std::hash::Hash;
use std::mem::{size_of, size_of_val};
use std::net::SocketAddr;
use std::num::{NonZeroU16, NonZeroU32};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use base64::prelude::{Engine, BASE64_STANDARD};
use bitflags::bitflags;
use bytes::Bytes;
use bytestring::ByteString;
use futures::StreamExt;
use get_size::GetSize;
use itertools::Itertools;
use serde::de::{self, Deserializer};
use serde::ser::{SerializeStruct, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{oneshot, RwLock};

use crate::acl::AuthInfo;
use crate::codec::types::MQTT_LEVEL_5;
use crate::codec::v3::{
    Connect as ConnectV3, ConnectAckReason as ConnectAckReasonV3, LastWill as LastWillV3,
};
use crate::codec::v5::{
    Connect as ConnectV5, ConnectAckReason as ConnectAckReasonV5, DisconnectReasonCode,
    LastWill as LastWillV5, PublishAck, PublishAck2, PublishAck2Reason, PublishAckReason, PublishProperties,
    RetainHandling, SubscribeAckReason, SubscriptionOptions as SubscriptionOptionsV5, ToReasonCode,
    UserProperties, UserProperty,
};
use crate::fitter::Fitter;
use crate::net::MqttError;
use crate::net::{v3, v5, Builder};
use crate::queue::{Queue, Sender};
use crate::utils::{self, timestamp_millis};
use crate::{codec, Error, Result};

use crate::context::ServerContext;
use crate::inflight::{OutInflight, OutInflightMessage};
use crate::session::OfflineInfo;
use crate::topic::Level;

pub use crate::codec::types::Publish as CodecPublish;

pub type Port = u16;
pub type NodeId = utils::NodeId;
pub type NodeName = String;
pub type RemoteSocketAddr = SocketAddr;
pub type LocalSocketAddr = SocketAddr;
pub type Addr = utils::Addr;
pub type ClientId = ByteString;
pub type UserName = ByteString;
pub type Superuser = bool;
pub type Password = bytes::Bytes;
pub type PacketId = u16;
///topic name or topic filter
pub type TopicName = ByteString;
pub type Topic = crate::topic::Topic;
///topic filter
pub type TopicFilter = ByteString;
pub type SharedGroup = ByteString;
pub type LimitSubsCount = Option<usize>;
pub type IsDisconnect = bool;
pub type MessageExpiry = bool;
pub type TimestampMillis = utils::TimestampMillis;
pub type Timestamp = utils::Timestamp;
pub type IsOnline = bool;
pub type IsAdmin = bool;
pub type LimiterName = u16;
pub type CleanStart = bool;
pub type AssignedClientId = bool;
pub type IsPing = bool;

pub type Tx = SessionTx;
pub type Rx = futures::channel::mpsc::UnboundedReceiver<Message>;

pub type DashSet<V> = dashmap::DashSet<V, ahash::RandomState>;
pub type DashMap<K, V> = dashmap::DashMap<K, V, ahash::RandomState>;
pub type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;
pub type QoS = rmqtt_codec::types::QoS;
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
pub type OutInflightType = Arc<RwLock<OutInflight>>; //@TODO 考虑去掉 RwLock

pub type ConnectInfoType = Arc<ConnectInfo>;
pub type FitterType = Arc<dyn Fitter>;
pub type ListenerConfig = Arc<Builder>;
pub type ListenerId = u16;

pub(crate) const UNDEFINED: &str = "undefined";

#[derive(Clone)]
pub struct SessionTx {
    #[cfg(feature = "debug")]
    scx: ServerContext,
    tx: futures::channel::mpsc::UnboundedSender<Message>,
}

impl SessionTx {
    pub fn new(
        tx: futures::channel::mpsc::UnboundedSender<Message>,
        #[cfg(feature = "debug")] scx: ServerContext,
    ) -> Self {
        Self {
            tx,
            #[cfg(feature = "debug")]
            scx,
        }
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
                self.scx.stats.debug_session_channels.inc();
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
pub enum ConnectInfo {
    V3(Id, Box<ConnectV3>),
    V5(Id, Box<ConnectV5>),
}

impl std::convert::From<Id> for ConnectInfo {
    fn from(id: Id) -> Self {
        ConnectInfo::V3(id, Box::default())
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
                    "proto_ver": MQTT_LEVEL_5,
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
                    "proto_ver": MQTT_LEVEL_5,
                    "clean_start": c.clean_start,
                })
            }
        }
    }

    #[inline]
    pub fn last_will(&self) -> Option<LastWill<'_>> {
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
    pub fn ipaddress(&self) -> Option<SocketAddr> {
        match self {
            ConnectInfo::V3(id, _) => id.remote_addr,
            ConnectInfo::V5(id, _) => id.remote_addr,
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

    #[inline]
    pub fn auth_method(&self) -> Option<&ByteString> {
        if let ConnectInfo::V5(_, connect) = self {
            connect.auth_method.as_ref()
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub enum Disconnect {
    V3,
    V5(codec::v5::Disconnect),
    Other(ByteString),
}

impl Disconnect {
    #[inline]
    pub fn reason_code(&self) -> Option<DisconnectReasonCode> {
        match self {
            Disconnect::V3 => None,
            Disconnect::V5(d) => Some(d.reason_code),
            Disconnect::Other(_) => None,
        }
    }

    #[inline]
    pub fn reason(&self) -> Option<&ByteString> {
        match self {
            Disconnect::V3 => None,
            Disconnect::V5(d) => d.reason_string.as_ref(),
            Disconnect::Other(r) => Some(r),
        }
    }
}

pub type SubscribeAclResult = SubscribeReturn;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishAclResult {
    Allow,
    Rejected(IsDisconnect),
}

#[derive(Debug, Clone)]
pub enum AuthResult {
    Allow(Superuser, Option<AuthInfo>),
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

pub type AllRelationsMap = DashMap<TopicFilter, HashMap<ClientId, (Id, SubscriptionOptions)>>;

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
    shared_subscription: bool,
    limit_subscription: bool,
) -> Result<(TopicFilter, Option<SharedGroup>, LimitSubsCount)> {
    let invalid_filter = || anyhow!(format!("Illegal topic filter, {:?}", topic_filter));
    let (topic, shared_group, limit_subs) = if shared_subscription || limit_subscription {
        let levels = topic_filter.splitn(3, '/').collect::<Vec<_>>();
        match (levels.first(), levels.get(1), levels.get(2)) {
            (Some(&"$share"), group, tf) => match (shared_subscription, group, tf) {
                (true, Some(group), Some(tf)) => {
                    let tf = TopicFilter::from(*tf);
                    (tf, Some(SharedGroup::from(*group)), None)
                }
                (true, _, _) => {
                    return Err(invalid_filter());
                }
                (false, _, _) => {
                    return Err(anyhow!(format!("Shared subscription is not enabled, {:?}", topic_filter)));
                }
            },
            (Some(&"$limit"), limit, tf) => match (limit_subscription, limit, tf) {
                (true, Some(limit), Some(tf)) => {
                    let tf = TopicFilter::from(*tf);
                    let limit = limit.parse::<usize>().map_err(|_| invalid_filter())?;
                    (tf, None, Some(limit))
                }
                (true, _, _) => {
                    return Err(invalid_filter());
                }
                (false, _, _) => {
                    return Err(anyhow!(format!("Limit subscription is not enabled, {:?}", topic_filter)));
                }
            },
            (Some(&"$exclusive"), _, _) => {
                if limit_subscription {
                    let tf = TopicFilter::from(topic_filter.trim_start_matches("$exclusive/"));
                    (tf, None, Some(1))
                } else {
                    return Err(anyhow!(format!("Limit subscription is not enabled, {:?}", topic_filter)));
                }
            }
            _ => (topic_filter.clone(), None, None),
        }
    } else {
        (topic_filter.clone(), None, None)
    };
    if topic.is_empty() {
        return Err(invalid_filter());
    }
    Ok((topic, shared_group, limit_subs))
}

#[derive(Debug, PartialEq, Eq, Clone, Serialize, Deserialize)]
pub enum SubscriptionOptions {
    V3(SubOptionsV3),
    V5(SubOptionsV5),
}

impl Default for SubscriptionOptions {
    fn default() -> Self {
        SubscriptionOptions::V3(SubOptionsV3 {
            qos: QoS::AtMostOnce,
            #[cfg(feature = "shared-subscription")]
            shared_group: None,
            #[cfg(feature = "limit-subscription")]
            limit_subs: None,
        })
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
        #[cfg(feature = "shared-subscription")]
        match self {
            SubscriptionOptions::V3(opts) => opts.shared_group.as_ref(),
            SubscriptionOptions::V5(opts) => opts.shared_group.as_ref(),
        }
        #[cfg(not(feature = "shared-subscription"))]
        None
    }

    #[inline]
    #[cfg(feature = "shared-subscription")]
    pub fn has_shared_group(&self) -> bool {
        match self {
            SubscriptionOptions::V3(opts) => opts.shared_group.is_some(),
            SubscriptionOptions::V5(opts) => opts.shared_group.is_some(),
        }
    }

    #[inline]
    pub fn limit_subs(&self) -> Option<usize> {
        #[cfg(feature = "limit-subscription")]
        match self {
            SubscriptionOptions::V3(opts) => opts.limit_subs,
            SubscriptionOptions::V5(opts) => opts.limit_subs,
        }
        #[cfg(not(feature = "limit-subscription"))]
        None
    }

    #[inline]
    #[cfg(feature = "limit-subscription")]
    pub fn has_limit_subs(&self) -> bool {
        match self {
            SubscriptionOptions::V3(opts) => opts.limit_subs.is_some(),
            SubscriptionOptions::V5(opts) => opts.limit_subs.is_some(),
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
    pub fn deserialize_qos<'de, D>(deserializer: D) -> std::result::Result<QoS, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = u8::deserialize(deserializer)?;
        Ok(match v {
            0 => QoS::AtMostOnce,
            1 => QoS::AtLeastOnce,
            2 => QoS::ExactlyOnce,
            _ => return Err(de::Error::custom(format!("invalid QoS value, {v}"))),
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
    #[cfg(feature = "shared-subscription")]
    pub shared_group: Option<SharedGroup>,
    #[cfg(feature = "limit-subscription")]
    pub limit_subs: LimitSubsCount,
}

impl SubOptionsV3 {
    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
        #[allow(unused_mut)]
        let mut obj = json!({
            "qos": self.qos.value(),
        });
        #[cfg(any(feature = "limit-subscription", feature = "shared-subscription"))]
        if let Some(obj) = obj.as_object_mut() {
            #[cfg(feature = "shared-subscription")]
            if let Some(g) = &self.shared_group {
                obj.insert("group".into(), serde_json::Value::String(g.to_string()));
            }
            #[cfg(feature = "limit-subscription")]
            if let Some(limit_subs) = &self.limit_subs {
                obj.insert("limit_subs".into(), serde_json::Value::from(*limit_subs));
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
    #[cfg(feature = "shared-subscription")]
    pub shared_group: Option<SharedGroup>,
    #[cfg(feature = "limit-subscription")]
    pub limit_subs: LimitSubsCount,
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
            #[cfg(feature = "shared-subscription")]
            if let Some(g) = &self.shared_group {
                obj.insert("group".into(), serde_json::Value::String(g.to_string()));
            }
            #[cfg(feature = "limit-subscription")]
            if let Some(limit_subs) = &self.limit_subs {
                obj.insert("limit_subs".into(), serde_json::Value::from(*limit_subs));
            }
            if let Some(id) = &self.id {
                obj.insert("id".into(), serde_json::Value::Number(serde_json::Number::from(id.get())));
            }
        }
        obj
    }

    #[inline]
    pub fn deserialize_retain_handling<'de, D>(
        deserializer: D,
    ) -> std::result::Result<RetainHandling, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = u8::deserialize(deserializer)?;
        Ok(match v {
            0 => RetainHandling::AtSubscribe,
            1 => RetainHandling::AtSubscribeNew,
            2 => RetainHandling::NoAtSubscribe,
            _ => return Err(de::Error::custom(format!("invalid RetainHandling value, {v}"))),
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

impl std::convert::From<(QoS, Option<SharedGroup>, LimitSubsCount)> for SubscriptionOptions {
    #[inline]
    fn from(opts: (QoS, Option<SharedGroup>, LimitSubsCount)) -> Self {
        SubscriptionOptions::V3(SubOptionsV3 {
            qos: opts.0,
            #[cfg(feature = "shared-subscription")]
            shared_group: opts.1,
            #[cfg(feature = "limit-subscription")]
            limit_subs: opts.2,
        })
    }
}

impl std::convert::From<(&SubscriptionOptionsV5, Option<SharedGroup>, LimitSubsCount, Option<NonZeroU32>)>
    for SubscriptionOptions
{
    #[inline]
    fn from(opts: (&SubscriptionOptionsV5, Option<SharedGroup>, LimitSubsCount, Option<NonZeroU32>)) -> Self {
        SubscriptionOptions::V5(SubOptionsV5 {
            qos: opts.0.qos,
            #[cfg(feature = "shared-subscription")]
            shared_group: opts.1,
            #[cfg(feature = "limit-subscription")]
            limit_subs: opts.2,
            no_local: opts.0.no_local,
            retain_as_published: opts.0.retain_as_published,
            retain_handling: opts.0.retain_handling,
            id: opts.3,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Subscribe {
    pub topic_filter: TopicFilter,
    pub opts: SubscriptionOptions,
}

impl Subscribe {
    #[inline]
    pub fn from_v3(
        topic_filter: &ByteString,
        qos: QoS,
        shared_subscription: bool,
        limit_subscription: bool,
    ) -> Result<Self> {
        let (topic_filter, shared_group, limit_subs) =
            parse_topic_filter(topic_filter, shared_subscription, limit_subscription)?;
        let opts = (qos, shared_group, limit_subs).into();
        Ok(Subscribe { topic_filter, opts })
    }

    #[inline]
    pub fn from_v5(
        topic_filter: &ByteString,
        opts: &SubscriptionOptionsV5,
        shared_subscription: bool,
        limit_subscription: bool,
        sub_id: Option<NonZeroU32>,
    ) -> Result<Self> {
        let (topic_filter, shared_group, limit_subs) =
            parse_topic_filter(topic_filter, shared_subscription, limit_subscription)?;
        let opts = (opts, shared_group, limit_subs, sub_id).into();
        Ok(Subscribe { topic_filter, opts })
    }

    #[inline]
    #[cfg(feature = "shared-subscription")]
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
//
// #[derive(Debug, PartialEq, Eq, Clone)]
// pub struct SubscribedV5 {
//     /// Packet Identifier
//     pub packet_id: NonZeroU16,
//     /// Subscription Identifier
//     pub id: Option<NonZeroU32>,
//     pub user_properties: UserProperties,
//     /// the list of Topic Filters and QoS to which the Client wants to subscribe.
//     pub topic_filter: (ByteString, SubscriptionOptionsV5),
// }

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
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
    pub fn from(
        topic_filter: &ByteString,
        shared_subscription: bool,
        limit_subscription: bool,
    ) -> Result<Self> {
        let (topic_filter, shared_group, _) =
            parse_topic_filter(topic_filter, shared_subscription, limit_subscription)?;
        Ok(Unsubscribe { topic_filter, shared_group })
    }

    #[inline]
    pub fn is_shared(&self) -> bool {
        self.shared_group.is_some()
    }
}

// #[derive(Clone, Debug)]
// pub enum UnsubscribeAck {
//     V3,
//     V5(UnsubscribeAckV5),
// }

#[derive(Clone)]
pub enum LastWill<'a> {
    V3(&'a LastWillV3),
    V5(&'a LastWillV5),
}

impl fmt::Debug for LastWill<'_> {
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

impl LastWill<'_> {
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

impl Serialize for LastWill<'_> {
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

#[derive(Serialize, Deserialize, Clone, Debug)]
/// High-level Publish structure used in the broker.
///
/// This wraps a low-level `CodecPublish` packet and adds
/// extra metadata such as the target client identifier.
pub struct Publish {
    /// The raw publish packet decoded from the codec layer.
    pub inner: Box<CodecPublish>,

    /// Optional target client ID that this publish message
    /// should be delivered to.
    pub target_clientid: Option<ClientId>,

    /// Delayed publish interval in seconds
    pub delay_interval: Option<u32>,

    /// Message creation timestamp
    pub create_time: Option<i64>,
}

impl Publish {
    #[inline]
    pub fn target_clientid(mut self, target_clientid: ClientId) -> Self {
        self.target_clientid = Some(target_clientid);
        self
    }

    #[inline]
    pub fn delay_interval(mut self, delay_interval: u32) -> Self {
        self.delay_interval = Some(delay_interval);
        self
    }

    #[inline]
    pub fn create_time(mut self, create_time: i64) -> Self {
        self.create_time = Some(create_time);
        self
    }

    #[inline]
    pub fn new(
        inner: Box<CodecPublish>,
        target_clientid: Option<ClientId>,
        delay_interval: Option<u32>,
        create_time: Option<i64>,
    ) -> Self {
        Publish { inner, target_clientid, delay_interval, create_time }
    }

    #[inline]
    pub fn take(self) -> Box<CodecPublish> {
        self.inner
    }

    #[inline]
    pub fn take_topic(self) -> ByteString {
        self.inner.topic
    }

    #[inline]
    pub fn take_payload(self) -> Bytes {
        self.inner.payload
    }
}

impl Deref for Publish {
    type Target = CodecPublish;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

impl DerefMut for Publish {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut()
    }
}

impl std::convert::From<CodecPublish> for Publish {
    fn from(p: CodecPublish) -> Self {
        Publish { inner: Box::new(p), target_clientid: None, delay_interval: None, create_time: None }
    }
}

impl std::convert::From<Box<CodecPublish>> for Publish {
    fn from(p: Box<CodecPublish>) -> Self {
        Publish { inner: p, target_clientid: None, delay_interval: None, create_time: None }
    }
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
                    is_utf8_payload: lw.is_utf8_payload.unwrap_or_default(),
                    response_topic: lw.response_topic.clone(),
                    ..Default::default()
                };
                (lw.retain, lw.qos, topic, lw.message.clone(), props)
            }
        };

        let p = CodecPublish {
            dup: false,
            retain,
            qos,
            topic,
            packet_id: None,
            payload,
            properties: Some(props),
        };
        let p = <CodecPublish as Into<Publish>>::into(p).create_time(timestamp_millis());
        Ok(p)
    }
}

pub enum Sink<Io> {
    V3(v3::MqttStream<Io>),
    V5(v5::MqttStream<Io>),
}

impl<Io> Sink<Io>
where
    Io: AsyncRead + AsyncWrite + Unpin,
{
    #[inline]
    pub(crate) fn v3_mut(&mut self) -> &mut v3::MqttStream<Io> {
        if let Sink::V3(s) = self {
            s
        } else {
            unreachable!()
        }
    }

    #[inline]
    pub(crate) fn v5_mut(&mut self) -> &mut v5::MqttStream<Io> {
        if let Sink::V5(s) = self {
            s
        } else {
            unreachable!()
        }
    }

    #[inline]
    pub(crate) async fn recv(&mut self) -> Result<Option<Packet>> {
        match self {
            Sink::V3(s) => match s.next().await {
                Some(Ok(pkt)) => Ok(Some(Packet::V3(pkt))),
                Some(Err(e)) => Err(e),
                None => Ok(None),
            },
            Sink::V5(s) => match s.next().await {
                Some(Ok(pkt)) => Ok(Some(Packet::V5(pkt))),
                Some(Err(e)) => Err(e),
                None => Ok(None),
            },
        }
    }

    #[inline]
    #[allow(dead_code)]
    pub(crate) async fn close(&mut self) -> Result<()> {
        match self {
            Sink::V3(s) => {
                s.close().await?;
            }
            Sink::V5(s) => s.close().await?,
        }
        Ok(())
    }

    #[inline]
    pub(crate) async fn publish(
        &mut self,
        mut p: Publish,
        message_expiry_interval: Option<NonZeroU32>,
        server_topic_aliases: Option<&Arc<ServerTopicAliases>>,
    ) -> Result<()> {
        match self {
            Sink::V3(s) => {
                s.send_publish(p.take()).await?;
            }
            Sink::V5(s) => {
                let (topic, alias) = {
                    if let Some(server_topic_aliases) = server_topic_aliases {
                        server_topic_aliases.get(p.topic.clone()).await
                    } else {
                        (Some(p.topic.clone()), None)
                    }
                };

                p.topic = topic.unwrap_or_default();

                if let Some(properties) = &mut p.properties {
                    properties.message_expiry_interval = message_expiry_interval;
                    properties.topic_alias = alias;
                }
                s.send_publish(p.take()).await?;
            }
        }
        Ok(())
    }

    #[inline]
    pub(crate) async fn send_publish_ack(&mut self, packet_id: NonZeroU16) -> Result<()> {
        match self {
            Sink::V3(s) => {
                s.send_publish_ack(packet_id).await?;
            }
            Sink::V5(s) => {
                let ack =
                    PublishAck { packet_id, reason_code: PublishAckReason::Success, ..Default::default() };
                s.send_publish_ack(ack).await?;
            }
        }
        Ok(())
    }

    #[inline]
    pub(crate) async fn send_publish_received(&mut self, packet_id: NonZeroU16) -> Result<()> {
        match self {
            Sink::V3(s) => {
                s.send_publish_received(packet_id).await?;
            }
            Sink::V5(s) => {
                let ack =
                    PublishAck { packet_id, reason_code: PublishAckReason::Success, ..Default::default() };
                s.send_publish_received(ack).await?;
            }
        }
        Ok(())
    }

    #[inline]
    #[allow(dead_code)]
    pub(crate) async fn send_publish_release(&mut self, packet_id: NonZeroU16) -> Result<()> {
        match self {
            Sink::V3(s) => {
                s.send_publish_release(packet_id).await?;
            }
            Sink::V5(s) => {
                let ack2 =
                    PublishAck2 { packet_id, reason_code: PublishAck2Reason::Success, ..Default::default() };
                s.send_publish_release(ack2).await?;
            }
        }
        Ok(())
    }

    #[inline]
    #[allow(dead_code)]
    pub(crate) async fn send(&mut self, p: Packet) -> Result<()> {
        match self {
            Sink::V3(s) => {
                if let Packet::V3(p) = p {
                    s.send(p).await?;
                }
            }
            Sink::V5(s) => {
                if let Packet::V5(p) = p {
                    s.send(p).await?;
                }
            }
        }
        Ok(())
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Packet {
    V3(codec::v3::Packet),
    V5(codec::v5::Packet),
}

#[derive(GetSize, Debug, Clone, Copy, Deserialize, Serialize)]
pub enum FromType {
    Custom,
    Admin,
    System,
    LastWill,
    Bridge,
}

impl FromType {
    #[inline]
    pub fn as_str(&self) -> &str {
        match self {
            FromType::Custom => "custom",
            FromType::Admin => "admin",
            FromType::System => "system",
            FromType::LastWill => "lastwill",
            FromType::Bridge => "bridge",
        }
    }
}

impl std::fmt::Display for FromType {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
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
        lid: ListenerId,
        local_addr: Option<SocketAddr>,
        remote_addr: Option<SocketAddr>,
        client_id: ClientId,
        username: Option<UserName>,
    ) -> Self {
        Self(Arc::new(_Id {
            node_id,
            lid,
            local_addr,
            remote_addr,
            client_id,
            username,
            create_time: timestamp_millis(),
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
        Self::new(node_id, 0, None, None, client_id, None)
    }

    #[inline]
    pub fn node(&self) -> NodeId {
        self.node_id
    }

    #[inline]
    pub fn lid(&self) -> ListenerId {
        self.lid
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
            "{}@{}:{}/{}/{}/{}/{}",
            self.node_id,
            self.local_addr.map(|addr| addr.ip().to_string()).unwrap_or_default(),
            self.lid,
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
        write!(f, "{self}")
    }
}

impl PartialEq<Id> for Id {
    #[inline]
    fn eq(&self, o: &Id) -> bool {
        self.node_id == o.node_id
            && self.lid == o.lid
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
        self.lid.hash(state);
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
    pub lid: ListenerId,
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
        + s.subscription_ids.len() * size_of::<NonZeroU32>()
}

fn get_publish_size_helper(p: &Publish) -> usize {
    // p.create_time.get_heap_size()
    p.packet_id.get_heap_size()
        // + p.dup.get_heap_size()
        + get_bytes_size_helper(&p.payload)
        // + p.retain.get_heap_size()
        + get_bytestring_size_helper(&p.topic)
        + size_of_val(&p.qos)
        + p.properties.as_ref().map(get_properties_size_helper).unwrap_or_default()
}

impl StoredMessage {
    #[inline]
    pub fn is_expiry(&self) -> bool {
        self.expiry_time_at < timestamp_millis()
    }

    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(bincode::serialize(&self)?)
    }

    #[inline]
    pub fn decode(data: &[u8]) -> Result<Self> {
        Ok(bincode::deserialize(data)?)
    }
}

#[derive(Debug)]
pub enum Message {
    Forward(From, Publish),
    SendRerelease(OutInflightMessage),
    Kick(oneshot::Sender<()>, Id, CleanStart, IsAdmin),
    // Disconnect(Disconnect),
    Closed(Reason),
    Subscribe(Subscribe, oneshot::Sender<Result<SubscribeReturn>>),
    Subscribes(Vec<Subscribe>, Option<oneshot::Sender<Vec<Result<SubscribeReturn>>>>),
    Unsubscribe(Unsubscribe, oneshot::Sender<Result<()>>),
    SessionStateTransfer(OfflineInfo, CleanStart),
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
    #[allow(unused_variables)]
    pub(crate) async fn _add(
        &self,
        scx: &ServerContext,
        topic_filter: TopicFilter,
        opts: SubscriptionOptions,
    ) -> Option<SubscriptionOptions> {
        #[cfg(feature = "shared-subscription")]
        let is_shared = opts.has_shared_group();

        let prev = {
            let mut subs = self.subs.write().await;
            let prev = subs.insert(topic_filter, opts);
            subs.shrink_to_fit();
            prev
        };

        if let Some(prev_opts) = &prev {
            #[cfg(feature = "shared-subscription")]
            match (prev_opts.has_shared_group(), is_shared) {
                (true, false) => {
                    #[cfg(feature = "stats")]
                    scx.stats.subscriptions_shared.dec();
                }
                (false, true) => {
                    #[cfg(feature = "stats")]
                    scx.stats.subscriptions_shared.inc();
                }
                (false, false) => {}
                (true, true) => {}
            }
        } else {
            #[cfg(feature = "stats")]
            scx.stats.subscriptions.inc();
            #[cfg(feature = "shared-subscription")]
            if is_shared {
                #[cfg(feature = "stats")]
                scx.stats.subscriptions_shared.inc();
            }
        }

        prev
    }

    #[inline]
    pub(crate) async fn _remove(
        &self,
        #[allow(unused_variables)] scx: &ServerContext,
        topic_filter: &str,
    ) -> Option<(TopicFilter, SubscriptionOptions)> {
        let removed = {
            let mut subs = self.subs.write().await;
            let removed = subs.remove_entry(topic_filter);
            subs.shrink_to_fit();
            removed
        };

        #[allow(unused_variables)]
        if let Some((_, opts)) = &removed {
            #[cfg(feature = "stats")]
            scx.stats.subscriptions.dec();
            #[cfg(feature = "shared-subscription")]
            if opts.has_shared_group() {
                #[cfg(feature = "stats")]
                scx.stats.subscriptions_shared.dec();
            }
        }

        removed
    }

    #[inline]
    pub(crate) async fn _drain(&self, scx: &ServerContext) -> Subscriptions {
        let topic_filters = self.subs.read().await.keys().cloned().collect::<Vec<_>>();
        let mut subs = Vec::new();
        for tf in topic_filters {
            if let Some(sub) = self._remove(scx, &tf).await {
                subs.push(sub);
            }
        }
        subs
    }

    #[inline]
    pub(crate) async fn _extend(&self, scx: &ServerContext, subs: Subscriptions) {
        for (topic_filter, opts) in subs {
            self._add(scx, topic_filter, opts).await;
        }
    }

    #[inline]
    pub async fn clear(&self, #[allow(unused_variables)] scx: &ServerContext) {
        {
            let subs = self.subs.read().await;
            #[allow(unused_variables)]
            for (_, opts) in subs.iter() {
                #[cfg(feature = "stats")]
                scx.stats.subscriptions.dec();
                #[cfg(feature = "shared-subscription")]
                if opts.has_shared_group() {
                    #[cfg(feature = "stats")]
                    scx.stats.subscriptions_shared.dec();
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
        #[cfg(feature = "shared-subscription")]
        {
            self.subs.read().await.iter().filter(|(_, opts)| opts.has_shared_group()).count()
        }
        #[cfg(not(feature = "shared-subscription"))]
        {
            0
        }
    }

    #[inline]
    pub async fn is_empty(&self) -> bool {
        self.subs.read().await.is_empty()
    }

    #[inline]
    pub async fn to_topic_filters(&self) -> TopicFilters {
        self.subs.read().await.keys().cloned().collect()
    }
}

// pub struct ExtraData<K, T> {
//     attrs: Arc<parking_lot::RwLock<HashMap<K, T>>>,
// }
//
// impl<K, T> Deref for ExtraData<K, T> {
//     type Target = Arc<parking_lot::RwLock<HashMap<K, T>>>;
//     #[inline]
//     fn deref(&self) -> &Self::Target {
//         &self.attrs
//     }
// }
//
// impl<K, T> Serialize for ExtraData<K, T>
// where
//     K: serde::Serialize,
//     T: serde::Serialize,
// {
//     #[inline]
//     fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
//     where
//         S: Serializer,
//     {
//         self.attrs.read().deref().serialize(serializer)
//     }
// }
//
// impl<'de, K, T> Deserialize<'de> for ExtraData<K, T>
// where
//     K: Eq + Hash,
//     K: serde::de::DeserializeOwned,
//     T: serde::de::DeserializeOwned,
// {
//     #[inline]
//     fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
//     where
//         D: Deserializer<'de>,
//     {
//         let v = HashMap::deserialize(deserializer)?;
//         Ok(Self { attrs: Arc::new(parking_lot::RwLock::new(v)) })
//     }
// }
//
// impl<K, T> Default for ExtraData<K, T>
// where
//     K: Eq + Hash,
// {
//     fn default() -> Self {
//         Self::new()
//     }
// }
// impl<K, T> ExtraData<K, T>
// where
//     K: Eq + Hash,
// {
//     #[inline]
//     pub fn new() -> Self {
//         Self { attrs: Arc::new(parking_lot::RwLock::new(HashMap::default())) }
//     }
//
//     #[inline]
//     pub fn len(&self) -> usize {
//         self.attrs.read().len()
//     }
//
//     #[inline]
//     pub fn is_empty(&self) -> bool {
//         self.attrs.read().is_empty()
//     }
//
//     #[inline]
//     pub fn clear(&self) {
//         self.attrs.write().clear()
//     }
//
//     #[inline]
//     pub fn insert(&self, key: K, value: T) {
//         self.attrs.write().insert(key, value);
//     }
// }
//
// pub struct ExtraAttrs {
//     attrs: HashMap<String, Box<dyn Any + Sync + Send>>,
// }
//
// impl Default for ExtraAttrs {
//     fn default() -> Self {
//         Self::new()
//     }
// }
//
// impl ExtraAttrs {
//     #[inline]
//     pub fn new() -> Self {
//         Self { attrs: HashMap::default() }
//     }
//
//     #[inline]
//     pub fn len(&self) -> usize {
//         self.attrs.len()
//     }
//
//     #[inline]
//     pub fn is_empty(&self) -> bool {
//         self.attrs.is_empty()
//     }
//
//     #[inline]
//     pub fn clear(&mut self) {
//         self.attrs.clear()
//     }
//
//     #[inline]
//     pub fn insert<T: Any + Sync + Send>(&mut self, key: String, value: T) {
//         self.attrs.insert(key, Box::new(value));
//     }
//
//     #[inline]
//     pub fn get<T: Any + Sync + Send>(&self, key: &str) -> Option<&T> {
//         self.attrs.get(key).and_then(|v| v.downcast_ref::<T>())
//     }
//
//     #[inline]
//     pub fn get_mut<T: Any + Sync + Send>(&mut self, key: &str) -> Option<&mut T> {
//         self.attrs.get_mut(key).and_then(|v| v.downcast_mut::<T>())
//     }
//
//     #[inline]
//     pub fn get_default_mut<T: Any + Sync + Send, F: Fn() -> T>(
//         &mut self,
//         key: String,
//         def_fn: F,
//     ) -> Option<&mut T> {
//         self.attrs.entry(key).or_insert_with(|| Box::new(def_fn())).downcast_mut::<T>()
//     }
//
//     #[inline]
//     pub fn serialize_key<S, T>(&self, key: &str, serializer: S) -> Result<S::Ok, S::Error>
//     where
//         T: Any + Sync + Send + serde::ser::Serialize,
//         S: Serializer,
//     {
//         self.get::<T>(key).serialize(serializer)
//     }
// }

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
    ConnectDisconnect(Option<Disconnect>),
    ConnectReadWriteTimeout,
    ConnectReadWriteError,
    ConnectRemoteClose,
    ConnectKeepaliveTimeout,
    ConnectKicked(IsAdmin),
    HandshakeRateExceeded,
    SessionExpiration,
    SubscribeFailed(Option<ByteString>),
    UnsubscribeFailed(Option<ByteString>),
    PublishFailed(ByteString),
    SubscribeRefused,
    PublishRefused,
    DelayedPublishRefused,
    MessageExpiration,
    MessageQueueFull,
    InflightWindowFull,
    ProtocolError(ByteString),
    Error(ByteString),
    MqttError(MqttError),
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
        Reason::MqttError(e)
    }
}

impl std::convert::From<Error> for Reason {
    #[inline]
    fn from(e: Error) -> Self {
        match e.downcast::<MqttError>() {
            Err(e) => Reason::Error(ByteString::from(e.to_string())),
            Ok(e) => Reason::MqttError(e),
        }
    }
}

impl ToReasonCode for Reason {
    fn to_reason_code(&self) -> DisconnectReasonCode {
        match self {
            Reason::ConnectDisconnect(Some(Disconnect::V5(d))) => d.reason_code,
            Reason::ConnectDisconnect(_) => DisconnectReasonCode::NormalDisconnection,
            Reason::ConnectReadWriteTimeout => DisconnectReasonCode::KeepAliveTimeout,
            Reason::ConnectReadWriteError => DisconnectReasonCode::UnspecifiedError,
            Reason::ConnectRemoteClose => DisconnectReasonCode::ServerShuttingDown,
            Reason::ConnectKeepaliveTimeout => DisconnectReasonCode::KeepAliveTimeout,
            Reason::ConnectKicked(is_admin) => {
                if *is_admin {
                    DisconnectReasonCode::AdministrativeAction
                } else {
                    DisconnectReasonCode::NotAuthorized
                }
            }
            Reason::HandshakeRateExceeded => DisconnectReasonCode::ConnectionRateExceeded,
            Reason::SessionExpiration => DisconnectReasonCode::SessionTakenOver,
            Reason::SubscribeFailed(_) => DisconnectReasonCode::UnspecifiedError,
            Reason::UnsubscribeFailed(_) => DisconnectReasonCode::UnspecifiedError,
            Reason::SubscribeRefused => DisconnectReasonCode::NotAuthorized,
            Reason::PublishRefused => DisconnectReasonCode::NotAuthorized,
            Reason::DelayedPublishRefused => DisconnectReasonCode::NotAuthorized,
            Reason::MessageExpiration => DisconnectReasonCode::MessageRateTooHigh,
            Reason::MessageQueueFull => DisconnectReasonCode::QuotaExceeded,
            Reason::PublishFailed(_) => DisconnectReasonCode::PacketTooLarge,
            Reason::InflightWindowFull => DisconnectReasonCode::ReceiveMaximumExceeded,
            Reason::ProtocolError(_) => DisconnectReasonCode::ProtocolError,
            Reason::Error(_) => DisconnectReasonCode::UnspecifiedError,
            Reason::MqttError(mqtt_error) => mqtt_error.to_reason_code(),
            Reason::Reasons(reasons) => {
                if let Some(first_reason) = reasons.first() {
                    first_reason.to_reason_code()
                } else {
                    DisconnectReasonCode::UnspecifiedError
                }
            }
            Reason::Unknown => DisconnectReasonCode::UnspecifiedError,
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
                    Some(r) => return write!(f, "Disconnect({r:?})"),
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
            Reason::HandshakeRateExceeded => {
                "HandshakeRateExceeded" //handshake rate exceeded
            }
            Reason::SessionExpiration => {
                "SessionExpiration" //session expiration
            }
            Reason::SubscribeFailed(r) => {
                //subscribe failed
                match r {
                    Some(r) => return write!(f, "SubscribeFailed({r})"),
                    None => "SubscribeFailed",
                }
            }
            Reason::UnsubscribeFailed(r) => {
                //unsubscribe failed
                match r {
                    Some(r) => return write!(f, "UnsubscribeFailed({r})"),
                    None => "UnsubscribeFailed",
                }
            }
            Reason::SubscribeRefused => {
                "SubscribeRefused" //subscribe refused
            }
            Reason::PublishRefused => {
                "PublishRefused" //publish refused
            }
            Reason::DelayedPublishRefused => {
                "DelayedPublishRefused" //delayed publish refused
            }
            Reason::MessageExpiration => {
                "MessageExpiration" //message expiration
            }
            Reason::MessageQueueFull => {
                "MessageQueueFull" //message deliver queue is full
            }
            Reason::PublishFailed(r) => return write!(f, "PublishFailed({r})"),
            Reason::InflightWindowFull => "Inflight window is full",
            Reason::Error(r) => r,
            Reason::MqttError(e) => &e.to_string(),
            Reason::ProtocolError(r) => return write!(f, "ProtocolError({r})"),
            Reason::Reasons(reasons) => match reasons.len() {
                0 => "",
                1 => return write!(f, "{}", reasons.first().map(|r| r.to_string()).unwrap_or_default()),
                _ => return write!(f, "{}", reasons.iter().map(|r| r.to_string()).join(",")),
            },
            Reason::Unknown => {
                "Unknown" //unknown
            }
        };
        write!(f, "{r}")
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
                    ).into()
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
                        ).into())
                    }
                    aliases.insert(alias, topic.clone());
                }
                Ok(topic)
            }
            (None, 0) => Err(MqttError::PublishAckReason(
                PublishAckReason::ImplementationSpecificError,
                ByteString::from("implementation specific error, ‘alias’ and ‘topic’ are both empty"),
            ).into()),
            (None, _) => Ok(topic),
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize, Clone)]
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
                Level::Normal(s) => s.len(),
                Level::Metadata(s) => s.len(),
                _ => 0,
            };
            size_of::<Level>() + data_len
        })
        .sum::<usize>()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DelayedPublish {
    pub expired_time: TimestampMillis,
    pub from: From,
    pub publish: Publish,
    pub message_storage_available: bool,
    pub message_expiry_interval: Option<Duration>,
}

impl DelayedPublish {
    #[inline]
    pub fn new(
        from: From,
        publish: Publish,
        message_storage_available: bool,
        message_expiry_interval: Option<Duration>,
    ) -> Self {
        let expired_time = publish
            .delay_interval
            .map(|di| timestamp_millis() + (di as TimestampMillis * 1000))
            .unwrap_or_else(timestamp_millis);
        Self { expired_time, from, publish, message_storage_available, message_expiry_interval }
    }

    #[inline]
    pub fn is_expired(&self) -> bool {
        timestamp_millis() > self.expired_time
    }
}

impl std::cmp::Eq for DelayedPublish {}

impl PartialEq for DelayedPublish {
    fn eq(&self, other: &Self) -> bool {
        other.expired_time.eq(&self.expired_time)
    }
}

impl std::cmp::Ord for DelayedPublish {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        other.expired_time.cmp(&self.expired_time)
    }
}

impl PartialOrd for DelayedPublish {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum OfflineSession {
    Exist(Option<OfflineInfo>),
    NotExist,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HealthInfo {
    pub running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descr: Option<String>,
    pub nodes: Vec<NodeHealthStatus>,
}

impl Default for HealthInfo {
    fn default() -> Self {
        Self { running: true, descr: None, nodes: vec![NodeHealthStatus::default()] }
    }
}

impl HealthInfo {
    pub fn to_json(&self) -> Value {
        let mut obj = Map::new();

        obj.insert("running".into(), json!(self.running));

        if let Some(descr) = &self.descr {
            if !descr.is_empty() {
                obj.insert("descr".into(), json!(descr));
            }
        }

        let nodes_json: Vec<Value> = self.nodes.iter().map(|node| node.to_json()).collect();

        obj.insert("nodes".into(), Value::Array(nodes_json));

        Value::Object(obj)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NodeHealthStatus {
    pub node_id: NodeId,
    pub running: bool,
    pub leader_id: Option<NodeId>,
    pub descr: Option<String>,
}

impl NodeHealthStatus {
    pub fn is_running(&self) -> bool {
        // A node is running if it's marked as running and either has no leader specified
        // or has a valid leader (including node ID 0, which is now valid)
        self.running
    }

    pub fn to_json(&self) -> Value {
        let mut obj = Map::new();

        obj.insert("node_id".into(), json!(self.node_id));
        obj.insert("running".into(), json!(self.is_running()));

        if let Some(leader_id) = &self.leader_id {
            obj.insert("leader_id".into(), json!(leader_id));
        }

        if let Some(descr) = &self.descr {
            if !descr.is_empty() {
                obj.insert("descr".into(), json!(descr));
            }
        }

        Value::Object(obj)
    }
}

impl Default for NodeHealthStatus {
    fn default() -> Self {
        Self { node_id: 0, running: true, leader_id: None, descr: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_health_status_is_running() {
        let test_cases = [
            (0, true, None, true, "None leader running"),
            (1, true, Some(0), true, "Leader ID 0 running"),
            (2, true, Some(1), true, "Leader ID 1 running"),
            (3, false, Some(0), false, "Not running"),
            (4, true, Some(1000), true, "Large leader ID running"),
        ];

        for (node_id, running, leader_id, expected, desc) in &test_cases {
            let status =
                NodeHealthStatus { node_id: *node_id, running: *running, leader_id: *leader_id, descr: None };
            assert_eq!(status.is_running(), *expected, "{}", desc);
        }
    }
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
