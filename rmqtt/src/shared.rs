//! MQTT Session Management & Message Routing Core
//!
//! Provides a distributed session management system with asynchronous message routing capabilities
//! for MQTT brokers. Key components implement thread-safe client state management and efficient
//! publish/subscribe message distribution across cluster nodes.
//!
//! ## Core Functionality
//! 1. **Session Lifecycle Management**:
//!    - Connection state tracking with atomic operations
//!    - Graceful session cleanup through `kick` mechanism
//!    - Subscription persistence across reconnects
//!
//! 2. **Message Routing**:
//!    - Topic-based message forwarding with QoS handling
//!    - Shared subscription support with load balancing
//!    - Cross-node message propagation using gRPC integration
//!
//! 3. **Cluster Coordination**:
//!    - Distributed client state tracking
//!    - Health checking and node status monitoring
//!    - Subscription synchronization across nodes
//!
//! ## Key Components
//! - `Entry` Trait: Atomic session operations including:
//!   - Subscription management (add/remove)
//!   - Message publishing with backpressure
//!   - Connection state transitions
//!   
//! - `Shared` Trait: Cluster-wide coordination implementing:
//!   - Concurrent client state storage using `DashMap`
//!   - Message routing with error handling
//!   - Subscription query interface
//!
//! - `DefaultShared`: Production-ready implementation featuring:
//!   - Async mutexes for state protection
//!   - gRPC client management for cluster communication
//!   - Message persistence integration
//!
//! ## Design Characteristics
//! 1. **Concurrency Model**:
//!    - Lock-free reads with copy-on-write patterns
//!    - Fine-grained locking using `tokio::sync::Mutex`
//!    - MPSC channels for inter-task communication
//!
//! 2. **Failure Handling**:
//!    - Timeout-based operation guards
//!    - Atomic transaction rollbacks
//!    - Graceful degradation during node failures
//!
//! 3. **Extensibility**:
//!    - Pluggable storage backends via `msgstore` feature
//!    - Router abstraction for custom subscription logic
//!    - Health monitoring hooks
//!
//! Typical usage includes:
//! - Maintaining client sessions with clean start/restore
//! - Distributing QoS 1/2 messages across cluster nodes
//! - Handling shared subscriptions with group coordination
//! - Performing cluster-wide subscription queries
//!
//! The implementation leverages Rust's ownership system and async/await patterns to achieve:
//! - Zero-copy message forwarding in common cases
//! - Linear scalability with connection count
//! - Sub-millisecond latency for core operations

use std::convert::From as _f;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
#[allow(unused_imports)]
use bitflags::Flags;
use tokio::sync::oneshot;
use tokio::sync::{Mutex, OwnedMutexGuard};

use crate::context::ServerContext;
#[cfg(feature = "grpc")]
use crate::grpc::GrpcClients;
use crate::session::Session;
use crate::types::*;
use crate::Result;

/// Represents a client session entry in the shared state.
///
/// Provides atomic operations for session lifecycle management,
/// including connection state transitions, subscription changes,
/// and message publishing. Each entry is identified by a unique
/// [`Id`] and can be locked for exclusive access.
#[async_trait]
pub trait Entry: Sync + Send {
    /// Attempt to acquire an exclusive lock on this entry.
    async fn try_lock(&self) -> Result<Box<dyn Entry>>;
    /// Return the unique identifier for this entry.
    fn id(&self) -> Id;
    /// Check whether the stored session ID matches this entry's ID.
    fn id_same(&self) -> Option<bool>;
    /// Replace the session and its associated sender channel.
    async fn set(&mut self, session: Session, tx: Tx) -> Result<()>;
    /// Remove the session entry and return the previous session and sender.
    async fn remove(&mut self) -> Result<Option<(Session, Tx)>>;
    /// Remove the session entry matching the given [`Id`].
    async fn remove_with(&mut self, id: &Id) -> Result<Option<(Session, Tx)>>;
    /// Forcefully disconnect the client session.
    ///
    /// # Arguments
    /// * `clean_start` - Whether to treat this as a clean start disconnect.
    /// * `clear_subscriptions` - Whether to remove all subscriptions.
    /// * `is_admin` - Whether the kick originates from an admin action.
    async fn kick(
        &mut self,
        clean_start: bool,
        clear_subscriptions: bool,
        is_admin: IsAdmin,
    ) -> Result<OfflineSession>;
    /// Check whether the client is currently online.
    async fn online(&self) -> bool;
    /// Check whether the client connection is active.
    async fn is_connected(&self) -> bool;
    /// Retrieve an optional reference to the stored [`Session`].
    fn session(&self) -> Option<Session>;
    /// Check whether an entry exists in the shared state.
    fn exist(&self) -> bool;
    /// Retrieve the sender channel for delivering messages to this client.
    fn tx(&self) -> Option<Tx>;
    /// Subscribe the client to a topic filter.
    async fn subscribe(&self, subscribe: &Subscribe) -> Result<SubscribeReturn>;
    /// Unsubscribe the client from a topic filter.
    async fn unsubscribe(&self, unsubscribe: &Unsubscribe) -> Result<bool>;
    /// Publish a message to this client.
    async fn publish(&self, from: From, p: Publish) -> std::result::Result<(), (From, Publish, Reason)>;
    /// List all current subscriptions for this client.
    async fn subscriptions(&self) -> Option<Vec<SubsSearchResult>>;
}

/// Cluster-wide shared session state and message routing.
///
/// Provides the core interface for distributed session management,
/// subscription tracking, and publish message forwarding across
/// cluster nodes. Implementations manage client state storage,
/// gRPC-based inter-node communication, and message delivery.
///
/// Async callback trait for processing messages loaded by `message_load_with`.
///
/// Implementations receive batches of loaded messages and can use `await`
/// internally — unlike the sync `Arc<dyn Fn(...)>` approach.
#[cfg(feature = "msgstore")]
#[async_trait]
pub trait MessageLoadCallback: Send + Sync + 'static {
    /// Called for each batch of loaded messages.
    ///
    /// # Arguments
    /// * `msgs` - A batch of messages from local storage or a remote peer.
    async fn on_messages(&self, msgs: Vec<(MsgID, From, Publish)>) -> Result<()>;
}

#[cfg(feature = "retain")]
#[async_trait]
pub trait RetainLoadCallback: Send + Sync + 'static {
    /// Called with the loaded batch of retain messages (topic + retain data).
    /// Returns the extracted (NodeId, MsgID) pairs for caller-side tracking.
    async fn on_retains(&self, retains: Vec<(TopicName, Retain)>) -> Result<Vec<(NodeId, MsgID)>>;
}

#[async_trait]
pub trait Shared: Sync + Send {
    /// Retrieve the session entry associated with the given client [`Id`].
    fn entry(&self, id: Id) -> Box<dyn Entry>;

    /// Check whether a session for the given `client_id` exists in the shared state.
    fn exist(&self, client_id: &str) -> bool;

    /// Route a publish message to all matching subscribers on this node.
    ///
    /// Returns the number of subscribers that received the message,
    /// or a tuple of (subscriber_count, delivery_failures).
    /// When `msg_id` is provided, records subscriber IDs via `mark_forwarded`
    /// after successful forwarding, ensuring subscriber tracking is tied to
    /// the forwarding result rather than deferred to a later `store()` call.
    async fn forwards(
        &self,
        msg_id: Option<MsgID>,
        from: From,
        publish: Publish,
    ) -> std::result::Result<ForwardedCount, (ForwardedCount, Vec<(To, From, Publish, Reason)>)>;

    /// Route a publish message and return both the subscription relation map and subscriber IDs.
    ///
    /// Unlike [`forwards`](Self::forwards), this also separates out shared subscription groups
    /// in the returned [`SubRelationsMap`] for caller-side processing.
    async fn forwards_and_get_shareds(
        &self,
        from: From,
        publish: Publish,
    ) -> std::result::Result<
        (SubRelationsMap, ForwardedRecipients),
        (ForwardedRecipients, Vec<(To, From, Publish, Reason)>),
    >;

    /// Forward a publish message to a specific set of subscription relations.
    ///
    /// Returns the list of subscriber sessions that received the message,
    /// or a tuple of (successful_sessions, delivery_failures).
    async fn forwards_to(
        &self,
        from: From,
        publish: &Publish,
        relations: SubRelations,
        msg_id: Option<MsgID>,
    ) -> std::result::Result<ForwardedRecipients, (ForwardedRecipients, Vec<(To, From, Publish, Reason)>)>;

    /// Return an iterator over all client session entries.
    fn iter(&self) -> Box<dyn Iterator<Item = Box<dyn Entry>> + Sync + Send + '_>;

    /// Pick a random connected session, if any exist.
    fn random_session(&self) -> Option<Session>;

    /// Retrieve the session status for a specific client ID.
    async fn session_status(&self, client_id: &str) -> Option<SessionStatus>;

    /// Return the total number of client states currently tracked.
    async fn client_states_count(&self) -> usize;

    /// Return the number of active sessions.
    fn sessions_count(&self) -> usize;

    /// Query subscriptions matching the given search parameters.
    async fn query_subscriptions(&self, q: &SubsSearchParams) -> Vec<SubsSearchResult>;

    /// Return the total number of subscriptions across all sessions.
    async fn subscriptions_count(&self) -> usize;

    ///This node is not included
    #[inline]
    #[cfg(feature = "grpc")]
    fn get_grpc_clients(&self) -> GrpcClients {
        Arc::new(HashMap::default())
    }

    #[inline]
    fn grpc_enable(&self) -> bool {
        #[cfg(feature = "grpc")]
        {
            !self.get_grpc_clients().is_empty()
        }
        #[cfg(not(feature = "grpc"))]
        false
    }

    #[inline]
    fn node_name(&self, id: NodeId) -> String {
        format!("{id}@127.0.0.1")
    }

    #[inline]
    async fn check_health(&self) -> Result<HealthInfo> {
        Ok(HealthInfo::default())
    }

    #[inline]
    async fn health_status(&self) -> Result<NodeHealthStatus> {
        Ok(NodeHealthStatus::default())
    }

    #[inline]
    fn operation_is_busy(&self) -> bool {
        false
    }

    #[cfg(feature = "msgstore")]
    async fn message_load(
        &self,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
    ) -> Result<Vec<(MsgID, From, Publish)>>;

    #[cfg(feature = "msgstore")]
    async fn message_load_with(
        &self,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
        cb: Arc<dyn MessageLoadCallback>,
    ) -> Result<()> {
        let msgs = self.message_load(client_id, topic_filter, group).await?;
        cb.on_messages(msgs).await
    }

    #[cfg(feature = "retain")]
    async fn retain_load_with(
        &self,
        topic_filter: &TopicFilter,
        cb: Arc<dyn RetainLoadCallback>,
    ) -> Result<Vec<(NodeId, MsgID)>>;

    /// Broadcast a retained message update to all cluster peers.
    ///
    /// Called after the local node has written a retain: propagates the new
    /// value to remote nodes so they can keep their local stores up to date.
    /// Default implementation is a no-op (single-node mode).
    #[cfg(feature = "retain")]
    async fn retain_set_broadcast(
        &self,
        topic: &TopicName,
        retain: &Retain,
        expiry_interval: Option<Duration>,
    ) -> Result<()> {
        let _ = (topic, retain, expiry_interval);
        Ok(())
    }

    /// Mark a message as successfully forwarded to the given subscribers.
    /// Delegates to [`MessageManager::mark_forwarded`] so that forwarding
    /// results are persisted and subsequent `get()` calls will not redeliver
    /// the same message to these clients.
    ///
    /// # Arguments
    /// * `msg_id` - Unique identifier of the forwarded message.
    /// * `subscribers` - Subscribers (with optional shared group info) that
    ///   have received the message, typically collected from shared subscription
    ///   forwarding.
    #[cfg(feature = "msgstore")]
    async fn message_mark_forwarded(
        &self,
        from_node_id: NodeId,
        msg_id: MsgID,
        recipients: ForwardedRecipients,
    ) -> Result<()>;
}

/// A locked reference to a client session entry.
///
/// Wraps an [`Entry`] with an optional mutex guard to ensure
/// exclusive access during session operations. Used internally
/// by [`DefaultShared`] to serialize access to a specific client's state.
pub struct LockEntry {
    id: Id,
    shared: DefaultShared,
    _locker: Option<OwnedMutexGuard<()>>,
}

impl Drop for LockEntry {
    #[inline]
    fn drop(&mut self) {
        if self._locker.is_some() {
            let _t = self.shared.lockers.remove(&self.id.client_id);
            log::debug!("{:?} LockEntry Drop ..., {}", self.id, _t.is_some());
        }
    }
}

impl LockEntry {
    /// Create a new `LockEntry` wrapping the given session [`Id`].
    ///
    /// # Arguments
    /// * `id` - The unique session identifier.
    /// * `shared` - Reference to the shared state manager.
    /// * `_locker` - Optional mutex guard for exclusive access.
    #[inline]
    pub fn new(id: Id, shared: DefaultShared, _locker: Option<OwnedMutexGuard<()>>) -> Self {
        Self { id, shared, _locker }
    }

    #[inline]
    async fn _unsubscribe(&self, id: Id, topic_filter: &str) -> Result<()> {
        self.shared.context().extends.router().await.remove(topic_filter, id).await?;
        if let Some(s) = self.session() {
            s.subscriptions_remove(topic_filter).await?;
        }
        Ok(())
    }

    /// Remove a peer entry identified by `with_id`, optionally clearing its subscriptions.
    #[inline]
    pub async fn _remove_with(&mut self, clear_subscriptions: bool, with_id: &Id) -> Option<(Session, Tx)> {
        if let Some((_, peer)) =
            { self.shared.peers.remove_if(&self.id.client_id, |_, entry| &entry.s.id == with_id) }
        {
            if clear_subscriptions {
                let topic_filters = peer.s.subscriptions().await.ok()?.to_topic_filters().await;
                for topic_filter in topic_filters {
                    if let Err(e) = self._unsubscribe(peer.s.id.clone(), &topic_filter).await {
                        log::warn!(
                            "{:?} remove._unsubscribe, topic_filter: {}, {:?}",
                            self.id,
                            topic_filter,
                            e
                        );
                    }
                }
            }
            log::debug!("{:?} remove ...", self.id);
            Some((peer.s, peer.tx))
        } else {
            None
        }
    }

    #[inline]
    pub async fn _remove(&mut self, clear_subscriptions: bool) -> Option<(Session, Tx)> {
        if let Some((_, peer)) = { self.shared.peers.remove(&self.id.client_id) } {
            if clear_subscriptions {
                let topic_filters = peer.s.subscriptions().await.ok()?.to_topic_filters().await;
                for topic_filter in topic_filters {
                    if let Err(e) = self._unsubscribe(peer.s.id.clone(), &topic_filter).await {
                        log::warn!(
                            "{:?} remove._unsubscribe, topic_filter: {}, {:?}",
                            self.id,
                            topic_filter,
                            e
                        );
                    }
                }
            }
            log::debug!("{:?} remove ...", self.id);
            Some((peer.s, peer.tx))
        } else {
            None
        }
    }
}

#[async_trait]
impl Entry for LockEntry {
    #[inline]
    async fn try_lock(&self) -> Result<Box<dyn Entry>> {
        log::debug!("{:?} LockEntry.try_lock", self.id);
        let locker = self
            .shared
            .lockers
            .entry(self.id.client_id.clone())
            .or_insert(Arc::new(Mutex::new(())))
            .clone()
            .try_lock_owned()?;
        Ok(Box::new(LockEntry::new(self.id.clone(), self.shared.clone(), Some(locker))))
    }

    #[inline]
    fn id(&self) -> Id {
        self.id.clone()
    }

    #[inline]
    fn id_same(&self) -> Option<bool> {
        self.shared.peers.get(&self.id.client_id).map(|peer| peer.s.id == self.id)
    }

    #[inline]
    async fn set(&mut self, s: Session, tx: Tx) -> Result<()> {
        self.shared.peers.insert(self.id.client_id.clone(), EntryItem { s, tx });
        Ok(())
    }

    #[inline]
    async fn remove(&mut self) -> Result<Option<(Session, Tx)>> {
        Ok(self._remove(true).await)
    }

    #[inline]
    async fn remove_with(&mut self, id: &Id) -> Result<Option<(Session, Tx)>> {
        Ok(self._remove_with(true, id).await)
    }

    #[inline]
    async fn kick(
        &mut self,
        clean_start: bool,
        clear_subscriptions: bool,
        is_admin: IsAdmin,
    ) -> Result<OfflineSession> {
        log::debug!(
            "{:?} LockEntry kick ..., clean_start: {}, clear_subscriptions: {}, is_admin: {}",
            self.session().map(|s| s.id.clone()),
            clean_start,
            clear_subscriptions,
            is_admin
        );

        if let Some(peer_tx) = self.tx().and_then(|tx| if tx.is_closed() { None } else { Some(tx) }) {
            let (tx, rx) = oneshot::channel();
            if let Ok(()) = peer_tx.unbounded_send(Message::Kick(tx, self.id.clone(), clean_start, is_admin))
            {
                match tokio::time::timeout(Duration::from_secs(5), rx).await {
                    Ok(Ok(())) => {
                        log::debug!("{:?} kicked, from {:?}", self.id, self.session().map(|s| s.id.clone()));
                    }
                    Ok(Err(e)) => {
                        log::warn!(
                            "{:?} kick, recv result is {:?}, from {:?}",
                            self.id,
                            e,
                            self.session().map(|s| s.id.clone())
                        );
                        return Err(anyhow!(format!("recv kick result is {:?}", e)));
                    }
                    Err(_) => {
                        log::warn!(
                            "{:?} kick, recv result is Timeout, from {:?}",
                            self.id,
                            self.session().map(|s| s.id.clone())
                        );
                    }
                }
            }
        }

        if let Some((s, _)) = self._remove(clear_subscriptions).await {
            if clean_start {
                Ok(OfflineSession::Exist(None))
            } else {
                match s.to_offline_info().await {
                    Ok(offline_info) => Ok(OfflineSession::Exist(Some(offline_info))),
                    Err(e) => {
                        log::error!("get offline info error, {e:?}");
                        Ok(OfflineSession::Exist(None))
                    }
                }
            }
        } else {
            Ok(OfflineSession::NotExist)
        }
    }

    #[inline]
    async fn online(&self) -> bool {
        self.is_connected().await
    }

    #[inline]
    async fn is_connected(&self) -> bool {
        if let Some(entry) = self.shared.peers.get(&self.id.client_id) {
            entry.s.connected().await.unwrap_or_default()
        } else {
            false
        }
    }

    #[inline]
    fn session(&self) -> Option<Session> {
        self.shared.peers.get(&self.id.client_id).map(|peer| peer.s.clone())
    }

    #[inline]
    fn exist(&self) -> bool {
        self.shared.peers.contains_key(&self.id.client_id)
    }

    #[inline]
    fn tx(&self) -> Option<Tx> {
        self.shared.peers.get(&self.id.client_id).map(|peer| peer.tx.clone())
    }

    #[inline]
    async fn subscribe(&self, sub: &Subscribe) -> Result<SubscribeReturn> {
        let scx = self.shared.context();
        let peer = self
            .shared
            .peers
            .get(&self.id.client_id)
            .map(|peer| peer.value().clone())
            .ok_or_else(|| anyhow!("session is not exist"))?;

        let this_node_id = scx.node.id();
        let node_id = peer.s.id.node_id;
        assert_eq!(
            node_id, this_node_id,
            "session node exception, session node id is {node_id}, this node id is {this_node_id}"
        );

        scx.extends.router().await.add(&sub.topic_filter, self.id.clone(), sub.opts.clone()).await?;
        let prev_opts = peer.s.subscriptions_add(sub.topic_filter.clone(), sub.opts.clone()).await?;
        Ok(SubscribeReturn::new_success(sub.opts.qos(), prev_opts))
    }

    #[inline]
    async fn unsubscribe(&self, unsubscribe: &Unsubscribe) -> Result<bool> {
        let peer = self
            .shared
            .peers
            .get(&self.id.client_id)
            .map(|peer| peer.value().clone())
            .ok_or_else(|| anyhow!("session is not exist"))?;

        if let Err(e) =
            self.shared.context().extends.router().await.remove(&unsubscribe.topic_filter, self.id()).await
        {
            log::warn!("{:?} unsubscribe, error:{:?}", self.id, e);
        }
        let remove_ok = peer.s.subscriptions_remove(&unsubscribe.topic_filter).await?.is_some();
        Ok(remove_ok)
    }

    #[inline]
    async fn publish(&self, from: From, p: Publish) -> std::result::Result<(), (From, Publish, Reason)> {
        let tx = if let Some(tx) = self.tx() {
            tx
        } else {
            log::warn!("{:?} forward, from:{:?}, error: Tx is None", self.id, from);
            return Err((from, p, Reason::from_static("Tx is None")));
        };
        if let Err(e) = tx.unbounded_send(Message::Forward(from, p)) {
            log::warn!("{:?} forward, error: {:?}", self.id, e);
            if let Message::Forward(from, p) = e.into_inner() {
                return Err((from, p, Reason::from_static("Tx is closed")));
            }
        }
        Ok(())
    }

    #[inline]
    async fn subscriptions(&self) -> Option<Vec<SubsSearchResult>> {
        if let Some(s) = self.session() {
            let subs = s
                .subscriptions()
                .await
                .ok()?
                .read()
                .await
                .iter()
                .map(|(topic_filter, opts)| SubsSearchResult {
                    node_id: s.id.node_id,
                    clientid: s.id.client_id.clone(),
                    client_addr: s.id.remote_addr,
                    topic: TopicFilter::from(topic_filter.as_ref()),
                    opts: opts.clone(),
                })
                .collect::<Vec<_>>();
            Some(subs)
        } else {
            None
        }
    }
}

#[derive(Clone)]
struct EntryItem {
    s: Session,
    tx: Tx,
}

/// Default production implementation of the [`Shared`] trait for cluster-wide session management.
///
/// Uses a `DashMap` for lock-free concurrent client state storage,
/// per-client mutexes for serialized access, and optional gRPC integration
/// for inter-node message forwarding and subscription synchronization.
#[derive(Clone)]
pub struct DefaultShared {
    scx: Option<ServerContext>,
    lockers: Arc<DashMap<ClientId, Arc<Mutex<()>>>>,
    peers: Arc<DashMap<ClientId, EntryItem>>,
}

impl DefaultShared {
    /// Create a new `DefaultShared` instance.
    #[inline]
    pub fn new(scx: Option<ServerContext>) -> DefaultShared {
        Self { scx, lockers: Arc::new(DashMap::default()), peers: Arc::new(DashMap::default()) }
    }

    #[inline]
    pub(crate) fn context(&self) -> &ServerContext {
        if let Some(scx) = &self.scx {
            scx
        } else {
            unreachable!()
        }
    }

    /// Retrieve the sender channel and target [`To`] address for a given `client_id`.
    #[inline]
    pub fn tx(&self, client_id: &str) -> Option<(Tx, To)> {
        self.peers.get(client_id).map(|peer| (peer.tx.clone(), peer.s.id.clone()))
    }

    /// Collect subscription client IDs from a [`SubRelationsMap`], resolving shared groups.
    #[inline]
    pub fn _collect_subscription_client_ids(&self, relations_map: &SubRelationsMap) -> ForwardedRecipients {
        relations_map
            .values()
            .flat_map(|subs| {
                subs.iter().flat_map(|(tf, cid, _, _, group_shared)| {
                    log::debug!(
                        "_collect_subscription_client_ids, tf: {tf:?}, cid {cid:?}, group_shared: {group_shared:?}"
                    );
                    if let Some((group, _, cids)) = group_shared {
                        cids.iter()
                            .map(|g_cid| {
                                if g_cid == cid {
                                    log::debug!(
                                        "_collect_subscription_client_ids is group_shared {g_cid:?}"
                                    );
                                    (g_cid.clone(), Some((tf.clone(), group.clone())))
                                } else {
                                    (g_cid.clone(), None)
                                }
                            })
                            .collect()
                    } else {
                        vec![(cid.clone(), None)]
                    }
                })
            })
            .collect()
    }

    /// Merge two [`ForwardedRecipients`] collections into one.
    #[inline]
    pub fn _merge_subscription_client_ids(
        &self,
        mut forwardeds: ForwardedRecipients,
        o_forwardeds: ForwardedRecipients,
    ) -> ForwardedRecipients {
        if !o_forwardeds.is_empty() {
            forwardeds.extend(o_forwardeds);
        }
        forwardeds
    }
}

#[async_trait]
impl Shared for DefaultShared {
    #[inline]
    fn entry(&self, id: Id) -> Box<dyn Entry> {
        Box::new(LockEntry::new(id, self.clone(), None))
    }

    #[inline]
    fn exist(&self, client_id: &str) -> bool {
        self.peers.contains_key(client_id)
    }

    #[inline]
    #[allow(unused_variables)]
    async fn forwards(
        &self,
        msg_id: Option<MsgID>,
        from: From,
        publish: Publish,
    ) -> std::result::Result<ForwardedCount, (ForwardedCount, Vec<(To, From, Publish, Reason)>)> {
        let scx = self.context();
        let from_node_id = from.node_id;
        let recipients_count = if let Some(target_clientid) = &publish.target_clientid {
            let mut opts = SubscriptionOptions::default();
            opts.set_qos(publish.qos);
            let relations = vec![(publish.topic.clone(), target_clientid.clone(), opts, None, None)];
            let recipients =
                self.forwards_to(from, &publish, relations, None).await.map_err(|(_, errs)| (0, errs))?;

            if from_node_id == scx.node.id() {
                #[cfg(feature = "msgstore")]
                if let Some(msg_id) = msg_id {
                    if !recipients.is_empty() {
                        if let Err(e) =
                            scx.extends.message_mgr().await.mark_forwarded(msg_id, recipients).await
                        {
                            log::warn!("forwards: mark_forwarded error, msg_id: {:?}, {e:?}", msg_id);
                        }
                    }
                }
                1
            } else {
                log::warn!(
                    "Received message from remote node, msg_id: {:?}, from node: {:?}",
                    msg_id,
                    from_node_id
                );
                0
            }
        } else {
            let mut relations_map =
                match scx.extends.router().await.matches(from.id.clone(), &publish.topic).await {
                    Ok(relations_map) => relations_map,
                    Err(e) => {
                        log::warn!("forwards, from:{:?}, topic:{:?}, error: {:?}", from, publish.topic, e);
                        SubRelationsMap::default()
                    }
                };

            let this_node_id = scx.node.id();
            let recipients_count = if let Some(relations) = relations_map.remove(&this_node_id) {
                let (recipients, errs) = match self.forwards_to(from, &publish, relations, None).await {
                    Ok(recipients) => (recipients, None),
                    Err((recipients, errs)) => (recipients, Some(errs)),
                };

                let recipients_count = recipients.len();

                #[cfg(feature = "msgstore")]
                if let Some(msg_id) = msg_id {
                    if !recipients.is_empty() {
                        if let Err(e) =
                            scx.extends.message_mgr().await.mark_forwarded(msg_id, recipients).await
                        {
                            log::warn!("forwards: mark_forwarded error, msg_id: {:?}, {e:?}", msg_id);
                        }
                    }
                }

                if let Some(errs) = errs {
                    return Err((recipients_count, errs));
                }

                recipients_count
            } else {
                0
            };
            if !relations_map.is_empty() {
                log::warn!(
                    "Received message from remote node, msg_id: {:?}, from node: {:?}, relations_map:{relations_map:?}",
                    msg_id,
                    from_node_id
                );
            }

            recipients_count
        };

        Ok(recipients_count)
    }

    #[inline]
    async fn forwards_and_get_shareds(
        &self,
        from: From,
        publish: Publish,
    ) -> std::result::Result<
        (SubRelationsMap, ForwardedRecipients),
        (ForwardedRecipients, Vec<(To, From, Publish, Reason)>),
    > {
        if let Some(target_clientid) = &publish.target_clientid {
            let mut opts = SubscriptionOptions::default();
            opts.set_qos(publish.qos);
            let relations = vec![(publish.topic.clone(), target_clientid.clone(), opts, None, None)];
            let recipients = self.forwards_to(from, &publish, relations, None).await?;
            Ok((SubRelationsMap::default(), recipients))
        } else {
            let relations_map =
                match self.context().extends.router().await.matches(from.id.clone(), &publish.topic).await {
                    Ok(relations_map) => relations_map,
                    Err(e) => {
                        log::warn!("forwards, from:{:?}, topic:{:?}, error: {:?}", from, publish.topic, e);
                        SubRelationsMap::default()
                    }
                };

            let mut relations = SubRelations::new();
            let mut sub_relations_map = SubRelationsMap::default();
            for (node_id, rels) in relations_map {
                for (topic_filter, client_id, opts, sub_ids, group) in rels {
                    if let Some(group) = group {
                        sub_relations_map.entry(node_id).or_default().push((
                            topic_filter,
                            client_id,
                            opts,
                            sub_ids,
                            Some(group),
                        ));
                    } else {
                        relations.push((topic_filter, client_id, opts, sub_ids, None));
                    }
                }
            }

            let recipients = if !relations.is_empty() {
                self.forwards_to(from, &publish, relations, None).await?
            } else {
                ForwardedRecipients::default()
            };

            Ok((sub_relations_map, recipients))
        }
    }

    #[inline]
    async fn forwards_to(
        &self,
        from: From,
        publish: &Publish,
        mut relations: SubRelations,
        _msg_id: Option<MsgID>,
    ) -> std::result::Result<ForwardedRecipients, (ForwardedRecipients, Vec<(To, From, Publish, Reason)>)>
    {
        let mut errs = Vec::new();
        let mut ok_recipients: ForwardedRecipients = Vec::new();

        for (topic_filter, client_id, opts, sub_ids, group_shared) in relations.drain(..) {
            let retain = if let Some(retain_as_published) = opts.retain_as_published() {
                //MQTT V5: Retain As Publish
                if retain_as_published {
                    publish.retain
                } else {
                    false
                }
            } else {
                false
            };

            let mut p = publish.clone();
            p.dup = false;
            p.retain = retain;
            p.qos = p.qos.less_value(opts.qos());
            p.packet_id = None;
            if let Some(sub_ids) = sub_ids {
                if let Some(p) = &mut p.properties {
                    p.subscription_ids = sub_ids;
                }
            }
            // p.properties.subscription_ids = sub_ids;

            let (tx, to) = if let Some((tx, to)) = self.tx(&client_id) {
                (tx, to)
            } else {
                log::debug!(
                    "forwards_to failed, from:{:?}, to:{:?}, topic_filter:{:?}, topic:{:?}, reason: the client has disconnected.",
                    from,
                    client_id,
                    topic_filter,
                    publish.topic
                );
                errs.push((
                    To::from(0, client_id),
                    from.clone(),
                    p,
                    Reason::from_static("the client has disconnected"),
                ));
                continue;
            };

            if let Err(e) = tx.unbounded_send(Message::Forward(from.clone(), p)) {
                log::warn!(
                    "forwards_to failed, from:{:?}, to:{:?}, topic_filter:{:?}, topic:{:?}, reason:{:?}",
                    from,
                    client_id,
                    topic_filter,
                    publish.topic,
                    e
                );
                if let Message::Forward(from, p) = e.into_inner() {
                    errs.push((to, from, p, Reason::from_static("Connection Tx is closed")));
                }
            } else {
                // Collect successful session info with shared group expansion
                if let Some((group, _, cids)) = &group_shared {
                    for g_cid in cids {
                        if g_cid == &client_id {
                            ok_recipients.push((g_cid.clone(), Some((topic_filter.clone(), group.clone()))));
                        } else {
                            ok_recipients.push((g_cid.clone(), None));
                        }
                    }
                } else {
                    ok_recipients.push((client_id, None));
                }
            }
        }

        if errs.is_empty() {
            Ok(ok_recipients)
        } else {
            Err((ok_recipients, errs))
        }
    }

    #[inline]
    fn iter(&self) -> Box<dyn Iterator<Item = Box<dyn Entry>> + Sync + Send + '_> {
        Box::new(DefaultIter { shared: self.clone(), ptr: self.peers.iter() })
    }

    #[inline]
    fn random_session(&self) -> Option<Session> {
        let mut sessions = self.peers.iter().map(|p| p.s.clone()).collect::<Vec<Session>>();
        let len = self.peers.len();
        if len > 0 {
            let idx = rand::random::<u64>() as usize % len;
            Some(sessions.remove(idx))
        } else {
            None
        }
    }

    #[inline]
    async fn session_status(&self, client_id: &str) -> Option<SessionStatus> {
        if let Some(entry) = self.peers.get(client_id) {
            Some(SessionStatus {
                id: entry.s.id.clone(),
                online: entry.s.connected().await.unwrap_or_default(),
                handshaking: false,
            })
        } else {
            None
        }
    }

    #[inline]
    async fn client_states_count(&self) -> usize {
        self.peers.len()
    }

    #[inline]
    fn sessions_count(&self) -> usize {
        self.peers.len()
    }

    #[inline]
    async fn query_subscriptions(&self, q: &SubsSearchParams) -> Vec<SubsSearchResult> {
        self.context().extends.router().await.query_subscriptions(q).await
    }

    #[inline]
    async fn subscriptions_count(&self) -> usize {
        futures::future::join_all(self.peers.iter().map(|entry| async move {
            if let Ok(subs) = entry.s.subscriptions().await {
                subs.len().await
            } else {
                0
            }
        }))
        .await
        .iter()
        .sum()
    }

    #[inline]
    #[cfg(feature = "msgstore")]
    async fn message_load(
        &self,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
    ) -> Result<Vec<(MsgID, From, Publish)>> {
        let scx = self.context();
        let message_mgr = scx.extends.message_mgr().await;
        message_mgr.get(client_id, topic_filter, group).await
    }

    #[cfg(feature = "msgstore")]
    async fn message_load_with(
        &self,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
        cb: Arc<dyn MessageLoadCallback>,
    ) -> Result<()> {
        let scx = self.context();
        let message_mgr = scx.extends.message_mgr().await;
        let msgs = message_mgr.get(client_id, topic_filter, group).await?;
        cb.on_messages(msgs).await
    }

    #[cfg(feature = "retain")]
    async fn retain_load_with(
        &self,
        topic_filter: &TopicFilter,
        cb: Arc<dyn RetainLoadCallback>,
    ) -> Result<Vec<(NodeId, MsgID)>> {
        let scx = self.context();
        let retain_mgr = scx.extends.retain().await;
        let retains = retain_mgr.get(topic_filter).await?;
        cb.on_retains(retains).await
    }

    #[inline]
    #[cfg(feature = "msgstore")]
    async fn message_mark_forwarded(
        &self,
        from_node_id: NodeId,
        msg_id: MsgID,
        recipients: ForwardedRecipients,
    ) -> Result<()> {
        let scx = self.context();
        if from_node_id == scx.node.id() {
            scx.extends.message_mgr().await.mark_forwarded(msg_id, recipients).await
        } else {
            log::warn!("forward mark flag to node {}", from_node_id);
            Ok(())
        }
    }
}

/// Iterator over all client session entries managed by [`DefaultShared`].
///
/// Wraps a `DashMap` iterator, yielding each entry as a `Box<dyn Entry>`.
pub struct DefaultIter<'a> {
    shared: DefaultShared,
    ptr: dashmap::iter::Iter<'a, ClientId, EntryItem, ahash::RandomState>,
}

impl Iterator for DefaultIter<'_> {
    type Item = Box<dyn Entry>;

    /// Advance the iterator and return the next session entry.
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.ptr.next() {
            Some(Box::new(LockEntry::new(item.s.id.clone(), self.shared.clone(), None)))
        } else {
            None
        }
    }
}

/// Deduplicate retained messages by topic, keeping the one with the latest `create_time`.
///
/// When clustering with node-local retain storage (ram/sled), the same topic may be
/// present on multiple nodes. This helper collapses those duplicates so a subscriber
/// receives only the newest retained message per topic.
#[cfg(feature = "retain")]
pub fn dedup_retains_by_topic(retains: Vec<(TopicName, Retain)>) -> Vec<(TopicName, Retain)> {
    let mut latest_idx: HashMap<TopicName, usize> = HashMap::default();
    for (idx, (topic, retain)) in retains.iter().enumerate() {
        let incoming_ts = retain.publish.create_time.unwrap_or(0);
        let keep = match latest_idx.get(topic) {
            Some(&existing_idx) => {
                let existing_ts = retains[existing_idx].1.publish.create_time.unwrap_or(0);
                incoming_ts > existing_ts
            }
            None => true,
        };
        if keep {
            latest_idx.insert(topic.clone(), idx);
        }
    }
    let mut indices: Vec<usize> = latest_idx.into_values().collect();
    indices.sort_unstable();
    indices.into_iter().map(|idx| retains[idx].clone()).collect()
}

#[cfg(all(test, feature = "retain"))]
mod tests {
    use super::*;
    use crate::codec::types::{Publish as CodecPublish, QoS};
    use crate::types::{ClientId, From, Id, Publish, Retain};
    use bytes::Bytes;
    use std::collections::HashMap as StdHashMap;

    fn make_retain(topic: &str, payload: &str, create_time: i64) -> (TopicName, Retain) {
        let topic = TopicName::from(topic);
        let codec_publish = CodecPublish {
            dup: false,
            retain: true,
            qos: QoS::AtMostOnce,
            topic: topic.clone(),
            packet_id: None,
            payload: Bytes::from(payload.to_string()),
            properties: None,
        };
        let publish = Publish::from(codec_publish).create_time(create_time);
        let retain =
            Retain { msg_id: None, from: From::from_custom(Id::from(0, ClientId::from_static(""))), publish };
        (topic, retain)
    }

    #[test]
    fn test_dedup_retains_by_topic_keeps_latest() {
        let retains = vec![
            make_retain("a/b", "old", 100),
            make_retain("a/b", "new", 200),
            make_retain("a/c", "other", 150),
        ];
        let deduped = dedup_retains_by_topic(retains);
        assert_eq!(deduped.len(), 2);
        let map: StdHashMap<String, String> = deduped
            .into_iter()
            .map(|(t, r)| (t.to_string(), String::from_utf8(r.publish.payload.to_vec()).unwrap()))
            .collect();
        assert_eq!(map.get("a/b").unwrap(), "new");
        assert_eq!(map.get("a/c").unwrap(), "other");
    }

    #[test]
    fn test_dedup_retains_by_topic_first_wins_on_equal_time() {
        let retains = vec![make_retain("a/b", "first", 100), make_retain("a/b", "second", 100)];
        let deduped = dedup_retains_by_topic(retains);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].1.publish.payload.as_ref(), b"first");
    }
}
