use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use uuid::Uuid;

use crate::acl::AuthInfo;
use crate::codec::types::Publish;
use crate::codec::types::{MQTT_LEVEL_31, MQTT_LEVEL_311, MQTT_LEVEL_5};
use crate::codec::v5::UserProperties;
use crate::codec::{v3, v5};
use crate::inflight::OutInflightMessage;
use crate::session::Session;
use crate::types::*;
use crate::utils::timestamp_millis;
use crate::{grpc, Result};

pub type Priority = u32;
pub type Proceed = bool;
pub type ReturnType = (Proceed, Option<HookResult>);

#[async_trait]
pub trait HookManager: Sync + Send {
    fn hook(&self, s: Session) -> Arc<dyn Hook>;

    fn register(&self) -> Box<dyn Register>;

    ///Before the server startup
    async fn before_startup(&self);

    ///When a connect message is received
    async fn client_connect(&self, connect_info: &ConnectInfo) -> Option<UserProperties>;

    ///authenticate
    async fn client_authenticate(
        &self,
        connect_info: &ConnectInfo,
        allow_anonymous: bool,
    ) -> (ConnectAckReason, Superuser, Option<AuthInfo>);

    ///When sending mqtt:: connectack message
    async fn client_connack(
        &self,
        connect_info: &ConnectInfo,
        return_code: ConnectAckReason,
    ) -> ConnectAckReason;

    ///Publish message received
    async fn message_publish(&self, s: Option<&Session>, from: From, publish: &Publish) -> Option<Publish>;

    ///Publish message Dropped
    async fn message_dropped(&self, to: Option<To>, from: From, p: Publish, reason: Reason);

    ///Publish message nonsubscribed
    async fn message_nonsubscribed(&self, from: From);

    ///grpc message received
    async fn grpc_message_received(
        &self,
        typ: grpc::MessageType,
        msg: grpc::Message,
    ) -> Result<grpc::MessageReply>;
}

#[async_trait]
pub trait Register: Sync + Send {
    async fn add(&self, typ: Type, handler: Box<dyn Handler>) {
        self.add_priority(typ, 0, handler).await;
    }

    async fn add_priority(&self, typ: Type, priority: Priority, handler: Box<dyn Handler>);

    async fn start(&self) {}

    async fn stop(&self) {}
}

#[async_trait]
pub trait Handler: Sync + Send {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType;
}

#[async_trait]
pub trait Hook: Sync + Send {
    ///session created
    async fn session_created(&self);

    ///After the mqtt:: connectack message is sent, the connection is created successfully
    async fn client_connected(&self);

    ///Disconnect message received
    async fn client_disconnected(&self, r: Reason);

    ///Session terminated
    async fn session_terminated(&self, r: Reason);

    ///subscribe check acl
    async fn client_subscribe_check_acl(&self, subscribe: &Subscribe) -> Option<SubscribeAclResult>;

    ///publish check acl
    async fn message_publish_check_acl(&self, publish: &Publish) -> PublishAclResult;

    ///Subscribe message received
    async fn client_subscribe(&self, subscribe: &Subscribe) -> Option<TopicFilter>;

    ///Subscription succeeded
    async fn session_subscribed(&self, subscribe: Subscribe);

    ///Unsubscribe message received
    async fn client_unsubscribe(&self, unsubscribe: &Unsubscribe) -> Option<TopicFilter>;

    ///Unsubscribe succeeded
    async fn session_unsubscribed(&self, unsubscribe: Unsubscribe);

    ///Publish message received
    async fn message_publish(&self, from: From, p: &Publish) -> Option<Publish>;

    ///Message delivered
    async fn message_delivered(&self, from: From, publish: &Publish) -> Option<Publish>;

    ///Message acked
    async fn message_acked(&self, from: From, publish: &Publish);

    ///Offline publish message
    async fn offline_message(&self, from: From, publish: &Publish);

    ///Offline inflight messages
    async fn offline_inflight_messages(&self, inflight_messages: Vec<OutInflightMessage>);

    ///Message expiry check
    async fn message_expiry_check(&self, from: From, publish: &Publish) -> MessageExpiryCheckResult;

    ///Client Keepalive
    async fn client_keepalive(&self, ping: IsPing);
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub enum Type {
    BeforeStartup,

    SessionCreated,
    SessionTerminated,
    SessionSubscribed,
    SessionUnsubscribed,

    ClientAuthenticate,
    ClientConnect,
    ClientConnack,
    ClientConnected,
    ClientDisconnected,
    ClientSubscribe,
    ClientUnsubscribe,
    ClientSubscribeCheckAcl,
    ClientKeepalive,

    MessagePublishCheckAcl,
    MessagePublish,
    MessageDelivered,
    MessageAcked,
    MessageDropped,
    MessageExpiryCheck,
    MessageNonsubscribed,

    OfflineMessage,
    OfflineInflightMessages,

    GrpcMessageReceived,
}

impl std::convert::From<&str> for Type {
    fn from(t: &str) -> Type {
        match t {
            "before_startup" => Type::BeforeStartup,

            "session_created" => Type::SessionCreated,
            "session_terminated" => Type::SessionTerminated,
            "session_subscribed" => Type::SessionSubscribed,
            "session_unsubscribed" => Type::SessionUnsubscribed,

            "client_authenticate" => Type::ClientAuthenticate,
            "client_connect" => Type::ClientConnect,
            "client_connack" => Type::ClientConnack,
            "client_connected" => Type::ClientConnected,
            "client_disconnected" => Type::ClientDisconnected,
            "client_subscribe" => Type::ClientSubscribe,
            "client_unsubscribe" => Type::ClientUnsubscribe,
            "client_subscribe_check_acl" => Type::ClientSubscribeCheckAcl,
            "client_keepalive" => Type::ClientKeepalive,

            "message_publish_check_acl" => Type::MessagePublishCheckAcl,
            "message_publish" => Type::MessagePublish,
            "message_delivered" => Type::MessageDelivered,
            "message_acked" => Type::MessageAcked,
            "message_dropped" => Type::MessageDropped,
            "message_expiry_check" => Type::MessageExpiryCheck,
            "message_nonsubscribed" => Type::MessageNonsubscribed,

            "offline_message" => Type::OfflineMessage,
            "offline_inflight_messages" => Type::OfflineInflightMessages,

            "grpc_message_received" => Type::GrpcMessageReceived,

            _ => unreachable!("{:?} is not defined", t),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Parameter<'a> {
    BeforeStartup,

    SessionCreated(&'a Session),
    SessionTerminated(&'a Session, Reason),
    SessionSubscribed(&'a Session, Subscribe),
    SessionUnsubscribed(&'a Session, Unsubscribe),

    ClientConnect(&'a ConnectInfo),
    ClientConnack(&'a ConnectInfo, &'a ConnectAckReason),
    ClientAuthenticate(&'a ConnectInfo),
    ClientConnected(&'a Session),
    ClientDisconnected(&'a Session, Reason),
    ClientSubscribe(&'a Session, &'a Subscribe),
    ClientUnsubscribe(&'a Session, &'a Unsubscribe),
    ClientSubscribeCheckAcl(&'a Session, &'a Subscribe),
    ClientKeepalive(&'a Session, IsPing),

    MessagePublishCheckAcl(&'a Session, &'a Publish),
    MessagePublish(Option<&'a Session>, From, &'a Publish),
    MessageDelivered(&'a Session, From, &'a Publish),
    MessageAcked(&'a Session, From, &'a Publish),
    MessageDropped(Option<To>, From, Publish, Reason),
    MessageExpiryCheck(&'a Session, From, &'a Publish),
    MessageNonsubscribed(From),

    OfflineMessage(&'a Session, From, &'a Publish),
    OfflineInflightMessages(&'a Session, Vec<OutInflightMessage>),
    GrpcMessageReceived(grpc::MessageType, grpc::Message),
}

impl Parameter<'_> {
    pub fn get_type(&self) -> Type {
        match self {
            Parameter::BeforeStartup => Type::BeforeStartup,

            Parameter::SessionCreated(_) => Type::SessionCreated,
            Parameter::SessionTerminated(_, _) => Type::SessionTerminated,
            Parameter::SessionSubscribed(_, _) => Type::SessionSubscribed,
            Parameter::SessionUnsubscribed(_, _) => Type::SessionUnsubscribed,

            Parameter::ClientAuthenticate(_) => Type::ClientAuthenticate,
            Parameter::ClientConnect(_) => Type::ClientConnect,
            Parameter::ClientConnack(_, _) => Type::ClientConnack,
            Parameter::ClientConnected(_) => Type::ClientConnected,
            Parameter::ClientDisconnected(_, _) => Type::ClientDisconnected,
            Parameter::ClientSubscribe(_, _) => Type::ClientSubscribe,
            Parameter::ClientUnsubscribe(_, _) => Type::ClientUnsubscribe,
            Parameter::ClientSubscribeCheckAcl(_, _) => Type::ClientSubscribeCheckAcl,
            Parameter::ClientKeepalive(_, _) => Type::ClientKeepalive,

            Parameter::MessagePublishCheckAcl(_, _) => Type::MessagePublishCheckAcl,
            Parameter::MessagePublish(_, _, _) => Type::MessagePublish,
            Parameter::MessageDelivered(_, _, _) => Type::MessageDelivered,
            Parameter::MessageAcked(_, _, _) => Type::MessageAcked,
            Parameter::MessageDropped(_, _, _, _) => Type::MessageDropped,
            Parameter::MessageExpiryCheck(_, _, _) => Type::MessageExpiryCheck,
            Parameter::MessageNonsubscribed(_) => Type::MessageNonsubscribed,

            Parameter::OfflineMessage(_, _, _) => Type::OfflineMessage,
            Parameter::OfflineInflightMessages(_, _) => Type::OfflineInflightMessages,
            Parameter::GrpcMessageReceived(_, _) => Type::GrpcMessageReceived,
        }
    }
}

#[derive(Debug)]
pub enum HookResult {
    ///User Properties, for ClientConnect
    UserProperties(UserProperties),
    ///Authentication failed, for ClientAuthenticate
    AuthResult(AuthResult),
    ///ConnectAckReason, for ClientConnack
    ConnectAckReason(ConnectAckReason),
    ///TopicFilters, for ClientSubscribe/ClientUnsubscribe
    TopicFilter(Option<TopicFilter>),
    ///Subscribe AclResult, for ClientSubscribeCheckAcl
    SubscribeAclResult(SubscribeAclResult),
    ///Publish AclResult, for MessagePublishCheckAcl
    PublishAclResult(PublishAclResult),
    ///Publish, for MessagePublish/MessageDelivered
    Publish(Publish),
    ///Message Expiry
    MessageExpiry,
    ///for GrpcMessageReceived
    GrpcMessageReply(Result<grpc::MessageReply>),
}

struct HookEntry {
    handler: Box<dyn Handler>,
    enabled: bool,
}

impl HookEntry {
    fn new(handler: Box<dyn Handler>) -> Self {
        Self { handler, enabled: false }
    }
}

type HandlerId = String;

#[derive(Clone)]
pub struct DefaultHookManager {
    #[allow(clippy::type_complexity)]
    handlers: Arc<DashMap<Type, Arc<tokio::sync::RwLock<BTreeMap<(Priority, HandlerId), HookEntry>>>>>,
}

impl Default for DefaultHookManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultHookManager {
    #[inline]
    pub fn new() -> DefaultHookManager {
        Self { handlers: Arc::new(DashMap::default()) }
    }

    #[inline]
    async fn add(&self, typ: Type, priority: Priority, handler: Box<dyn Handler>) -> Result<HandlerId> {
        let id = Uuid::new_v4().as_simple().encode_lower(&mut Uuid::encode_buffer()).to_string();
        let type_handlers =
            self.handlers.entry(typ).or_insert(Arc::new(tokio::sync::RwLock::new(BTreeMap::default())));
        let mut type_handlers = type_handlers.write().await;
        let key = (priority, id.clone());
        let contains_key = type_handlers.contains_key(&key);
        if contains_key {
            Err(anyhow!(format!("handler id is repetition, key is {:?}, type is {:?}", key, typ)))
        } else {
            type_handlers.insert(key, HookEntry::new(handler));
            Ok(id)
        }
    }

    #[inline]
    async fn exec<'a>(&'a self, t: Type, p: Parameter<'a>) -> Option<HookResult> {
        let mut acc = None;
        let type_handlers = { self.handlers.get(&t).map(|h| (*h.value()).clone()) };
        if let Some(type_handlers) = type_handlers {
            let type_handlers = type_handlers.read().await;
            for (_, entry) in type_handlers.iter().rev() {
                if entry.enabled {
                    let (proceed, new_acc) = entry.handler.hook(&p, acc).await;
                    if !proceed {
                        return new_acc;
                    }
                    acc = new_acc;
                }
            }
        }
        acc
    }
}

#[async_trait]
impl HookManager for DefaultHookManager {
    #[inline]
    fn hook(&self, s: Session) -> Arc<dyn Hook> {
        Arc::new(DefaultHook::new(self.clone(), s))
    }

    #[inline]
    fn register(&self) -> Box<dyn Register> {
        Box::new(DefaultHookRegister::new(self.clone()))
    }

    #[inline]
    async fn before_startup(&self) {
        self.exec(Type::BeforeStartup, Parameter::BeforeStartup).await;
    }

    #[inline]
    async fn client_connect(&self, connect_info: &ConnectInfo) -> Option<UserProperties> {
        let result = self.exec(Type::ClientConnect, Parameter::ClientConnect(connect_info)).await;
        log::debug!("{:?} result: {:?}", connect_info.id(), result);
        if let Some(HookResult::UserProperties(props)) = result {
            Some(props)
        } else {
            None
        }
    }

    #[inline]
    async fn client_authenticate(
        &self,
        connect_info: &ConnectInfo,
        allow_anonymous: bool,
    ) -> (ConnectAckReason, Superuser, Option<AuthInfo>) {
        let proto_ver = connect_info.proto_ver();
        let ok = || match proto_ver {
            MQTT_LEVEL_31 => ConnectAckReason::V3(v3::ConnectAckReason::ConnectionAccepted),
            MQTT_LEVEL_311 => ConnectAckReason::V3(v3::ConnectAckReason::ConnectionAccepted),
            MQTT_LEVEL_5 => ConnectAckReason::V5(v5::ConnectAckReason::Success),
            _ => ConnectAckReason::V3(v3::ConnectAckReason::ConnectionAccepted),
        };

        log::debug!("{:?} username: {:?}", connect_info.id(), connect_info.username());
        if connect_info.username().is_none() && allow_anonymous {
            return (ok(), false, None);
        }

        let result = self.exec(Type::ClientAuthenticate, Parameter::ClientAuthenticate(connect_info)).await;
        log::debug!("{:?} result: {:?}", connect_info.id(), result);
        let (bad_user_or_pass, not_auth) = match result {
            Some(HookResult::AuthResult(AuthResult::BadUsernameOrPassword)) => (true, false),
            Some(HookResult::AuthResult(AuthResult::NotAuthorized)) => (false, true),
            Some(HookResult::AuthResult(AuthResult::Allow(superuser, auth_info))) => {
                return (ok(), superuser, auth_info)
            }
            _ => {
                //or AuthResult::NotFound
                if allow_anonymous {
                    return (ok(), false, None);
                } else {
                    (false, true)
                }
            }
        };

        if bad_user_or_pass {
            return (
                match proto_ver {
                    MQTT_LEVEL_31 => ConnectAckReason::V3(v3::ConnectAckReason::BadUserNameOrPassword),
                    MQTT_LEVEL_311 => ConnectAckReason::V3(v3::ConnectAckReason::BadUserNameOrPassword),
                    MQTT_LEVEL_5 => ConnectAckReason::V5(v5::ConnectAckReason::BadUserNameOrPassword),
                    _ => ConnectAckReason::V3(v3::ConnectAckReason::BadUserNameOrPassword),
                },
                false,
                None,
            );
        }

        if not_auth {
            return (
                match proto_ver {
                    MQTT_LEVEL_31 => ConnectAckReason::V3(v3::ConnectAckReason::NotAuthorized),
                    MQTT_LEVEL_311 => ConnectAckReason::V3(v3::ConnectAckReason::NotAuthorized),
                    MQTT_LEVEL_5 => ConnectAckReason::V5(v5::ConnectAckReason::NotAuthorized),
                    _ => ConnectAckReason::V3(v3::ConnectAckReason::NotAuthorized),
                },
                false,
                None,
            );
        }

        (ok(), false, None)
    }

    ///When sending mqtt:: connectack message
    async fn client_connack(
        &self,
        connect_info: &ConnectInfo,
        return_code: ConnectAckReason,
    ) -> ConnectAckReason {
        let result =
            self.exec(Type::ClientConnack, Parameter::ClientConnack(connect_info, &return_code)).await;
        log::debug!("{:?} result: {:?}", connect_info.id(), result);
        if let Some(HookResult::ConnectAckReason(new_return_code)) = result {
            new_return_code
        } else {
            return_code
        }
    }

    #[inline]
    async fn message_publish(&self, s: Option<&Session>, from: From, publish: &Publish) -> Option<Publish> {
        let result = self.exec(Type::MessagePublish, Parameter::MessagePublish(s, from, publish)).await;
        if let Some(HookResult::Publish(publish)) = result {
            Some(publish)
        } else {
            None
        }
    }

    ///Publish message Dropped
    #[inline]
    async fn message_dropped(&self, to: Option<To>, from: From, publish: Publish, reason: Reason) {
        let _ = self.exec(Type::MessageDropped, Parameter::MessageDropped(to, from, publish, reason)).await;
    }

    ///Publish message nonsubscribed
    #[inline]
    async fn message_nonsubscribed(&self, from: From) {
        let _ = self.exec(Type::MessageNonsubscribed, Parameter::MessageNonsubscribed(from)).await;
    }

    ///grpc message received
    #[inline]
    async fn grpc_message_received(
        &self,
        typ: grpc::MessageType,
        msg: grpc::Message,
    ) -> crate::Result<grpc::MessageReply> {
        let result = self.exec(Type::GrpcMessageReceived, Parameter::GrpcMessageReceived(typ, msg)).await;
        if let Some(HookResult::GrpcMessageReply(reply)) = result {
            reply
        } else {
            Ok(grpc::MessageReply::Success)
        }
    }
}

pub struct DefaultHookRegister {
    manager: DefaultHookManager,
    type_ids: Arc<DashSet<(Type, (Priority, HandlerId))>>,
}

impl DefaultHookRegister {
    #[inline]
    fn new(manager: DefaultHookManager) -> Self {
        DefaultHookRegister { manager, type_ids: Arc::new(DashSet::default()) }
    }

    #[inline]
    async fn adjust_status(&self, b: bool) {
        for type_id in self.type_ids.iter() {
            let (typ, key) = type_id.key();
            if let Some(type_handlers) = self.manager.handlers.get(typ) {
                if let Some(entry) = type_handlers.write().await.get_mut(key) {
                    if entry.enabled != b {
                        entry.enabled = b;
                    }
                }
            }
        }
    }
}

#[async_trait]
impl Register for DefaultHookRegister {
    #[inline]
    async fn add_priority(&self, typ: Type, priority: Priority, handler: Box<dyn Handler>) {
        match self.manager.add(typ, priority, handler).await {
            Ok(id) => {
                self.type_ids.insert((typ, (priority, id)));
            }
            Err(e) => {
                log::error!("Hook add handler fail, {:?}", e);
            }
        }
    }

    #[inline]
    async fn start(&self) {
        self.adjust_status(true).await;
    }

    #[inline]
    async fn stop(&self) {
        self.adjust_status(false).await;
    }
}

#[derive(Clone)]
pub struct DefaultHook {
    manager: DefaultHookManager,
    s: Session,
}

impl DefaultHook {
    #[inline]
    pub fn new(manager: DefaultHookManager, s: Session) -> Self {
        Self { manager, s }
    }
}

#[async_trait]
impl Hook for DefaultHook {
    #[inline]
    async fn session_created(&self) {
        self.manager.exec(Type::SessionCreated, Parameter::SessionCreated(&self.s)).await;
    }

    #[inline]
    async fn client_connected(&self) {
        let _ = self.manager.exec(Type::ClientConnected, Parameter::ClientConnected(&self.s)).await;
    }

    #[inline]
    async fn client_disconnected(&self, r: Reason) {
        let _ = self.manager.exec(Type::ClientDisconnected, Parameter::ClientDisconnected(&self.s, r)).await;
    }

    #[inline]
    async fn session_terminated(&self, r: Reason) {
        let _ = self.manager.exec(Type::SessionTerminated, Parameter::SessionTerminated(&self.s, r)).await;
    }

    #[inline]
    async fn client_subscribe_check_acl(&self, sub: &Subscribe) -> Option<SubscribeAclResult> {
        if self.s.superuser().await.unwrap_or_default() {
            return Some(SubscribeAclResult::new_success(sub.opts.qos(), None));
        }
        let reply = self
            .manager
            .exec(Type::ClientSubscribeCheckAcl, Parameter::ClientSubscribeCheckAcl(&self.s, sub))
            .await;
        log::debug!("{:?} result: {:?}", self.s.id, reply);
        if let Some(HookResult::SubscribeAclResult(r)) = reply {
            Some(r)
        } else {
            None
        }
    }

    #[inline]
    async fn message_publish_check_acl(&self, publish: &Publish) -> PublishAclResult {
        if self.s.superuser().await.unwrap_or_default() {
            return PublishAclResult::Allow;
        }
        let result = self
            .manager
            .exec(Type::MessagePublishCheckAcl, Parameter::MessagePublishCheckAcl(&self.s, publish))
            .await;
        log::debug!("{:?} result: {:?}", self.s.id, result);
        if let Some(HookResult::PublishAclResult(acl_result)) = result {
            acl_result
        } else {
            PublishAclResult::Allow
        }
    }

    #[inline]
    async fn client_subscribe(&self, sub: &Subscribe) -> Option<TopicFilter> {
        let reply = self.manager.exec(Type::ClientSubscribe, Parameter::ClientSubscribe(&self.s, sub)).await;
        log::debug!("{:?} result: {:?}", self.s.id, reply);
        if let Some(HookResult::TopicFilter(Some(tf))) = reply {
            Some(tf)
        } else {
            None
        }
    }

    #[inline]
    async fn session_subscribed(&self, subscribe: Subscribe) {
        let _ = self
            .manager
            .exec(Type::SessionSubscribed, Parameter::SessionSubscribed(&self.s, subscribe))
            .await;
    }

    #[inline]
    async fn client_unsubscribe(&self, unsub: &Unsubscribe) -> Option<TopicFilter> {
        let reply =
            self.manager.exec(Type::ClientUnsubscribe, Parameter::ClientUnsubscribe(&self.s, unsub)).await;
        log::debug!("{:?} result: {:?}", self.s.id, reply);
        if let Some(HookResult::TopicFilter(Some(tf))) = reply {
            Some(tf)
        } else {
            None
        }
    }

    #[inline]
    async fn session_unsubscribed(&self, unsubscribe: Unsubscribe) {
        let _ = self
            .manager
            .exec(Type::SessionUnsubscribed, Parameter::SessionUnsubscribed(&self.s, unsubscribe))
            .await;
    }

    #[inline]
    async fn message_publish(&self, from: From, publish: &Publish) -> Option<Publish> {
        self.manager.message_publish(Some(&self.s), from, publish).await
    }

    #[inline]
    async fn message_delivered(&self, from: From, publish: &Publish) -> Option<Publish> {
        let result = self
            .manager
            .exec(Type::MessageDelivered, Parameter::MessageDelivered(&self.s, from, publish))
            .await;
        log::debug!("{:?} result: {:?}", self.s.id, result);
        if let Some(HookResult::Publish(publish)) = result {
            Some(publish)
        } else {
            None
        }
    }

    #[inline]
    async fn message_acked(&self, from: From, publish: &Publish) {
        let _ = self.manager.exec(Type::MessageAcked, Parameter::MessageAcked(&self.s, from, publish)).await;
    }

    #[inline]
    async fn offline_message(&self, from: From, publish: &Publish) {
        let _ =
            self.manager.exec(Type::OfflineMessage, Parameter::OfflineMessage(&self.s, from, publish)).await;
    }

    #[inline]
    async fn offline_inflight_messages(&self, inflight_messages: Vec<OutInflightMessage>) {
        let _ = self
            .manager
            .exec(
                Type::OfflineInflightMessages,
                Parameter::OfflineInflightMessages(&self.s, inflight_messages),
            )
            .await;
    }

    #[inline]
    async fn message_expiry_check(&self, from: From, publish: &Publish) -> MessageExpiryCheckResult {
        log::debug!("{:?} publish: {:?}", self.s.id, publish);
        let result = self
            .manager
            .exec(Type::MessageExpiryCheck, Parameter::MessageExpiryCheck(&self.s, from, publish))
            .await;
        log::debug!("{:?} result: {:?}", self.s.id, result);
        if let Some(HookResult::MessageExpiry) = result {
            return MessageExpiryCheckResult::Expiry;
        }

        let expiry_interval = publish
            .properties
            .as_ref()
            .and_then(|p| p.message_expiry_interval.map(|i| (i.get() as i64 * 1000)))
            .unwrap_or_else(|| self.s.listen_cfg().message_expiry_interval.as_millis() as i64);

        log::debug!("{:?} expiry_interval: {:?}", self.s.id, expiry_interval);
        if expiry_interval == 0 {
            return MessageExpiryCheckResult::Remaining(None);
        }
        let remaining = timestamp_millis() - publish.create_time.unwrap_or_else(timestamp_millis);
        if remaining < expiry_interval {
            return MessageExpiryCheckResult::Remaining(NonZeroU32::new(
                ((expiry_interval - remaining) / 1000) as u32,
            ));
        }
        MessageExpiryCheckResult::Expiry
    }

    #[inline]
    async fn client_keepalive(&self, ping: IsPing) {
        let _ = self.manager.exec(Type::ClientKeepalive, Parameter::ClientKeepalive(&self.s, ping)).await;
    }
}
