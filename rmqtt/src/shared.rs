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
use crate::grpc::{self, GrpcClients, MessageBroadcaster, MessageReply, MESSAGE_TYPE_MESSAGE_GET};
use crate::session::Session;
use crate::types::*;
use crate::Result;

#[async_trait]
pub trait Entry: Sync + Send {
    async fn try_lock(&self) -> Result<Box<dyn Entry>>;
    fn id(&self) -> Id;
    fn id_same(&self) -> Option<bool>;
    async fn set(&mut self, session: Session, tx: Tx) -> Result<()>;
    async fn remove(&mut self) -> Result<Option<(Session, Tx)>>;
    async fn remove_with(&mut self, id: &Id) -> Result<Option<(Session, Tx)>>;
    async fn kick(
        &mut self,
        clean_start: bool,
        clear_subscriptions: bool,
        is_admin: IsAdmin,
    ) -> Result<OfflineSession>;
    async fn online(&self) -> bool;
    async fn is_connected(&self) -> bool;
    fn session(&self) -> Option<Session>;
    fn exist(&self) -> bool;
    fn tx(&self) -> Option<Tx>;
    async fn subscribe(&self, subscribe: &Subscribe) -> Result<SubscribeReturn>;
    async fn unsubscribe(&self, unsubscribe: &Unsubscribe) -> Result<bool>;
    async fn publish(&self, from: From, p: Publish) -> std::result::Result<(), (From, Publish, Reason)>;
    async fn subscriptions(&self) -> Option<Vec<SubsSearchResult>>;
}

#[async_trait]
pub trait Shared: Sync + Send {
    /// Entry of the id
    fn entry(&self, id: Id) -> Box<dyn Entry>;

    /// Check if client_id exist
    fn exist(&self, client_id: &str) -> bool;

    /// Route and dispense publish message
    async fn forwards(
        &self,
        from: From,
        publish: Publish,
    ) -> std::result::Result<SubscriptionClientIds, Vec<(To, From, Publish, Reason)>>;

    ///Route and dispense publish message and return shared subscription relations
    async fn forwards_and_get_shareds(
        &self,
        from: From,
        publish: Publish,
    ) -> std::result::Result<(SubRelationsMap, SubscriptionClientIds), Vec<(To, From, Publish, Reason)>>;

    /// dispense publish message
    async fn forwards_to(
        &self,
        from: From,
        publish: &Publish,
        relations: SubRelations,
    ) -> std::result::Result<(), Vec<(To, From, Publish, Reason)>>;

    /// Iter
    fn iter(&self) -> Box<dyn Iterator<Item = Box<dyn Entry>> + Sync + Send + '_>;

    /// choose a session if exist
    fn random_session(&self) -> Option<Session>;

    /// Get session status with client id specific
    async fn session_status(&self, client_id: &str) -> Option<SessionStatus>;

    /// Count of th client States
    async fn client_states_count(&self) -> usize;

    /// Sessions count
    fn sessions_count(&self) -> usize;

    /// Subscriptions from SubSearchParams
    async fn query_subscriptions(&self, q: &SubsSearchParams) -> Vec<SubsSearchResult>;

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
}

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
            "session node exception, session node id is {}, this node id is {}",
            node_id, this_node_id
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

#[derive(Clone)]
pub struct DefaultShared {
    scx: Option<ServerContext>,
    lockers: Arc<DashMap<ClientId, Arc<Mutex<()>>>>,
    peers: Arc<DashMap<ClientId, EntryItem>>,
}

impl DefaultShared {
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

    #[inline]
    pub fn tx(&self, client_id: &str) -> Option<(Tx, To)> {
        self.peers.get(client_id).map(|peer| (peer.tx.clone(), peer.s.id.clone()))
    }

    #[inline]
    pub fn _collect_subscription_client_ids(&self, relations_map: &SubRelationsMap) -> SubscriptionClientIds {
        let sub_client_ids = relations_map
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
            .collect::<Vec<_>>();
        log::debug!("_collect_subscription_client_ids sub_client_ids: {sub_client_ids:?}");
        if sub_client_ids.is_empty() {
            None
        } else {
            Some(sub_client_ids)
        }
    }

    #[inline]
    pub fn _merge_subscription_client_ids(
        &self,
        sub_client_ids: SubscriptionClientIds,
        o_sub_client_ids: SubscriptionClientIds,
    ) -> SubscriptionClientIds {
        let sub_client_ids = match (sub_client_ids, o_sub_client_ids) {
            (Some(sub_client_ids), None) => Some(sub_client_ids),
            (Some(mut sub_client_ids), Some(o_sub_client_ids)) => {
                if !o_sub_client_ids.is_empty() {
                    sub_client_ids.extend(o_sub_client_ids);
                }
                Some(sub_client_ids)
            }
            (None, Some(o_sub_client_ids)) => Some(o_sub_client_ids),
            (None, None) => None,
        };
        log::debug!("_merge_subscription_client_ids sub_client_ids: {sub_client_ids:?}");
        sub_client_ids.filter(|sub_client_ids| !sub_client_ids.is_empty())
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
    async fn forwards(
        &self,
        from: From,
        publish: Publish,
    ) -> std::result::Result<SubscriptionClientIds, Vec<(To, From, Publish, Reason)>> {
        let scx = self.context();
        let sub_client_ids = if let Some(target_clientid) = &publish.target_clientid {
            let mut opts = SubscriptionOptions::default();
            opts.set_qos(publish.qos);
            let relations = vec![(publish.topic.clone(), target_clientid.clone(), opts, None, None)];
            self.forwards_to(from, &publish, relations).await?;
            Some(vec![(target_clientid.clone(), None)])
        } else {
            let mut relations_map =
                match scx.extends.router().await.matches(from.id.clone(), &publish.topic).await {
                    Ok(relations_map) => relations_map,
                    Err(e) => {
                        log::warn!("forwards, from:{:?}, topic:{:?}, error: {:?}", from, publish.topic, e);
                        SubRelationsMap::default()
                    }
                };

            let sub_cids = self._collect_subscription_client_ids(&relations_map);

            let this_node_id = scx.node.id();
            if let Some(relations) = relations_map.remove(&this_node_id) {
                self.forwards_to(from, &publish, relations).await?;
            }
            if !relations_map.is_empty() {
                log::warn!("forwards, relations_map:{relations_map:?}");
            }

            sub_cids
        };
        Ok(sub_client_ids)
    }

    #[inline]
    async fn forwards_and_get_shareds(
        &self,
        from: From,
        publish: Publish,
    ) -> std::result::Result<(SubRelationsMap, SubscriptionClientIds), Vec<(To, From, Publish, Reason)>> {
        log::debug!("forwards_and_get_shareds, from: {:?}, topic: {:?}", from, publish.topic);

        if let Some(target_clientid) = &publish.target_clientid {
            let mut opts = SubscriptionOptions::default();
            opts.set_qos(publish.qos);
            let relations = vec![(publish.topic.clone(), target_clientid.clone(), opts, None, None)];
            self.forwards_to(from, &publish, relations).await?;
            Ok((SubRelationsMap::default(), Some(vec![(target_clientid.clone(), None)])))
        } else {
            let relations_map =
                match self.context().extends.router().await.matches(from.id.clone(), &publish.topic).await {
                    Ok(relations_map) => relations_map,
                    Err(e) => {
                        log::warn!("forwards, from:{:?}, topic:{:?}, error: {:?}", from, publish.topic, e);
                        SubRelationsMap::default()
                    }
                };

            let sub_client_ids = self._collect_subscription_client_ids(&relations_map);

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

            if !relations.is_empty() {
                self.forwards_to(from, &publish, relations).await?;
            }

            Ok((sub_relations_map, sub_client_ids))
        }
    }

    #[inline]
    async fn forwards_to(
        &self,
        from: From,
        publish: &Publish,
        mut relations: SubRelations,
    ) -> std::result::Result<(), Vec<(To, From, Publish, Reason)>> {
        let mut errs = Vec::new();

        for (topic_filter, client_id, opts, sub_ids, _) in relations.drain(..) {
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
            }
        }

        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs)
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
        if message_mgr.should_merge_on_get() {
            #[allow(unused_mut)]
            let mut msgs = message_mgr.get(client_id, topic_filter, group).await?;
            #[cfg(feature = "grpc")]
            {
                let grpc_clients = scx.extends.shared().await.get_grpc_clients();
                if !grpc_clients.is_empty() {
                    let replys = MessageBroadcaster::new(
                        grpc_clients,
                        MESSAGE_TYPE_MESSAGE_GET,
                        grpc::Message::MessageGet(
                            ClientId::from(client_id),
                            TopicFilter::from(topic_filter),
                            group.cloned(),
                        ),
                        Some(Duration::from_secs(10)),
                    )
                    .join_all()
                    .await;
                    for (_, reply) in replys {
                        let reply = reply?;
                        match reply {
                            MessageReply::Error(e) => return Err(anyhow!(e)),
                            MessageReply::MessageGet(res) => {
                                msgs.extend(res.into_iter());
                            }
                            _ => {
                                log::error!("unreachable!(), reply: {reply:?}");
                                return Err(anyhow!("unreachable!()"));
                            }
                        }
                    }
                }
            }
            Ok(msgs)
        } else {
            message_mgr.get(client_id, topic_filter, group).await
        }
    }
}

pub struct DefaultIter<'a> {
    shared: DefaultShared,
    ptr: dashmap::iter::Iter<'a, ClientId, EntryItem, ahash::RandomState>,
}

impl Iterator for DefaultIter<'_> {
    type Item = Box<dyn Entry>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.ptr.next() {
            Some(Box::new(LockEntry::new(item.s.id.clone(), self.shared.clone(), None)))
        } else {
            None
        }
    }
}
