//! MQTT Broker Plugin Hook System
//!
//! Provides extensible event-driven architecture for MQTT broker operations with:
//! - 30+ hook points across the client lifecycle
//! - Priority-based handler execution
//! - Protocol version-aware processing
//! - Comprehensive ACL integration
//!
//! ## Core Functionality
//! 1. ​**​Lifecycle Management​**​:
//!    - Client connection/disconnection events
//!    - Session creation/termination
//!    - Subscription state changes
//!
//! 2. ​**​Message Processing​**​:
//!    - Publish authorization checks
//!    - Message delivery/acknowledgement
//!    - Offline message handling
//!    - Message expiry validation
//!
//! 3. ​**​Access Control​**​:
//!    - Authentication/authorization hooks
//!    - Subscribe/publish ACL verification
//!    - Superuser privilege handling
//!
//! ## Key Features
//! - Protocol version specific handling (v3.1, v3.1.1, v5)
//! - Priority-based handler ordering (0=highest)
//! - Atomic hook registration/enabling
//! - Session-aware processing context
//! - Comprehensive result type system
//!
//! ## Implementation Details
//! - Async trait-based interface
//! - DashMap-backed concurrent storage
//! - UUID-based handler identification
//! - BTreeMap for priority ordering
//! - Zero-cost abstraction for common cases
//!
//! Typical Usage:
//! 1. Implement `Handler` trait for custom logic
//! 2. Register via `HookManager::register()`
//! 3. Process hook results via `HookResult` enum
//!
//! Note: All hooks are executed in reverse priority order (high to low)
//! until a handler returns `proceed=false`.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::acl::AuthInfo;
use crate::codec::types::{MQTT_LEVEL_31, MQTT_LEVEL_311, MQTT_LEVEL_5};
use crate::codec::v5::UserProperties;
use crate::codec::{v3, v5};
#[cfg(feature = "grpc")]
use crate::grpc;
use crate::inflight::OutInflightMessage;
use crate::session::Session;
use crate::types::{
    AuthResult, ConnectAckReason, ConnectInfo, DashMap, DashSet, From, IsPing, MessageExpiryCheckResult,
    Publish, PublishAclResult, Reason, Subscribe, SubscribeAclResult, Superuser, To, TopicFilter,
    Unsubscribe,
};
use crate::utils::timestamp_millis;
use crate::Result;

/// Hook handler priority value.
///
/// Lower numerical values indicate higher priority.
/// Priority 0 is the highest (executed first).
pub type Priority = u32;

/// Whether hook chain execution should continue.
///
/// `true` allows subsequent handlers to execute.
/// `false` short-circuits the chain and returns the current result.
pub type Proceed = bool;

/// Return type for hook handler execution.
///
/// The first element indicates whether to continue processing
/// subsequent handlers. The second element is the accumulated
/// result passed through the handler chain.
pub type ReturnType = (Proceed, Option<HookResult>);

/// Manages the lifecycle and execution of all hook points in the broker.
///
/// Implementations organize hook handlers by [`Type`], execute them
/// in priority order, and manage the accumulation of results across
/// the handler chain.
///
/// # Hook Execution Flow
///
/// 1. A session-level event occurs (e.g., connect, publish, subscribe).
/// 2. [`HookManager`] dispatches to the appropriate hook [`Type`].
/// 3. Registered handlers execute in reverse priority order (highest first).
/// 4. Each handler receives the accumulated result from previous handlers.
/// 5. If any handler returns `proceed=false`, execution short-circuits.
/// 6. The final result is returned to the caller.
///
/// # Thread Safety
///
/// All hook methods are async and require `Sync + Send` bounds
/// to support concurrent session processing.
#[async_trait]
pub trait HookManager: Sync + Send {
    /// Create a session-scoped [`Hook`] instance.
    ///
    /// The returned `Hook` is bound to the given [`Session`] and
    /// provides convenient access to session-aware hook execution.
    fn hook(&self, s: Session) -> Arc<dyn Hook>;

    /// Obtain a [`Register`] handle for registering hook handlers.
    ///
    /// The returned handle can add handlers with specific priorities,
    /// and start/stop them in batch.
    fn register(&self) -> Box<dyn Register>;

    /// Triggered before the server starts accepting connections.
    ///
    /// Hook handlers can perform one-time initialization that must
    /// complete before the broker begins operation.
    async fn before_startup(&self);

    /// Triggered when a CONNECT packet is received from a client.
    ///
    /// Returns optional [`UserProperties`] that will be included
    /// in the CONNACK response to the client.
    async fn client_connect(&self, connect_info: &ConnectInfo) -> Option<UserProperties>;

    /// Authenticate a connecting client.
    ///
    /// Returns a triple of (connection acknowledgment reason,
    /// superuser status, optional authentication info).
    /// If `allow_anonymous` is true and no username is provided,
    /// authentication is automatically granted.
    async fn client_authenticate(
        &self,
        connect_info: &ConnectInfo,
        allow_anonymous: bool,
    ) -> (ConnectAckReason, Superuser, Option<AuthInfo>);

    /// Triggered when a CONNACK is about to be sent to the client.
    ///
    /// Allows inspection or modification of the connection
    /// acknowledgment reason code before it is transmitted.
    async fn client_connack(
        &self,
        connect_info: &ConnectInfo,
        return_code: ConnectAckReason,
    ) -> ConnectAckReason;

    /// Intercept an incoming PUBLISH message.
    ///
    /// Returns `Some(Publish)` with a (possibly modified) message
    /// to continue processing, or `None` to silently drop it.
    async fn message_publish(&self, s: Option<&Session>, from: From, publish: &Publish) -> Option<Publish>;

    /// Notify that a publish message was dropped before delivery.
    ///
    /// Triggered when a message could not be delivered to any
    /// matching subscriber or was discarded for other reasons.
    async fn message_dropped(&self, to: Option<To>, from: From, p: Publish, reason: Reason);

    /// Notify that a publish message had no matching subscribers.
    async fn message_nonsubscribed(&self, from: From);

    /// Handle a gRPC message received from another cluster node.
    #[cfg(feature = "grpc")]
    async fn grpc_message_received(
        &self,
        typ: grpc::MessageType,
        msg: grpc::Message,
    ) -> Result<grpc::MessageReply>;
}

/// Manages the registration lifecycle of hook handlers.
///
/// Provides methods to add handlers with configurable priority,
/// and to start/stop all registered handlers atomically.
///
/// # Lifecycle
///
/// 1. `add` / `add_priority` — register a handler for a hook type.
/// 2. `start` — enable all registered handlers (sets `enabled = true`).
/// 3. `stop` — disable all registered handlers (sets `enabled = false`).
///
/// Handlers are not active until `start()` is called.
#[async_trait]
pub trait Register: Sync + Send {
    /// Register a handler with default priority (0, highest).
    async fn add(&self, typ: Type, handler: Box<dyn Handler>) {
        self.add_priority(typ, 0, handler).await;
    }

    /// Register a handler with a specific priority level.
    ///
    /// Lower priority values execute first.
    async fn add_priority(&self, typ: Type, priority: Priority, handler: Box<dyn Handler>);

    /// Activate all registered handlers.
    ///
    /// After this call, handlers will begin receiving hook events.
    async fn start(&self) {}

    /// Deactivate all registered handlers.
    ///
    /// After this call, handlers will stop receiving hook events.
    async fn stop(&self) {}
}

/// Processes hook events and returns results.
///
/// Implementations define custom behavior for specific hook points.
/// Each handler receives the hook parameters and any accumulated
/// result from previous handlers in the chain.
///
/// # Return Value
///
/// A `ReturnType` tuple:
/// - First element (`Proceed`): `false` short-circuits the chain.
/// - Second element: Updated accumulated result for subsequent handlers.
#[async_trait]
pub trait Handler: Sync + Send {
    /// Execute this handler for the given hook event.
    ///
    /// # Arguments
    ///
    /// * `param` — The hook event context and parameters.
    /// * `acc` — The accumulated result from prior handlers, if any.
    ///
    /// # Returns
    ///
    /// A `ReturnType` indicating whether to proceed and the
    /// (possibly updated) accumulated result.
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType;
}

/// Session-scoped hook interface for client connection events.
///
/// A `Hook` instance is created per session via [`HookManager::hook`].
/// It provides access to all session-level hook points without
/// requiring the caller to pass session context explicitly.
///
/// # ACL Behavior
///
/// Superuser sessions automatically bypass ACL checks for
/// both subscribe and publish operations.
#[async_trait]
pub trait Hook: Sync + Send {
    /// A new session has been created for this connection.
    async fn session_created(&self);

    /// The connection has been established after CONNACK was sent.
    async fn client_connected(&self);

    /// A DISCONNECT packet has been received from the client.
    async fn client_disconnected(&self, r: Reason);

    /// The session has been terminated (cleanup complete).
    async fn session_terminated(&self, r: Reason);

    /// Check ACL for a subscribe request.
    ///
    /// Returns `Some(SubscribeAclResult)` with the ACL decision,
    /// or `None` to allow by default.
    /// Superuser sessions automatically pass this check.
    async fn client_subscribe_check_acl(&self, subscribe: &Subscribe) -> Option<SubscribeAclResult>;

    /// Check ACL for a publish request.
    ///
    /// Returns the ACL decision for the given publish message.
    /// Superuser sessions automatically pass this check.
    async fn message_publish_check_acl(&self, publish: &Publish) -> PublishAclResult;

    /// A SUBSCRIBE packet has been received from the client.
    ///
    /// Returns an optional modified [`TopicFilter`] to replace
    /// the original subscription.
    async fn client_subscribe(&self, subscribe: &Subscribe) -> Option<TopicFilter>;

    /// A subscription has been successfully added to the session.
    async fn session_subscribed(&self, subscribe: Subscribe);

    /// An UNSUBSCRIBE packet has been received from the client.
    ///
    /// Returns an optional modified [`TopicFilter`] to replace
    /// the original unsubscription.
    async fn client_unsubscribe(&self, unsubscribe: &Unsubscribe) -> Option<TopicFilter>;

    /// An unsubscription has been successfully removed from the session.
    async fn session_unsubscribed(&self, unsubscribe: Unsubscribe);

    /// An incoming PUBLISH message has been received.
    ///
    /// Returns `Some(Publish)` with a potentially modified message,
    /// or `None` to drop the message silently.
    async fn message_publish(&self, from: From, p: &Publish) -> Option<Publish>;

    /// A message has been delivered to the client.
    ///
    /// Returns `Some(Publish)` to replace the delivered message,
    /// or `None` to keep the original.
    async fn message_delivered(&self, from: From, publish: &Publish) -> Option<Publish>;

    /// A message has been acknowledged by the client (QoS 1/2).
    async fn message_acked(&self, from: From, publish: &Publish);

    /// A publish message has been queued for offline delivery.
    async fn offline_message(&self, from: From, publish: &Publish);

    /// Offline inflight messages are being prepared for a reconnecting client.
    ///
    /// These are QoS 1/2 messages that were in-flight when the
    /// client disconnected but not yet acknowledged.
    async fn offline_inflight_messages(&self, inflight_messages: Vec<OutInflightMessage>);

    /// Check whether a message has expired before delivery.
    ///
    /// Returns [`MessageExpiryCheckResult::Expiry`] if the message
    /// should be discarded, or [`MessageExpiryCheckResult::Remaining`]
    /// with the time left before expiry.
    async fn message_expiry_check(&self, from: From, publish: &Publish) -> MessageExpiryCheckResult;

    /// The client's keepalive timer has triggered.
    ///
    /// `IsPing` indicates whether the client sent a ping request.
    async fn client_keepalive(&self, ping: IsPing);
}

/// Identifies the type of hook event.
///
/// Each variant corresponds to a specific point in the MQTT
/// client or message lifecycle where plugins can intervene.
///
/// # Categories
///
/// - **Startup**: `BeforeStartup`
/// - **Session lifecycle**: `SessionCreated`, `SessionTerminated`,
///   `SessionSubscribed`, `SessionUnsubscribed`
/// - **Client lifecycle**: `ClientAuthenticate` through `ClientKeepalive`
/// - **Message lifecycle**: `MessagePublishCheckAcl` through `MessageNonsubscribed`
/// - **Offline handling**: `OfflineMessage`, `OfflineInflightMessages`
/// - **Cluster**: `GrpcMessageReceived`
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Deserialize, Serialize)]
pub enum Type {
    /// Before the server starts listening for connections.
    BeforeStartup,

    /// A new client session has been created.
    SessionCreated,
    /// A client session has been terminated.
    SessionTerminated,
    /// A subscription has been added to a session.
    SessionSubscribed,
    /// A subscription has been removed from a session.
    SessionUnsubscribed,

    /// Authenticate a connecting client.
    ClientAuthenticate,
    /// Process a received CONNECT packet.
    ClientConnect,
    /// Modify the CONNACK reason code before sending.
    ClientConnack,
    /// A client connection has been fully established.
    ClientConnected,
    /// A client has disconnected.
    ClientDisconnected,
    /// Process a received SUBSCRIBE packet.
    ClientSubscribe,
    /// Process a received UNSUBSCRIBE packet.
    ClientUnsubscribe,
    /// Check ACL permissions for a subscribe request.
    ClientSubscribeCheckAcl,
    /// Client keepalive timeout or ping received.
    ClientKeepalive,

    /// Check ACL permissions for a publish request.
    MessagePublishCheckAcl,
    /// Process an incoming PUBLISH packet.
    MessagePublish,
    /// A message was delivered to a subscriber.
    MessageDelivered,
    /// A message was acknowledged by the subscriber.
    MessageAcked,
    /// A message was dropped before delivery.
    MessageDropped,
    /// Check whether a message has expired.
    MessageExpiryCheck,
    /// A published message had no matching subscribers.
    MessageNonsubscribed,

    /// A publish message queued for offline delivery.
    OfflineMessage,
    /// Inflight messages loaded for a reconnecting client.
    OfflineInflightMessages,

    /// A gRPC message received from another cluster node.
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

/// Hook execution parameters for handler callbacks.
///
/// Each variant carries the contextual data needed by handlers
/// for a specific hook event. Parameters reference session state,
/// message data, and other ephemeral event information.
///
/// # Lifetime
///
/// The lifetime `'a` is tied to the borrow of the session and
/// message data. Handlers must not capture references that
/// outlive the hook invocation.
#[derive(Debug, Clone)]
pub enum Parameter<'a> {
    /// No parameters for server startup.
    BeforeStartup,

    /// A new session was created.
    SessionCreated(&'a Session),
    /// A session was terminated with the given reason.
    SessionTerminated(&'a Session, Reason),
    /// A subscription was added.
    SessionSubscribed(&'a Session, Subscribe),
    /// A subscription was removed.
    SessionUnsubscribed(&'a Session, Unsubscribe),

    /// A CONNECT packet was received.
    ClientConnect(&'a ConnectInfo),
    /// A CONNACK is about to be sent.
    ClientConnack(&'a ConnectInfo, &'a ConnectAckReason),
    /// Authentication request.
    ClientAuthenticate(&'a ConnectInfo),
    /// A client connection was established.
    ClientConnected(&'a Session),
    /// A client disconnected.
    ClientDisconnected(&'a Session, Reason),
    /// A SUBSCRIBE packet was received.
    ClientSubscribe(&'a Session, &'a Subscribe),
    /// An UNSUBSCRIBE packet was received.
    ClientUnsubscribe(&'a Session, &'a Unsubscribe),
    /// ACL check for a subscribe request.
    ClientSubscribeCheckAcl(&'a Session, &'a Subscribe),
    /// Keepalive timer event.
    ClientKeepalive(&'a Session, IsPing),

    /// ACL check for a publish request.
    MessagePublishCheckAcl(&'a Session, &'a Publish),
    /// An incoming PUBLISH packet.
    MessagePublish(Option<&'a Session>, From, &'a Publish),
    /// A message was delivered to a subscriber.
    MessageDelivered(&'a Session, From, &'a Publish),
    /// A message was acknowledged.
    MessageAcked(&'a Session, From, &'a Publish),
    /// A message was dropped.
    MessageDropped(Option<To>, From, Publish, Reason),
    /// Expiry check for a publish message.
    MessageExpiryCheck(&'a Session, From, &'a Publish),
    /// A published message had no subscribers.
    MessageNonsubscribed(From),

    /// A message queued for offline delivery.
    OfflineMessage(&'a Session, From, &'a Publish),
    /// Inflight messages for a reconnecting client.
    OfflineInflightMessages(&'a Session, Vec<OutInflightMessage>),
    /// A gRPC cluster message.
    #[cfg(feature = "grpc")]
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
            #[cfg(feature = "grpc")]
            Parameter::GrpcMessageReceived(_, _) => Type::GrpcMessageReceived,
        }
    }
}

/// Accumulated result from hook handler execution.
///
/// The variant indicates which hook type produced the result
/// and carries type-specific data. This enum is accumulated
/// across the handler chain: each handler may inspect and
/// replace the current result.
///
/// # Handling
///
/// Callers match on the variant to extract the relevant data
/// for the hook type they invoked. Unmatched variants should
/// be treated as "no modification" by convention.
#[derive(Debug)]
pub enum HookResult {
    /// User properties returned from a `ClientConnect` hook.
    UserProperties(UserProperties),
    /// Authentication result from a `ClientAuthenticate` hook.
    AuthResult(AuthResult),
    /// Modified connection acknowledgment reason code.
    ConnectAckReason(ConnectAckReason),
    /// Modified topic filter from subscribe/unsubscribe hooks.
    TopicFilter(Option<TopicFilter>),
    /// ACL decision for a subscribe request.
    SubscribeAclResult(SubscribeAclResult),
    /// ACL decision for a publish request.
    PublishAclResult(PublishAclResult),
    /// Modified publish message from publish/delivery hooks.
    Publish(Publish),
    /// Indicates the message has expired.
    MessageExpiry,
    /// Reply to a gRPC cluster message.
    #[cfg(feature = "grpc")]
    GrpcMessageReply(Result<grpc::MessageReply>),
}

/// Internal registry entry for a single hook handler.
///
/// Tracks the handler instance and its enabled state.
/// Handlers can be registered but disabled, allowing
/// deferred activation via the [`Register`] lifecycle.
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

/// The default [`HookManager`] implementation.
///
/// Stores handlers in a concurrent map keyed by hook [`Type`],
/// with each type having a sorted collection of `(Priority, HandlerId)` entries.
///
/// # Concurrency
///
/// Uses a two-level locking strategy:
/// - Outer [`DashMap`] for lock-free sharded access by hook type.
/// - Inner [`tokio::sync::RwLock`] protecting the `BTreeMap` of handlers
///   for fine-grained concurrent reads during hook execution.
///
/// # Handler Execution
///
/// Handlers are executed in reverse BTreeMap order (highest priority first).
/// Each handler receives the accumulated result from prior handlers.
/// If any handler returns `proceed = false`, the chain short-circuits.
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
    #[cfg(feature = "grpc")]
    async fn grpc_message_received(
        &self,
        typ: grpc::MessageType,
        msg: grpc::Message,
    ) -> Result<grpc::MessageReply> {
        let result = self.exec(Type::GrpcMessageReceived, Parameter::GrpcMessageReceived(typ, msg)).await;
        if let Some(HookResult::GrpcMessageReply(reply)) = result {
            reply
        } else {
            Ok(grpc::MessageReply::Success)
        }
    }
}

/// Default [`Register`] implementation that manages handler lifecycle.
///
/// Tracks all registered handlers by type and ID, and provides
/// batch enable/disable operations via [`start`](Register::start)
/// and [`stop`](Register::stop).
///
/// # Implementation
///
/// Uses a [`DashSet`] to maintain a set of `(Type, (Priority, HandlerId))`
/// tuples. On `start`/`stop`, iterates over the set and toggles the
/// `enabled` flag on each entry's [`HookEntry`].
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
                log::error!("Hook add handler fail, {e:?}");
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

/// Default [`Hook`] implementation bound to a specific session.
///
/// Wraps a [`DefaultHookManager`] and a [`Session`] reference,
/// providing session-scoped hook execution. All hook methods
/// delegate to the manager with the session as part of the
/// execution parameter.
///
/// # ACL Integration
///
/// `client_subscribe_check_acl` and `message_publish_check_acl`
/// automatically bypass ACL checks for superuser sessions before
/// delegating to registered handlers.
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
            return PublishAclResult::allow();
        }
        let result = self
            .manager
            .exec(Type::MessagePublishCheckAcl, Parameter::MessagePublishCheckAcl(&self.s, publish))
            .await;
        log::debug!("{:?} result: {:?}", self.s.id, result);
        if let Some(HookResult::PublishAclResult(acl_result)) = result {
            acl_result
        } else {
            PublishAclResult::allow()
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
            .and_then(|p| p.message_expiry_interval.map(|i| i.get() as i64 * 1000))
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
