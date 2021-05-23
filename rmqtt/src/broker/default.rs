use anyhow::{Error, Result};
use leaky_bucket::LeakyBucket;
use once_cell::sync::OnceCell;
use parking_lot::RwLock;
use std::convert::From as _;
use std::iter::Iterator;
use std::num::NonZeroU32;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::{self, Mutex, OwnedMutexGuard};
use tokio::time::Duration;
use uuid::Uuid;
type DashSet<V> = dashmap::DashSet<V, ahash::RandomState>;
type DashMap<K, V> = dashmap::DashMap<K, V, ahash::RandomState>;
type LinkedMap<K, V> = linked_hash_map::LinkedHashMap<K, V, ahash::RandomState>;

use ntex_mqtt::types::{MQTT_LEVEL_31, MQTT_LEVEL_311, MQTT_LEVEL_5};
use ntex_mqtt::v3::codec::SubscribeReturnCode as SubscribeReturnCodeV3;

use super::{
    retain::RetainTree, topic::TopicTree, Entry, Limiter, LimiterManager, RetainStorage, Router, Shared,
};
use crate::broker::fitter::{Fitter, FitterManager};
use crate::broker::hook::{Handler, Hook, HookManager, HookResult, Parameter, Register, Type};
use crate::broker::session::{ClientInfo, Session, SessionOfflineInfo};
use crate::broker::types::*;
use crate::settings::listener::Listener;
use crate::{grpc, ClientId, Id, NodeId, QoS, Runtime, Topic, TopicFilter};

pub struct LockEntry {
    id: Id,
    shared: &'static DefaultShared,
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
    pub fn new(id: Id, shared: &'static DefaultShared, _locker: Option<OwnedMutexGuard<()>>) -> Self {
        Self { id, shared, _locker }
    }

    #[inline]
    pub async fn _unsubscribe(&self, id: Id, topic_filter: &TopicFilter) -> Result<()> {
        {
            let router = Runtime::instance().extends.router().await;
            router.remove(topic_filter, Runtime::instance().node.id(), &id.client_id).await?;
        }
        Ok(())
    }

    #[inline]
    pub async fn _remove(&mut self, clear_subscriptions: bool) -> Option<(Session, Tx, ClientInfo)> {
        if let Some((_, peer)) = self.shared.peers.remove(&self.id.client_id) {
            if clear_subscriptions {
                for (topic_filter, _) in peer.s.subscriptions().await.iter() {
                    if let Err(e) = self._unsubscribe(peer.c.id.clone(), topic_filter).await {
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
            Some((peer.s, peer.tx, peer.c))
        } else {
            None
        }
    }
}

#[async_trait]
impl super::Entry for LockEntry {
    #[inline]
    fn try_lock(&self) -> Result<Box<dyn Entry>> {
        log::debug!("{:?} try_lock", self.id);
        let locker = self
            .shared
            .lockers
            .entry(self.id.client_id.clone())
            .or_insert(Arc::new(Mutex::new(())))
            .clone()
            .try_lock_owned()?;
        Ok(Box::new(LockEntry::new(self.id.clone(), self.shared, Some(locker))))
    }

    #[inline]
    fn id(&self) -> Id {
        self.id.clone()
    }

    #[inline]
    async fn set(&mut self, s: Session, tx: Tx, c: ClientInfo) -> Result<()> {
        self.shared.peers.insert(self.id.client_id.clone(), EntryItem { s, tx, c });
        Ok(())
    }

    #[inline]
    async fn remove(&mut self) -> Result<Option<(Session, Tx, ClientInfo)>> {
        Ok(self._remove(true).await)
    }

    #[inline]
    async fn kick(&mut self, clear_subscriptions: bool) -> Result<Option<SessionOfflineInfo>> {
        if let Some((s, peer_tx, c)) = self._remove(clear_subscriptions).await {
            let (tx, mut rx) = mpsc::unbounded_channel();
            if let Err(e) = peer_tx.send(Message::Kick(tx, self.id.clone())) {
                log::warn!("{:?} kick, {:?}, {:?}", self.id, c.id, e);
            } else {
                match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
                    Ok(_) => {
                        log::debug!("{:?} kicked, from {:?}", self.id, c.id);
                    }
                    Err(_) => {
                        log::warn!("{:?} kick, recv result is Timeout, from {:?}", self.id, c.id);
                    }
                }
            }
            Ok(Some(s.to_offline_info().await))
        } else {
            Ok(None)
        }
    }

    #[inline]
    async fn is_connected(&self) -> bool {
        if let Some(entry) = self.shared.peers.get(&self.id.client_id) {
            entry.c.connected.load(Ordering::SeqCst)
        } else {
            false
        }
    }

    #[inline]
    async fn session(&self) -> Option<Session> {
        self.shared.peers.get(&self.id.client_id).map(|peer| peer.s.clone())
    }

    #[inline]
    async fn client(&self) -> Option<ClientInfo> {
        self.shared.peers.get(&self.id.client_id).map(|peer| peer.c.clone())
    }

    #[inline]
    fn tx(&self) -> Option<Tx> {
        self.shared.peers.get(&self.id.client_id).map(|peer| peer.tx.clone())
    }

    #[inline]
    async fn subscribe(&self, subscribe: Subscribe) -> Result<SubscribeAck> {
        let peer = self
            .shared
            .peers
            .get(&self.id.client_id)
            .map(|peer| peer.value().clone())
            .ok_or_else(|| anyhow::Error::msg("session is not exist"))?;

        let router = Runtime::instance().extends.router().await;
        let this_node_id = Runtime::instance().node.id();
        let node_id = peer.c.id.node_id;
        assert_eq!(
            node_id, this_node_id,
            "session node exception, session node id is {}, this node id is {}",
            node_id, this_node_id
        );

        let ack = match subscribe {
            Subscribe::V3(mut topic_filters) => {
                let mut acks = Vec::new();
                for (topic_filter, qos) in topic_filters.drain(..) {
                    if let Err(e) = router.add(&topic_filter, node_id, &self.id.client_id, qos).await {
                        log::warn!(
                            "{:?} subscribes fail, node_id: {}, topic_filter:{}, {:?}",
                            self.id,
                            node_id,
                            topic_filter,
                            e
                        );
                        acks.push(SubscribeReturnCodeV3::Failure);
                    } else {
                        peer.s.subscriptions_add(topic_filter, qos).await;
                        acks.push(SubscribeReturnCodeV3::Success(qos));
                    }
                }
                SubscribeAck::V3(acks)
            }
            Subscribe::V5(_subs) => {
                return Err(anyhow::Error::msg("Not implemented"));
            }
        };
        Ok(ack)
    }

    #[inline]
    async fn unsubscribe(&self, unsubscribe: &Unsubscribe) -> Result<UnsubscribeAck> {
        let peer = self
            .shared
            .peers
            .get(&self.id.client_id)
            .map(|peer| peer.value().clone())
            .ok_or_else(|| anyhow::Error::msg("session is not exist"))?;

        let ack = match unsubscribe {
            Unsubscribe::V3(topic_filters) => {
                for topic_filter in topic_filters.iter() {
                    if let Err(e) = self._unsubscribe(peer.c.id.clone(), topic_filter).await {
                        log::warn!("{:?} unsubscribe, error:{:?}", self.id, e);
                    }
                    peer.s.subscriptions_remove(topic_filter).await;
                }
                UnsubscribeAck::V3
            }
            Unsubscribe::V5(_subs) => {
                return Err(anyhow::Error::msg("Not implemented"));
            }
        };
        Ok(ack)
    }

    #[inline]
    async fn publish(&self, from: From, p: Publish) -> Result<(), (From, Publish, Reason)> {
        let tx = if let Some(tx) = self.tx() {
            tx
        } else {
            log::warn!("{:?} forward, from:{:?}, error: Tx is None", self.id, from);
            return Err((from, p, Reason::from_static("Tx is None")));
        };
        if let Err(e) = tx.send(Message::Forward(from, p)) {
            log::warn!("{:?} forward, error: {:?}", self.id, e);
            if let Message::Forward(from, p) = e.0 {
                return Err((from, p, Reason::from_static("Tx is closed")));
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
struct EntryItem {
    s: Session,
    tx: Tx,
    c: ClientInfo,
}

pub struct DefaultShared {
    lockers: DashMap<ClientId, Arc<Mutex<()>>>,
    peers: DashMap<ClientId, EntryItem>,
}

impl DefaultShared {
    #[inline]
    pub fn instance() -> &'static DefaultShared {
        static INSTANCE: OnceCell<DefaultShared> = OnceCell::new();
        INSTANCE.get_or_init(|| Self { lockers: DashMap::default(), peers: DashMap::default() })
    }

    #[inline]
    pub fn tx(&self, client_id: &str) -> Option<(Tx, To)> {
        self.peers.get(client_id).map(|peer| (peer.tx.clone(), peer.c.id.clone()))
    }
}

#[async_trait]
impl Shared for &'static DefaultShared {
    #[inline]
    fn entry(&self, id: Id) -> Box<dyn Entry> {
        Box::new(LockEntry::new(id, self, None))
    }

    #[inline]
    async fn forwards(&self, from: From, publish: Publish) -> Result<(), Vec<(To, From, Publish, Reason)>> {
        let mut errs = Vec::new();
        match publish {
            Publish::V3(publish) => {
                let router = Runtime::instance().extends.router().await;
                let (mut relations, _other_relations) = router.matches(&publish.topic).await;

                for (topic_filter, client_id, qos) in relations.drain(..) {
                    let mut p = publish.clone();
                    p.packet.dup = false;
                    p.packet.retain = false;
                    p.packet.qos = p.packet.qos.less_value(qos);
                    p.packet.packet_id = None;
                    let (tx, to) = if let Some((tx, to)) = self.tx(&client_id) {
                        (tx, to)
                    } else {
                        log::warn!(
                            "forwards, from:{:?}, to:{:?}, topic_filter:{:?}, topic:{:?}, error: Tx is None",
                            from,
                            client_id,
                            topic_filter,
                            publish.topic
                        );
                        errs.push((
                            To::from(0, client_id),
                            from.clone(),
                            Publish::V3(p),
                            Reason::from_static("Tx is None"),
                        ));
                        continue;
                    };

                    if let Err(e) = tx.send(Message::Forward(from.clone(), Publish::V3(p))) {
                        log::warn!(
                            "forwards,  from:{:?}, to:{:?}, topic_filter:{:?}, topic:{:?}, error:{:?}",
                            from,
                            client_id,
                            topic_filter,
                            publish.topic,
                            e
                        );
                        if let Message::Forward(from, p) = e.0 {
                            errs.push((to, from, p, Reason::from_static("Connection Tx is closed")));
                        }
                    }
                }

                for (_node_id, _topic_filters) in _other_relations {
                    //@TODO send topic_filters to other node(node_id), In cluster mode
                }
            }
            Publish::V5(_publish) => {
                log::warn!("Not implemented");
            }
        }

        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs)
        }
    }

    #[inline]
    async fn clients(&self) -> usize {
        self.peers.iter().filter(|entry| entry.value().c.is_connected()).count()
    }

    #[inline]
    async fn sessions(&self) -> usize {
        self.peers.len()
    }

    #[inline]
    fn iter(&self) -> Box<dyn Iterator<Item = Box<dyn Entry>> + Sync + Send> {
        Box::new(DefaultIter { shared: self, ptr: self.peers.iter() })
    }

    #[inline]
    fn random_session(&self) -> Option<(Session, ClientInfo)> {
        let mut sessions =
            self.peers.iter().map(|p| (p.s.clone(), p.c.clone())).collect::<Vec<(Session, ClientInfo)>>();
        let len = self.peers.len();
        if len > 0 {
            let idx = rand::prelude::random::<usize>() % len;
            Some(sessions.remove(idx))
        } else {
            None
        }
    }
}

pub struct DefaultIter<'a> {
    shared: &'static DefaultShared,
    ptr: dashmap::iter::Iter<'a, ClientId, EntryItem, ahash::RandomState>,
}

impl Iterator for DefaultIter<'_> {
    type Item = Box<dyn Entry>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.ptr.next() {
            Some(Box::new(LockEntry::new(item.c.id.clone(), self.shared, None)))
        } else {
            None
        }
    }
}

pub struct DefaultRouter {
    tree: RwLock<TopicTree<NodeId>>,
    relations: DashMap<TopicFilter, DashMap<ClientId, QoS>>,
}

type Relation = (TopicFilter, ClientId, QoS);

impl DefaultRouter {
    #[inline]
    pub fn instance() -> &'static DefaultRouter {
        static INSTANCE: OnceCell<DefaultRouter> = OnceCell::new();
        INSTANCE
            .get_or_init(|| Self { tree: RwLock::new(TopicTree::default()), relations: DashMap::default() })
    }

    #[inline]
    pub fn _matches(
        &self,
        topic: &Topic,
        this_node_id: NodeId,
    ) -> (Vec<Relation>, std::collections::HashMap<NodeId, Vec<TopicFilter>>) {
        let mut this_subs: Vec<(TopicFilter, ClientId, QoS)> = Vec::new();
        let mut other_subs: std::collections::HashMap<NodeId, Vec<TopicFilter>> =
            std::collections::HashMap::new();

        for (topic_filter, node_ids) in self.tree.read().matches(topic) {
            for node_id in node_ids {
                if this_node_id == node_id {
                    //Get subscription relationship from local
                    if let Some(entry) = self.relations.get(&topic_filter) {
                        for e in entry.iter() {
                            this_subs.push((topic_filter.clone(), e.key().to_owned(), *e.value()));
                        }
                    }
                } else {
                    other_subs.entry(node_id).or_default().push(topic_filter.clone());
                }
            }
        }

        (this_subs, other_subs)
    }
}

#[async_trait]
impl Router for &'static DefaultRouter {
    #[inline]
    async fn add(
        &self,
        topic_filter: &TopicFilter,
        node_id: NodeId,
        client_id: &str,
        qos: QoS,
    ) -> Result<()> {
        self.tree.write().insert(topic_filter, node_id); //@TODO Or send the routing relationship to the cluster ...

        //Storage subscription to local
        self.relations.entry(topic_filter.to_owned()).or_default().insert(ClientId::from(client_id), qos);

        Ok(())
    }

    #[inline]
    async fn remove(&self, topic_filter: &TopicFilter, node_id: NodeId, client_id: &str) -> Result<()> {
        //Remove subscription relationship from local
        let relations_len = self
            .relations
            .get(topic_filter)
            .map(|entry| {
                entry.remove(client_id);
                entry.len()
            })
            .unwrap_or_default();

        if relations_len == 0 {
            //Remove routing information from the routing table when there is no subscription relationship
            self.tree.write().remove(topic_filter, &node_id);
            //@TODO Remove routing relationship from cluster ...
        }

        Ok(())
    }

    #[inline]
    async fn matches(
        &self,
        topic: &Topic,
    ) -> (Vec<(TopicFilter, ClientId, QoS)>, std::collections::HashMap<NodeId, Vec<TopicFilter>>) {
        self._matches(topic, Runtime::instance().node.id())
    }

    #[inline]
    fn list(&self, top: usize) -> Vec<String> {
        self.tree.read().list(top)
    }
}

pub struct DefaultRetainStorage {
    messages: RwLock<RetainTree<Retain>>,
}

impl DefaultRetainStorage {
    #[inline]
    pub fn instance() -> &'static DefaultRetainStorage {
        static INSTANCE: OnceCell<DefaultRetainStorage> = OnceCell::new();
        INSTANCE.get_or_init(|| Self { messages: RwLock::new(RetainTree::default()) })
    }

    #[inline]
    pub fn add(&self, topic: &Topic, retain: Retain) {
        self.messages.write().insert(topic, retain);
    }

    #[inline]
    pub fn remove(&self, topic: &Topic) {
        self.messages.write().remove(topic);
    }
}

#[async_trait]
impl RetainStorage for &'static DefaultRetainStorage {
    #[inline]
    async fn set(&self, topic: &Topic, retain: Retain) -> Result<()> {
        self.remove(topic);
        if !retain.publish.is_empty() {
            self.add(topic, retain);
        }
        Ok(())
    }

    #[inline]
    async fn get(&self, topic_filter: &Topic) -> Result<Vec<(Topic, Retain)>> {
        Ok(self.messages.write().matches(topic_filter))
    }
}

pub struct DefaultFitterManager {}

impl DefaultFitterManager {
    #[inline]
    pub fn instance() -> &'static DefaultFitterManager {
        static INSTANCE: OnceCell<DefaultFitterManager> = OnceCell::new();
        INSTANCE.get_or_init(|| Self {})
    }
}

impl FitterManager for &'static DefaultFitterManager {
    #[inline]
    fn get(&self, id: Id, listen_cfg: Listener) -> std::rc::Rc<dyn Fitter> {
        std::rc::Rc::new(DefaultFitter::new(id, listen_cfg))
    }
}

#[derive(Clone)]
pub struct DefaultFitter {
    listen_cfg: Listener,
}

impl DefaultFitter {
    #[inline]
    pub fn new(_id: Id, listen_cfg: Listener) -> Self {
        Self { listen_cfg }
    }
}

#[async_trait]
impl Fitter for DefaultFitter {
    #[inline]
    fn keep_alive(&self, keep_alive: u16) -> Result<u16> {
        if keep_alive < self.listen_cfg.min_keepalive {
            return Err(Error::msg(format!(
                "Keepalive is too small, cannot be less than {}",
                self.listen_cfg.min_keepalive
            )));
        }
        Ok(((keep_alive as f32 * self.listen_cfg.keepalive_backoff) * 2.0) as u16)
    }

    #[inline]
    fn max_mqueue_len(&self) -> usize {
        self.listen_cfg.max_mqueue_len
    }

    #[inline]
    fn mqueue_rate_limit(&self) -> (NonZeroU32, Duration) {
        self.listen_cfg.mqueue_rate_limit
    }
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

//#[derive(Clone)]
pub struct DefaultHookManager {
    #[allow(clippy::type_complexity)]
    handlers: Arc<DashMap<Type, Arc<sync::RwLock<LinkedMap<HandlerId, HookEntry>>>>>,
}

impl DefaultHookManager {
    #[inline]
    pub fn instance() -> &'static DefaultHookManager {
        static INSTANCE: OnceCell<DefaultHookManager> = OnceCell::new();
        INSTANCE.get_or_init(|| Self { handlers: Arc::new(DashMap::default()) })
    }

    #[inline]
    async fn add(&self, typ: Type, handler: Box<dyn Handler>) -> Result<HandlerId> {
        let id = Uuid::new_v4().to_simple().encode_lower(&mut Uuid::encode_buffer()).to_string();
        let type_handlers =
            self.handlers.entry(typ).or_insert(Arc::new(sync::RwLock::new(LinkedMap::default())));
        let mut type_handlers = type_handlers.write().await;
        if type_handlers.contains_key(&id) {
            Err(Error::msg(format!("handler id is repetition, id is {}, type is {:?}", id, typ)))
        } else {
            type_handlers.insert(id.clone(), HookEntry::new(handler));
            Ok(id)
        }
    }

    #[inline]
    async fn exec<'a>(&'a self, t: Type, p: Parameter<'a>) -> Option<HookResult> {
        let mut acc = None;
        let type_handlers = { self.handlers.get(&t).map(|h| (*h.value()).clone()) };
        if let Some(type_handlers) = type_handlers {
            let type_handlers = type_handlers.read().await;
            for (_, entry) in type_handlers.iter() {
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
impl HookManager for &'static DefaultHookManager {
    #[inline]
    fn hook(&self, s: &Session, c: &ClientInfo) -> std::rc::Rc<dyn Hook> {
        std::rc::Rc::new(DefaultHook::new(self, s, c))
    }

    #[inline]
    fn register(&self) -> Box<dyn Register> {
        Box::new(DefaultHookRegister::new(self))
    }

    #[inline]
    async fn before_startup(&self) {
        self.exec(Type::BeforeStartup, Parameter::BeforeStartup).await;
    }

    #[inline]
    async fn client_connect(&self, connect_info: &ConnectInfo) -> Option<UserProperties> {
        let result = self.exec(Type::ClientConnect, Parameter::ClientConnect(connect_info)).await;
        if let Some(HookResult::UserProperties(props)) = result {
            Some(props)
        } else {
            None
        }
    }

    ///When sending mqtt:: connectack message
    async fn client_connack(
        &self,
        connect_info: &ConnectInfo,
        return_code: ConnectAckReason,
    ) -> ConnectAckReason {
        let result =
            self.exec(Type::ClientConnack, Parameter::ClientConnack(connect_info, &return_code)).await;
        if let Some(HookResult::ConnectAckReason(new_return_code)) = result {
            new_return_code
        } else {
            return_code
        }
    }

    ///Publish message Dropped
    async fn message_dropped(&self, to: Option<To>, from: From, publish: Publish, reason: Reason) {
        let _ = self.exec(Type::MessageDropped, Parameter::MessageDropped(to, from, publish, reason)).await;
    }

    ///grpc message received
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
    manager: &'static DefaultHookManager,
    type_ids: Arc<DashSet<(Type, HandlerId)>>,
}

impl DefaultHookRegister {
    #[inline]
    fn new(manager: &'static DefaultHookManager) -> Self {
        DefaultHookRegister { manager, type_ids: Arc::new(DashSet::default()) }
    }

    #[inline]
    async fn adjust_status(&self, b: bool) {
        for type_id in self.type_ids.iter() {
            let (typ, id) = type_id.key();
            if let Some(type_handlers) = self.manager.handlers.get(typ) {
                if let Some(entry) = type_handlers.write().await.get_mut(id) {
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
    async fn add(&self, typ: Type, handler: Box<dyn Handler>) {
        match self.manager.add(typ, handler).await {
            Ok(id) => {
                self.type_ids.insert((typ, id));
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
    manager: &'static DefaultHookManager,
    s: Session,
    c: ClientInfo,
}

impl DefaultHook {
    #[inline]
    pub fn new(manager: &'static DefaultHookManager, s: &Session, c: &ClientInfo) -> Self {
        Self { manager, s: s.clone(), c: c.clone() }
    }
}

#[async_trait]
impl Hook for DefaultHook {
    #[inline]
    async fn session_created(&self) {
        self.manager.exec(Type::SessionCreated, Parameter::SessionCreated(&self.s, &self.c)).await;
    }

    #[inline]
    async fn client_authenticate(&self, password: Option<Password>) -> ConnectAckReason {
        let ok = || match self.c.protocol() {
            MQTT_LEVEL_31 => ConnectAckReason::V3(ConnectAckReasonV3::ConnectionAccepted),
            MQTT_LEVEL_311 => ConnectAckReason::V3(ConnectAckReasonV3::ConnectionAccepted),
            MQTT_LEVEL_5 => ConnectAckReason::V5(ConnectAckReasonV5::Success),
            _ => ConnectAckReason::V3(ConnectAckReasonV3::ConnectionAccepted),
        };

        if self.s.listen_cfg.allow_anonymous {
            return ok();
        }
        let result = self
            .manager
            .exec(Type::ClientAuthenticate, Parameter::ClientAuthenticate(&self.s, &self.c, password))
            .await;

        let (bad_user_or_pass, not_auth) = match result {
            Some(HookResult::AuthResult(AuthResult::BadUsernameOrPassword)) => (true, false),
            Some(HookResult::AuthResult(AuthResult::NotAuthorized)) => (false, true),
            _ => (false, false),
        };

        if bad_user_or_pass {
            return match self.c.protocol() {
                MQTT_LEVEL_31 => ConnectAckReason::V3(ConnectAckReasonV3::BadUserNameOrPassword),
                MQTT_LEVEL_311 => ConnectAckReason::V3(ConnectAckReasonV3::BadUserNameOrPassword),
                MQTT_LEVEL_5 => ConnectAckReason::V5(ConnectAckReasonV5::BadUserNameOrPassword),
                _ => ConnectAckReason::V3(ConnectAckReasonV3::BadUserNameOrPassword),
            };
        }

        if not_auth {
            return match self.c.protocol() {
                MQTT_LEVEL_31 => ConnectAckReason::V3(ConnectAckReasonV3::NotAuthorized),
                MQTT_LEVEL_311 => ConnectAckReason::V3(ConnectAckReasonV3::NotAuthorized),
                MQTT_LEVEL_5 => ConnectAckReason::V5(ConnectAckReasonV5::NotAuthorized),
                _ => ConnectAckReason::V3(ConnectAckReasonV3::NotAuthorized),
            };
        }

        ok()
    }

    #[inline]
    async fn client_connected(&self) {
        let _ = self.manager.exec(Type::ClientConnected, Parameter::ClientConnected(&self.s, &self.c)).await;
    }

    #[inline]
    async fn client_disconnected(&self, r: Reason) {
        let _ = self
            .manager
            .exec(Type::ClientDisconnected, Parameter::ClientDisconnected(&self.s, &self.c, r))
            .await;
    }

    #[inline]
    async fn session_terminated(&self, r: Reason) {
        let _ = self
            .manager
            .exec(Type::SessionTerminated, Parameter::SessionTerminated(&self.s, &self.c, r))
            .await;
    }

    #[inline]
    async fn client_subscribe_check_acl(&self, subscribe: &Subscribe) -> Option<SubscribeAclResult> {
        let result = self
            .manager
            .exec(
                Type::ClientSubscribeCheckAcl,
                Parameter::ClientSubscribeCheckAcl(&self.s, &self.c, subscribe),
            )
            .await;

        if let Some(HookResult::SubscribeAclResult(acl_result)) = result {
            Some(acl_result)
        } else {
            None
        }
    }

    #[inline]
    async fn message_publish_check_acl(&self, publish: &Publish) -> PublishAclResult {
        let result = self
            .manager
            .exec(Type::MessagePublishCheckAcl, Parameter::MessagePublishCheckAcl(&self.s, &self.c, publish))
            .await;
        if let Some(HookResult::PublishAclResult(acl_result)) = result {
            acl_result
        } else {
            PublishAclResult::Allow
        }
    }

    #[inline]
    async fn client_subscribe(&self, subscribe: &Subscribe) -> Option<TopicFilters> {
        let result = self
            .manager
            .exec(Type::ClientSubscribe, Parameter::ClientSubscribe(&self.s, &self.c, subscribe))
            .await;
        if let Some(HookResult::TopicFilters(topic_filters)) = result {
            Some(topic_filters)
        } else {
            None
        }
    }

    #[inline]
    async fn session_subscribed(&self, subscribed: Subscribed) {
        let _ = self
            .manager
            .exec(Type::SessionSubscribed, Parameter::SessionSubscribed(&self.s, &self.c, subscribed))
            .await;
    }

    #[inline]
    async fn client_unsubscribe(&self, unsubscribe: &Unsubscribe) -> Option<TopicFilters> {
        let result = self
            .manager
            .exec(Type::ClientUnsubscribe, Parameter::ClientUnsubscribe(&self.s, &self.c, unsubscribe))
            .await;

        if let Some(HookResult::TopicFilters(topic_filters)) = result {
            Some(topic_filters)
        } else {
            None
        }
    }

    #[inline]
    async fn session_unsubscribed(&self, unsubscribed: Unsubscribed) {
        let _ = self
            .manager
            .exec(Type::SessionUnsubscribed, Parameter::SessionUnsubscribed(&self.s, &self.c, unsubscribed))
            .await;
    }

    #[inline]
    async fn message_publish(&self, publish: &Publish) -> Option<Publish> {
        let result = self
            .manager
            .exec(Type::MessagePublish, Parameter::MessagePublish(&self.s, &self.c, publish))
            .await;

        if let Some(HookResult::Publish(publish)) = result {
            Some(publish)
        } else {
            None
        }
    }

    #[inline]
    async fn message_delivered(&self, from: From, publish: &Publish) -> Option<Publish> {
        let result = self
            .manager
            .exec(Type::MessageDelivered, Parameter::MessageDelivered(&self.s, &self.c, from, publish))
            .await;

        if let Some(HookResult::Publish(publish)) = result {
            Some(publish)
        } else {
            None
        }
    }

    #[inline]
    async fn message_acked(&self, from: From, publish: &Publish) {
        let _ = self
            .manager
            .exec(Type::MessageAcked, Parameter::MessageAcked(&self.s, &self.c, from, publish))
            .await;
    }

    // #[inline]
    // async fn message_dropped(&self, to: Option<To>, from: From, publish: Publish, reason: Reason) {
    //     let _ = self
    //         .manager
    //         .exec(
    //             Type::MessageDropped,
    //             Parameter::MessageDropped(&self.s, &self.c, to, from, publish, reason),
    //         )
    //         .await;
    // }

    #[inline]
    async fn message_expiry_check(&self, from: From, publish: &Publish) -> MessageExpiry {
        let result = self
            .manager
            .exec(Type::MessageExpiryCheck, Parameter::MessageExpiryCheck(&self.s, &self.c, from, publish))
            .await;

        if let Some(HookResult::MessageExpiry) = result {
            return true;
        }

        let expiry_interval = self.s.listen_cfg.message_expiry_interval.as_millis() as i64;
        if expiry_interval == 0 {
            return false;
        }
        if (chrono::Local::now().timestamp_millis() - publish.create_time()) < expiry_interval {
            return false;
        }
        true
    }
}

pub struct DefaultLimiterManager {
    limiters: DashMap<String, DefaultLimiter>,
}

impl DefaultLimiterManager {
    #[inline]
    pub fn instance() -> &'static DefaultLimiterManager {
        static INSTANCE: OnceCell<DefaultLimiterManager> = OnceCell::new();
        INSTANCE.get_or_init(|| Self { limiters: DashMap::default() })
    }
}

impl LimiterManager for &'static DefaultLimiterManager {
    #[inline]
    fn get(&self, name: String, listen_cfg: Listener) -> Result<Box<dyn Limiter>> {
        let l = self
            .limiters
            .entry(name.clone())
            .or_insert(DefaultLimiter::new(name, listen_cfg.conn_await_acquire, listen_cfg.max_conn_rate)?)
            .value()
            .clone();
        Ok(Box::new(l))
    }
}

#[derive(Clone)]
pub struct DefaultLimiter {
    name: String,
    await_acquire: bool,
    limiter: Arc<LeakyBucket>,
}

impl DefaultLimiter {
    #[inline]
    pub fn new(name: String, await_acquire: bool, permits: usize) -> Result<Self> {
        Ok(Self {
            name,
            await_acquire,
            limiter: Arc::new(
                LeakyBucket::builder()
                    .max(permits)
                    .tokens(permits)
                    .refill_interval(Duration::from_secs(1))
                    .refill_amount(permits)
                    .build()?,
            ),
        })
    }
}

#[async_trait]
impl Limiter for DefaultLimiter {
    #[inline]
    async fn acquire_one(&self) -> Result<()> {
        self.acquire(1).await?;
        Ok(())
    }

    #[inline]
    async fn acquire(&self, amount: usize) -> Result<()> {
        if !self.await_acquire && self.limiter.tokens() < amount {
            return Err(Error::msg("not enough tokens"));
        }
        self.limiter.acquire(amount).await?;
        Ok(())
    }
}
