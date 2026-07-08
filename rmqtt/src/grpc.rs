//! Distributed MQTT Broker gRPC Communication Layer
//!
//! Implements inter-node communication for clustered MQTT deployments with:
//! - High-performance message forwarding
//! - Cluster-wide operations and queries
//! - Fault-tolerant request handling
//!
//! ## Core Functionality
//! 1. ​**​gRPC Server​**​:
//!    - Manages incoming cluster RPC requests
//!    - Handles message forwarding between nodes
//!    - Processes cluster-wide queries (subscriptions, routes, etc.)
//!    - Automatic reconnection on failure
//!
//! 2. ​**​gRPC Client​**​:
//!    - Connection pooling and management
//!    - Request timeout handling
//!    - Concurrent request limiting
//!    - Message queue monitoring
//!
//! 3. ​**​Message Types​**​:
//!    - 22+ predefined message types for cluster operations
//!    - Serialization/deserialization via postcard
//!    - Support for custom binary data payloads
//!
//! ## Key Features
//! - Priority-based message channels
//! - Request/response tracking
//! - Broadcast operations to all nodes
//! - Optimistic response selection
//! - Configurable timeouts and limits:
//!   - `client_timeout`: Per-request timeout
//!   - `client_concurrency_limit`: Max concurrent requests
//!   - `chunk_size`: Message fragmentation threshold (2MB default)
//!
//! ## Implementation Details
//! - Uses Tokio for async I/O
//! - Atomic counters for request tracking
//! - Zero-copy message processing where possible
//! - Connection reuse for performance
//!
//! Typical Usage:
//! 1. Initialize gRPC server with `listen_and_serve()`
//! 2. Create client connections with `GrpcClient::new()`
//! 3. Send messages via:
//!    - Direct `send_message()` for point-to-point
//!    - `MessageBroadcaster` for cluster-wide ops
//! 4. Handle responses via `MessageReply` enum
//!
//! Note: All message types below 1000 are reserved for internal use.
//!
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

#[allow(unused_imports)]
use futures::stream::FuturesUnordered;
use futures::FutureExt;
use futures::StreamExt;
use rust_box::handy_grpc::client::Mailbox;
use rust_box::handy_grpc::{
    client::{Client, DuplexMailbox},
    server::{server, Message as GrpcMessage},
    Priority,
};
use rust_box::mpsc::priority_channel as channel;
use scopeguard::defer;
use serde::{Deserialize, Serialize};
use strum::EnumCount;
use tower::{Layer, Service, ServiceExt};
use tower_resilience_circuitbreaker::{
    CircuitBreaker as TowerCB, CircuitBreakerError, CircuitBreakerLayer, CircuitMetrics, CircuitState,
    DefaultClassifier, SlidingWindowType,
};

use crate::context::CircuitBreakerConfig;
use crate::context::ServerContext;
use crate::context::WindowConfig;
use crate::types::{
    Addr, CleanStart, ClearSubscriptions, ClientId, ForwardedRecipients, From, HashMap, Id, IsAdmin, MsgID,
    NodeId, OfflineSession, Publish, Retain, Route, SessionStatus, SharedGroup, SubRelations,
    SubRelationsMap, SubsSearchParams, SubsSearchResult, TopicFilter, TopicName,
};
use crate::utils::Counter;
use crate::Result;

/// Internal type alias for async join_all_with handler futures.
type JoinAllWithAsyncFut<R> = FuturesUnordered<Pin<Box<dyn Future<Output = (NodeId, Result<R>)> + Send>>>;

/// gRPC server for inter-node cluster communication.
///
/// Listens on a configurable address and dispatches incoming
/// messages to the appropriate handler based on [`MessageType`].
#[derive(Clone)]
pub struct GrpcServer {
    scx: ServerContext,
}

impl GrpcServer {
    pub fn new(scx: ServerContext) -> Self {
        Self { scx }
    }

    pub async fn listen_and_serve(
        self,
        server_laddr: SocketAddr,
        reuseaddr: bool,
        reuseport: bool,
    ) -> Result<()> {
        let runner = async move {
            let (tx, mut rx) = channel::<Priority, GrpcMessage>(100_000);

            #[cfg(feature = "rate-counter")]
            {
                let counters = self.scx.stats.grpc_message_counters.clone();
                tokio::spawn(async move {
                    let interval = Duration::from_millis(1500);
                    loop {
                        tokio::time::sleep(interval).await;
                        for (_, c) in counters.iter() {
                            c.tick(interval);
                        }
                    }
                });
            }

            let recv_data_fut = async move {
                let exec = self.scx.get_exec(("GRPC_SERVER_EXEC", 5_000, 50_000));
                while let Some((_, (data, reply_tx))) = rx.next().await {
                    #[cfg(feature = "stats")]
                    self.scx.stats.grpc_server_actives.inc();
                    let s = self.clone();
                    let recv_fut = async move {
                        let reply = s.on_recv_message(data).await;
                        if let Some(reply_tx) = reply_tx {
                            if let Err(e) = reply_tx.send(reply.map(|r| r.unwrap_or_default())) {
                                log::warn!("gRPC send result failure, {e:?}");
                            }
                        }
                        #[cfg(feature = "stats")]
                        s.scx.stats.grpc_server_actives.dec();
                    };
                    let _ = exec.spawn(recv_fut).await;
                }
                log::error!("Recv None");
            };

            let run_receiver_fut = async move {
                loop {
                    if let Err(e) = server(server_laddr, tx.clone())
                        .max_decoding_message_size(1024 * 1024 * 4)
                        .max_encoding_message_size(1024 * 1024 * 4)
                        .reuseaddr(reuseaddr)
                        .reuseport(reuseport)
                        .run()
                        .await
                    {
                        log::warn!("Run gRPC receiver error, {e}");
                    }
                    tokio::time::sleep(Duration::from_secs(3)).await;
                }
            };
            futures::future::join(recv_data_fut, run_receiver_fut).await;
        };

        runner.await;

        Ok(())
    }

    async fn on_recv_message(&self, req: Vec<u8>) -> Result<Option<Vec<u8>>> {
        let (typ, msg) = Message::decode(&req)?;

        #[cfg(feature = "rate-counter")]
        let vid = msg.variant_id();
        #[cfg(feature = "rate-counter")]
        if let Some(c) = self.scx.stats.grpc_message_counters.get(&vid) {
            c.inc();
        }

        let reply = self.scx.extends.hook_mgr().grpc_message_received(typ, msg).await?;

        #[cfg(feature = "rate-counter")]
        if let Some(c) = self.scx.stats.grpc_message_counters.get(&vid) {
            c.dec();
        }

        Ok(Some(reply.encode()?))
    }
}

/// Shared map of connected gRPC clients indexed by node ID.
pub type GrpcClients = Arc<HashMap<NodeId, (Addr, GrpcClient)>>;

// ─── Tower Service wrappers for circuit-breaker integration ───────────────────

/// Encoded gRPC request passed to the circuit-breaker-wrapped service.
///
/// Encoding happens *before* entering the service so that encode errors
/// are NOT counted as circuit-breaker failures.
struct GrpcCall {
    req: Vec<u8>,
    timeout: Option<Duration>,
    quick: bool,
}

/// Inner service for duplex (request/response) gRPC calls.
#[derive(Clone)]
struct GrpcSendService {
    duplex_mailbox: DuplexMailbox,
}

impl Service<GrpcCall> for GrpcSendService {
    type Response = MessageReply;
    type Error = anyhow::Error;
    type Future = Pin<Box<dyn Future<Output = std::result::Result<MessageReply, anyhow::Error>> + Send>>;

    #[inline]
    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, call: GrpcCall) -> Self::Future {
        let mut mailbox = self.duplex_mailbox.clone();
        Box::pin(async move {
            let reply = if call.quick {
                if let Some(timeout) = call.timeout {
                    tokio::time::timeout(timeout, mailbox.quick_send(call.req)).await??
                } else {
                    mailbox.quick_send(call.req).await?
                }
            } else if let Some(timeout) = call.timeout {
                tokio::time::timeout(timeout, mailbox.send(call.req)).await??
            } else {
                mailbox.send(call.req).await?
            };
            MessageReply::decode(&reply)
        })
    }
}

/// Inner service for fire-and-forget gRPC notifications.
#[derive(Clone)]
struct GrpcNotifyService {
    mailbox: Mailbox,
}

impl Service<GrpcCall> for GrpcNotifyService {
    type Response = ();
    type Error = anyhow::Error;
    type Future = Pin<Box<dyn Future<Output = std::result::Result<(), anyhow::Error>> + Send>>;

    #[inline]
    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<std::result::Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, call: GrpcCall) -> Self::Future {
        let mut mailbox = self.mailbox.clone();
        Box::pin(async move {
            if call.quick {
                if let Some(timeout) = call.timeout {
                    tokio::time::timeout(timeout, mailbox.quick_send(call.req)).await??;
                } else {
                    mailbox.quick_send(call.req).await?;
                };
            } else if let Some(timeout) = call.timeout {
                tokio::time::timeout(timeout, mailbox.send(call.req)).await??;
            } else {
                mailbox.send(call.req).await?;
            }
            Ok(())
        })
    }
}

/// Build a `CircuitBreakerLayer` from a `CircuitBreakerConfig`.
///
/// This is the single place where the tower Layer is constructed from
/// the user-facing configuration struct. Each call produces a new
/// Layer (lightweight, cloning config only — actual breaker state
/// is created when `layer.layer(service)` is called).
fn build_circuit_breaker_layer(config: &CircuitBreakerConfig) -> CircuitBreakerLayer {
    let mut builder = CircuitBreakerLayer::builder()
        .failure_rate_threshold(config.failure_rate_threshold)
        .minimum_number_of_calls(config.minimum_number_of_calls)
        .wait_duration_in_open(config.wait_duration_in_open)
        .slow_call_duration_threshold(config.slow_call_duration_threshold)
        .slow_call_rate_threshold(config.slow_call_rate_threshold)
        .name(&config.name);

    match &config.window {
        WindowConfig::CountBased(w) => {
            builder = builder
                .sliding_window_type(SlidingWindowType::CountBased)
                .sliding_window_size(w.sliding_window_size);
        }
        WindowConfig::TimeBased(w) => {
            builder = builder
                .sliding_window_type(SlidingWindowType::TimeBased)
                .sliding_window_size(w.sliding_window_size)
                .sliding_window_duration(w.sliding_window_duration);
        }
    }

    builder.build()
}

/// A gRPC client connected to a remote cluster node.
///
/// Supports both request/response (duplex) and fire-and-forget
/// message passing with configurable timeouts and concurrency limits.
/// Circuit-breaker protection is integrated via Tower middleware.
#[derive(Clone)]
pub struct GrpcClient {
    mailbox: Mailbox,
    send_service: TowerCB<GrpcSendService, DefaultClassifier>,
    notify_service: TowerCB<GrpcNotifyService, DefaultClassifier>,
    active_tasks: Arc<Counter>,
}

impl GrpcClient {
    //server_addr - ip:port, 127.0.0.1:6666
    #[inline]
    pub async fn new(
        server_addr: &str,
        client_timeout: Duration,
        client_concurrency_limit: usize,
        cb_config: &CircuitBreakerConfig,
    ) -> Result<Self> {
        log::info!(
            "GrpcClient::new server_addr: {server_addr}, client_concurrency_limit: {client_concurrency_limit}"
        );
        let mut c = Client::new(server_addr.into())
            .connect_timeout(client_timeout)
            .concurrency_limit(client_concurrency_limit)
            .chunk_size(1024 * 1024 * 2)
            .connect_lazy()?;
        let duplex_mailbox = c.duplex_transfer_start(100_000).await;
        let mailbox = c.transfer_start(100_000).await;
        let active_tasks = Arc::new(Counter::new());

        // Build circuit-breaker layer from config (not a pre-built layer).
        let cb_layer = build_circuit_breaker_layer(cb_config);

        // Each layer() call creates an independent circuit-breaker state.
        let send_service = cb_layer.layer(GrpcSendService { duplex_mailbox: duplex_mailbox.clone() });
        let notify_service = cb_layer.layer(GrpcNotifyService { mailbox: mailbox.clone() });

        Ok(Self { mailbox, send_service, notify_service, active_tasks })
    }

    #[inline]
    pub fn active_tasks(&self) -> &Counter {
        self.active_tasks.as_ref()
    }

    #[inline]
    pub fn transfer_queue_len(&self) -> usize {
        self.mailbox.req_queue_len()
    }

    /// Returns the current circuit-breaker state (sync, lock-free).
    #[inline]
    pub fn circuit_breaker_state(&self) -> CircuitState {
        self.send_service.state_sync()
    }

    /// Returns `true` if the circuit breaker is currently OPEN.
    #[inline]
    pub fn circuit_breaker_is_open(&self) -> bool {
        self.send_service.is_open()
    }

    /// Returns a snapshot of circuit-breaker metrics for observability.
    #[inline]
    pub async fn circuit_breaker_metrics(&self) -> CircuitMetrics {
        self.send_service.metrics().await
    }

    /// Serializes circuit-breaker state to JSON for the admin API.
    #[inline]
    pub async fn circuit_breaker_json(&self) -> serde_json::Value {
        let m = self.send_service.metrics().await;
        serde_json::json!({
            "state": format!("{:?}", m.state),
            "is_open": m.state == CircuitState::Open,
            "total_calls": m.total_calls,
            "failure_count": m.failure_count,
            "success_count": m.success_count,
            "slow_call_count": m.slow_call_count,
            "failure_rate": m.failure_rate,
            "slow_call_rate": m.slow_call_rate,
        })
    }

    #[inline]
    pub async fn send_message(
        &mut self,
        typ: MessageType,
        msg: Message,
        timeout: Option<Duration>,
    ) -> Result<MessageReply> {
        let active_tasks = self.active_tasks.clone();
        active_tasks.inc();
        defer! {
            active_tasks.dec();
        }
        self._send_message(typ, msg, timeout, false).await
    }

    #[inline]
    pub async fn quick_send_message(
        &mut self,
        typ: MessageType,
        msg: Message,
        timeout: Option<Duration>,
    ) -> Result<MessageReply> {
        let active_tasks = self.active_tasks.clone();
        active_tasks.inc();
        defer! {
            active_tasks.dec();
        }
        self._send_message(typ, msg, timeout, true).await
    }

    #[inline]
    async fn _send_message(
        &mut self,
        typ: MessageType,
        msg: Message,
        timeout: Option<Duration>,
        quick: bool,
    ) -> Result<MessageReply> {
        let req = msg.encode(typ)?; // encoding error → fast exit, NOT a CB failure

        let result = self.send_service.clone().ready().await?.call(GrpcCall { req, timeout, quick }).await;

        result.map_err(|e| match e {
            CircuitBreakerError::OpenCircuit => {
                anyhow::anyhow!("gRPC circuit breaker is OPEN, skipping send_message")
            }
            CircuitBreakerError::Inner(e) => e,
        })
    }

    #[inline]
    pub async fn notify(&mut self, typ: MessageType, msg: Message, timeout: Option<Duration>) -> Result<()> {
        let active_tasks = self.active_tasks.clone();
        active_tasks.inc();
        defer! {
            active_tasks.dec();
        }
        self._notify(typ, msg, timeout, false).await
    }

    #[inline]
    pub async fn quick_notify(
        &mut self,
        typ: MessageType,
        msg: Message,
        timeout: Option<Duration>,
    ) -> Result<()> {
        let active_tasks = self.active_tasks.clone();
        active_tasks.inc();
        defer! {
            active_tasks.dec();
        }
        self._notify(typ, msg, timeout, true).await
    }

    #[inline]
    async fn _notify(
        &mut self,
        typ: MessageType,
        msg: Message,
        timeout: Option<Duration>,
        quick: bool,
    ) -> Result<()> {
        let req = msg.encode(typ)?; // encoding error → fast exit, NOT a CB failure

        let result = self.notify_service.clone().ready().await?.call(GrpcCall { req, timeout, quick }).await;

        result.map_err(|e| match e {
            CircuitBreakerError::OpenCircuit => {
                anyhow::anyhow!("gRPC circuit breaker is OPEN, skipping notify")
            }
            CircuitBreakerError::Inner(e) => e,
        })
    }
}

/// Message type identifier for cluster communication.
///
/// Values below 1000 are reserved for internal broker messages.
pub type MessageType = u64;

/// Cluster message payloads for inter-node operations.
///
/// Each variant represents a specific cluster operation:
/// message forwarding, client management, state queries, etc.
#[derive(Serialize, Deserialize, Clone, Debug, EnumCount)]
pub enum Message {
    Forwards(From, Publish),
    ForwardsTo(From, Publish, SubRelations, Option<MsgID>),
    ForwardsToAck(MsgID, NodeId, ForwardedRecipients),
    Kick(Id, CleanStart, ClearSubscriptions, IsAdmin),
    GetRetains(TopicFilter),
    GetAllRetains {
        offset: usize,
        limit: usize,
    },
    SetRetain(TopicName, Retain, Option<Duration>),
    /// Lightweight topic-only retain sync — set (for shared backends like Redis).
    /// Carries the topic name and expiry — the receiver unconditionally
    /// inserts the topic into its in-memory index without querying Redis.
    SetRetainTopicAdd(TopicName, Option<Duration>),
    /// Lightweight topic-only retain sync — remove (for shared backends like Redis).
    /// Carries only the topic name — the receiver unconditionally removes
    /// the topic from its in-memory index.
    SetRetainTopicRemove(TopicName),
    SubscriptionsSearch(SubsSearchParams),
    SubscriptionsGet(ClientId),
    RoutesGet(usize),
    RoutesGetBy(TopicFilter),
    NumberOfClients,
    NumberOfSessions,
    Online(ClientId),
    SessionStatus(ClientId),
    MessageGet(ClientId, TopicFilter, Option<SharedGroup>),
    Data(Vec<u8>),
}

impl Message {
    #[inline]
    pub fn encode(&self, typ: MessageType) -> Result<Vec<u8>> {
        Ok(postcard::to_stdvec(&(typ, self))?)
    }
    #[inline]
    pub fn decode(data: &[u8]) -> Result<(MessageType, Message)> {
        Ok(postcard::from_bytes::<(MessageType, Message)>(data)?)
    }

    /// Returns a unique byte ID for each enum variant, used as the key
    /// in [`Stats::grpc_message_counters`] for per-variant rate tracking.
    #[inline]
    pub fn variant_id(&self) -> u8 {
        match self {
            Message::Forwards(..) => 0,
            Message::ForwardsTo(..) => 1,
            Message::ForwardsToAck(..) => 2,
            Message::Kick(..) => 3,
            Message::GetRetains(..) => 4,
            Message::GetAllRetains { .. } => 5,
            Message::SetRetain(..) => 6,
            Message::SetRetainTopicAdd(..) => 7,
            Message::SetRetainTopicRemove(..) => 8,
            Message::SubscriptionsSearch(..) => 9,
            Message::SubscriptionsGet(..) => 10,
            Message::RoutesGet(..) => 11,
            Message::RoutesGetBy(..) => 12,
            Message::NumberOfClients => 13,
            Message::NumberOfSessions => 14,
            Message::Online(..) => 15,
            Message::SessionStatus(..) => 16,
            Message::MessageGet(..) => 17,
            Message::Data(..) => 18,
        }
    }

    /// Returns the display name for a given variant ID (inverse of [`variant_id`]).
    ///
    /// Used by [`Stats::to_sys_json`] to label per-variant rate counters.
    #[inline]
    pub fn variant_id_to_name(id: u8) -> &'static str {
        match id {
            0 => "Forwards",
            1 => "ForwardsTo",
            2 => "ForwardsToAck",
            3 => "Kick",
            4 => "GetRetains",
            5 => "GetAllRetains",
            6 => "SetRetain",
            7 => "SetRetainTopicAdd",
            8 => "SetRetainTopicRemove",
            9 => "SubscriptionsSearch",
            10 => "SubscriptionsGet",
            11 => "RoutesGet",
            12 => "RoutesGetBy",
            13 => "NumberOfClients",
            14 => "NumberOfSessions",
            15 => "Online",
            16 => "SessionStatus",
            17 => "MessageGet",
            18 => "Data",
            _ => "Unknown",
        }
    }

    /// Shorthand: returns the display name of this variant.
    #[inline]
    pub fn variant_name(&self) -> &'static str {
        Self::variant_id_to_name(self.variant_id())
    }

    /// Total number of [`Message`] variants — used to pre-allocate
    /// per-variant rate counters in [`crate::stats::Stats`].
    pub const VARIANT_COUNT: usize = <Self as EnumCount>::COUNT;
}

/// Reply variants for cluster message responses.
#[derive(Serialize, Deserialize, Debug)]
pub enum MessageReply {
    Success,
    Forwards(SubRelationsMap, ForwardedRecipients),
    Error(String),
    Kick(OfflineSession),
    GetRetains(Vec<(TopicName, Retain)>),
    GetAllRetainsChunk {
        items: Vec<(TopicName, Retain, Option<Duration>)>,
        has_more: bool,
        total_hint: usize,
    },
    SetRetain(bool),
    SubscriptionsSearch(Vec<SubsSearchResult>),
    SubscriptionsGet(Option<Vec<SubsSearchResult>>),
    RoutesGet(Vec<Route>),
    RoutesGetBy(Vec<Route>),
    NumberOfClients(usize),
    NumberOfSessions(usize),
    Online(bool),
    SessionStatus(Option<SessionStatus>),
    MessageGet(Vec<(MsgID, From, Publish)>),
    Data(Vec<u8>),
}

impl MessageReply {
    #[inline]
    pub fn encode(&self) -> Result<Vec<u8>> {
        Ok(postcard::to_stdvec(self)?)
    }
    #[inline]
    pub fn decode(data: &[u8]) -> Result<MessageReply> {
        Ok(postcard::from_bytes::<MessageReply>(data)?)
    }
}

/// Convenience wrapper for sending a single cluster message.
///
/// Supports both request/response via [`send`](MessageSender::send)
/// and fire-and-forget via [`notify`](MessageSender::notify).
pub struct MessageSender {
    client: GrpcClient,
    msg_type: MessageType,
    msg: Message,
    timeout: Option<Duration>,
    quick: bool,
}

impl MessageSender {
    #[inline]
    pub fn new(client: GrpcClient, msg_type: MessageType, msg: Message, timeout: Option<Duration>) -> Self {
        Self { client, msg_type, msg, timeout, quick: false }
    }

    #[inline]
    pub fn new_quick(
        client: GrpcClient,
        msg_type: MessageType,
        msg: Message,
        timeout: Option<Duration>,
    ) -> Self {
        Self { client, msg_type, msg, timeout, quick: true }
    }

    #[inline]
    pub async fn send(mut self) -> Result<MessageReply> {
        let reply = if self.quick {
            self.client.quick_send_message(self.msg_type, self.msg, self.timeout).await
        } else {
            self.client.send_message(self.msg_type, self.msg, self.timeout).await
        };
        match reply {
            Ok(reply) => Ok(reply),
            Err(e) => {
                log::warn!("error sending message, {e}");
                Err(e)
            }
        }
    }

    #[inline]
    pub async fn notify(mut self) -> Result<()> {
        let reply = if self.quick {
            self.client.quick_notify(self.msg_type, self.msg, self.timeout).await
        } else {
            self.client.notify(self.msg_type, self.msg, self.timeout).await
        };

        match reply {
            Ok(()) => Ok(()),
            Err(e) => {
                log::warn!("error notify message, {e}");
                Err(e)
            }
        }
    }
}

/// Broadcasts a cluster message to all connected peer nodes.
///
/// Supports:
/// - `join_all`: Wait for all peers to respond.
/// - `select_ok`: Return the first successful response.
pub struct MessageBroadcaster {
    grpc_clients: GrpcClients,
    msg_type: MessageType,
    msg: Message,
    timeout: Option<Duration>,
    quick: bool,
}

impl MessageBroadcaster {
    #[inline]
    pub fn new(
        grpc_clients: GrpcClients,
        msg_type: MessageType,
        msg: Message,
        timeout: Option<Duration>,
    ) -> Self {
        assert!(!grpc_clients.is_empty(), "gRPC clients is empty!");
        Self { grpc_clients, msg_type, msg, timeout, quick: false }
    }

    #[inline]
    pub fn new_quick(
        grpc_clients: GrpcClients,
        msg_type: MessageType,
        msg: Message,
        timeout: Option<Duration>,
    ) -> Self {
        assert!(!grpc_clients.is_empty(), "gRPC clients is empty!");
        Self { grpc_clients, msg_type, msg, timeout, quick: true }
    }

    #[inline]
    pub async fn join_all(self) -> Vec<(NodeId, Result<MessageReply>)> {
        let msg = self.msg;
        let quick = self.quick;
        let mut senders = Vec::new();
        for (id, (_, grpc_client)) in self.grpc_clients.iter() {
            let msg_type = self.msg_type;
            let msg = msg.clone();
            let fut = async move {
                let reply = if quick {
                    grpc_client.clone().quick_send_message(msg_type, msg, self.timeout).await
                } else {
                    grpc_client.clone().send_message(msg_type, msg, self.timeout).await
                };
                (*id, reply)
            };
            senders.push(fut.boxed());
        }
        futures::future::join_all(senders).await
    }

    #[inline]
    pub async fn notify_all(self) -> Vec<(NodeId, Result<()>)> {
        let msg = self.msg;
        let quick = self.quick;
        let mut senders = Vec::new();
        for (id, (_, grpc_client)) in self.grpc_clients.iter() {
            let msg_type = self.msg_type;
            let msg = msg.clone();
            let fut = async move {
                let reply = if quick {
                    grpc_client.clone().quick_notify(msg_type, msg, self.timeout).await
                } else {
                    grpc_client.clone().notify(msg_type, msg, self.timeout).await
                };
                (*id, reply)
            };
            senders.push(fut.boxed());
        }
        futures::future::join_all(senders).await
    }

    #[inline]
    pub async fn join_all_with<R, F>(self, handler: F) -> Vec<(NodeId, Result<R>)>
    where
        R: Send + Sync + 'static,
        F: Fn(Result<MessageReply>) -> Result<R> + Send + Sync,
    {
        let msg = self.msg;
        let quick = self.quick;
        let msg_type = self.msg_type;
        let timeout = self.timeout;

        let mut futures = FuturesUnordered::new();
        for (id, (_, grpc_client)) in self.grpc_clients.iter() {
            let mut client = grpc_client.clone();
            let msg = msg.clone();
            futures.push(async move {
                let reply = if quick {
                    client.quick_send_message(msg_type, msg, timeout).await
                } else {
                    client.send_message(msg_type, msg, timeout).await
                };
                (*id, reply)
            });
        }

        let mut results = Vec::new();

        while let Some((id, reply)) = futures.next().await {
            let r = handler(reply);
            results.push((id, r));
        }

        results
    }

    /// Join all peers with an **async** handler that processes each reply as it arrives.
    ///
    /// Unlike `join_all_with`, the handler returns a `Future` so callers can `.await`
    /// inside the handler — useful for feeding messages into an async callback.
    #[inline]
    pub async fn join_all_with_async<R, Fut, F>(self, handler: F) -> Vec<(NodeId, Result<R>)>
    where
        R: Send + Sync + 'static,
        Fut: Future<Output = Result<R>> + Send + 'static,
        F: Fn(Result<MessageReply>) -> Fut + Send + Sync + 'static,
    {
        let msg = self.msg;
        let msg_type = self.msg_type;
        let quick = self.quick;
        let timeout = self.timeout;

        let handler = Arc::new(handler);
        let mut futures: JoinAllWithAsyncFut<R> = JoinAllWithAsyncFut::new();
        for (&id, (_, grpc_client)) in self.grpc_clients.iter() {
            let mut client = grpc_client.clone();
            let msg = msg.clone();
            let handler = Arc::clone(&handler);
            futures.push(Box::pin(async move {
                let reply = if quick {
                    client.quick_send_message(msg_type, msg, timeout).await
                } else {
                    client.send_message(msg_type, msg, timeout).await
                };
                (id, handler(reply).await)
            }));
        }

        let mut results = Vec::with_capacity(futures.len());
        while let Some(result) = futures.next().await {
            results.push(result);
        }
        results
    }

    #[inline]
    pub async fn select_ok<R, F>(self, check_fn: F) -> Result<R>
    where
        R: std::any::Any + Send + Sync,
        F: Fn(MessageReply) -> Result<R> + Send + Sync,
    {
        let msg = self.msg;
        let quick = self.quick;
        let mut senders = Vec::new();
        let max_idx = self.grpc_clients.len() - 1;
        for (i, (_, (_, grpc_client))) in self.grpc_clients.iter().enumerate() {
            if i == max_idx {
                senders.push(
                    Self::send(grpc_client, self.msg_type, msg, self.timeout, quick, &check_fn).boxed(),
                );
                break;
            } else {
                senders.push(
                    Self::send(grpc_client, self.msg_type, msg.clone(), self.timeout, quick, &check_fn)
                        .boxed(),
                );
            }
        }
        let (reply, _) = futures::future::select_ok(senders).await?;
        Ok(reply)
    }

    #[inline]
    async fn send<R, F>(
        grpc_client: &GrpcClient,
        typ: MessageType,
        msg: Message,
        timeout: Option<Duration>,
        quick: bool,
        check_fn: &F,
    ) -> Result<R>
    where
        R: std::any::Any + Send + Sync,
        F: Fn(MessageReply) -> Result<R> + Send + Sync,
    {
        let reply = if quick {
            grpc_client.clone().quick_send_message(typ, msg, timeout).await
        } else {
            grpc_client.clone().send_message(typ, msg, timeout).await
        };

        match reply {
            Ok(r) => {
                log::debug!("OK reply: {r:?}");
                check_fn(r)
            }
            Err(e) => {
                log::debug!("ERROR reply: {e}");
                Err(e)
            }
        }
    }
}
