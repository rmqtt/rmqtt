use anyhow::anyhow;
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::convert::From as _f;
use std::iter::Iterator;
use std::num::NonZeroU16;
use std::num::NonZeroU32;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[allow(unused_imports)]
use bitflags::Flags;
use itertools::Itertools;
use ntex_mqtt::types::{MQTT_LEVEL_31, MQTT_LEVEL_311, MQTT_LEVEL_5};
use ntex_mqtt::TopicLevel;
use once_cell::sync::OnceCell;
use rust_box::task_exec_queue::{Builder, SpawnExt, TaskExecQueue};
use tokio::sync::oneshot;
use tokio::sync::RwLock;
use tokio::sync::{self, Mutex, OwnedMutexGuard};
use tokio::time::{sleep, Duration};
use uuid::Uuid;

use crate::broker::fitter::{Fitter, FitterManager};
use crate::broker::hook::{Handler, Hook, HookManager, HookResult, Parameter, Priority, Register, Type};
use crate::broker::session::{Session, SessionLike, SessionManager, SessionOfflineInfo};
use crate::broker::topic::{Topic, VecToTopic};
use crate::broker::types::*;
use crate::broker::MessageManager;
use crate::settings::listener::Listener;
use crate::stats::Counter;
use crate::{grpc, ClientId, Id, MqttError, NodeId, Result, Runtime, TopicFilter};

use super::{
    retain::RetainTree, topic::TopicTree, Entry, IsOnline, RetainStorage, Router, Shared, SharedSubscription,
    SubRelations, SubRelationsMap,
};

type DashSet<V> = dashmap::DashSet<V, ahash::RandomState>;
type DashMap<K, V> = dashmap::DashMap<K, V, ahash::RandomState>;
type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

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
    async fn _unsubscribe(&self, id: Id, topic_filter: &str) -> Result<()> {
        Runtime::instance().extends.router().await.remove(topic_filter, id).await?;
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
impl super::Entry for LockEntry {
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
        Ok(Box::new(LockEntry::new(self.id.clone(), self.shared, Some(locker))))
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
    ) -> Result<Option<SessionOfflineInfo>> {
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
                        return Err(MqttError::Msg(format!("recv kick result is {:?}", e)));
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
                Ok(None)
            } else {
                Ok(Some(s.to_offline_info().await?))
            }
        } else {
            Ok(None)
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
        let peer = self
            .shared
            .peers
            .get(&self.id.client_id)
            .map(|peer| peer.value().clone())
            .ok_or_else(|| MqttError::from("session is not exist"))?;

        let this_node_id = Runtime::instance().node.id();
        let node_id = peer.s.id.node_id;
        assert_eq!(
            node_id, this_node_id,
            "session node exception, session node id is {}, this node id is {}",
            node_id, this_node_id
        );

        Runtime::instance()
            .extends
            .router()
            .await
            .add(&sub.topic_filter, self.id.clone(), sub.opts.clone())
            .await?;
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
            .ok_or_else(|| MqttError::from("session is not exist"))?;

        if let Err(e) =
            Runtime::instance().extends.router().await.remove(&unsubscribe.topic_filter, self.id()).await
        {
            log::warn!("{:?} unsubscribe, error:{:?}", self.id, e);
        }
        let remove_ok = peer.s.subscriptions_remove(&unsubscribe.topic_filter).await?.is_some();
        Ok(remove_ok)
    }

    #[inline]
    async fn publish(&self, from: From, p: Publish) -> Result<(), (From, Publish, Reason)> {
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
        self.peers.get(client_id).map(|peer| (peer.tx.clone(), peer.s.id.clone()))
    }

    #[inline]
    pub async fn _query_subscriptions(&self, q: &SubsSearchParams) -> Vec<SubsSearchResult> {
        DefaultRouter::instance()._query_subscriptions(q).await
    }

    #[inline]
    pub fn _collect_subscription_client_ids(&self, relations_map: &SubRelationsMap) -> SubscriptionClientIds {
        let sub_client_ids = relations_map
            .values()
            .map(|subs| {
                subs.iter().map(|(tf, cid, _, _, group_shared)| {
                    log::debug!(
                        "_collect_subscription_client_ids, tf: {:?}, cid {:?}, group_shared: {:?}",
                        tf,
                        cid,
                        group_shared
                    );
                    if let Some((group, _, cids)) = group_shared {
                        cids.iter()
                            .map(|g_cid| {
                                if g_cid == cid {
                                    log::debug!(
                                        "_collect_subscription_client_ids is group_shared {:?}",
                                        g_cid
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
            .flatten()
            .flatten()
            //.sorted()
            //.dedup()
            .collect::<Vec<_>>();
        log::debug!("_collect_subscription_client_ids sub_client_ids: {:?}", sub_client_ids);
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
        log::debug!("_merge_subscription_client_ids sub_client_ids: {:?}", sub_client_ids);
        if let Some(sub_client_ids) = sub_client_ids {
            if sub_client_ids.is_empty() {
                None
            } else {
                Some(sub_client_ids)
            }
        } else {
            None
        }
    }
}

#[async_trait]
impl Shared for &'static DefaultShared {
    #[inline]
    fn entry(&self, id: Id) -> Box<dyn Entry> {
        Box::new(LockEntry::new(id, self, None))
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
    ) -> Result<SubscriptionClientIds, Vec<(To, From, Publish, Reason)>> {
        let topic = publish.topic();
        let mut relations_map = match Runtime::instance()
            .extends
            .router()
            .await
            .matches(from.id.clone(), publish.topic())
            .await
        {
            Ok(relations_map) => relations_map,
            Err(e) => {
                log::warn!("forwards, from:{:?}, topic:{:?}, error: {:?}", from, topic, e);
                SubRelationsMap::default()
            }
        };

        let sub_client_ids = self._collect_subscription_client_ids(&relations_map);

        let this_node_id = Runtime::instance().node.id();
        if let Some(relations) = relations_map.remove(&this_node_id) {
            self.forwards_to(from, &publish, relations).await?;
        }
        if !relations_map.is_empty() {
            log::warn!("forwards, relations_map:{:?}", relations_map);
        }
        Ok(sub_client_ids)
    }

    #[inline]
    async fn forwards_and_get_shareds(
        &self,
        from: From,
        publish: Publish,
    ) -> Result<(SubRelationsMap, SubscriptionClientIds), Vec<(To, From, Publish, Reason)>> {
        let topic = publish.topic();
        log::debug!("forwards_and_get_shareds, from: {:?}, topic: {:?}", from, topic.to_string());
        let relations_map =
            match Runtime::instance().extends.router().await.matches(from.id.clone(), topic).await {
                Ok(relations_map) => relations_map,
                Err(e) => {
                    log::warn!("forwards, from:{:?}, topic:{:?}, error: {:?}", from, topic, e);
                    SubRelationsMap::default()
                }
            };

        //let subs_size: SubscriptionSize = relations_map.values().map(|subs| subs.len()).sum();
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

    #[inline]
    async fn forwards_to(
        &self,
        from: From,
        publish: &Publish,
        mut relations: SubRelations,
    ) -> Result<(), Vec<(To, From, Publish, Reason)>> {
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
            p.properties.subscription_ids = sub_ids;
            let (tx, to) = if let Some((tx, to)) = self.tx(&client_id) {
                (tx, to)
            } else {
                log::warn!(
                    "forwards_to, from:{:?}, to:{:?}, topic_filter:{:?}, topic:{:?}, error: Tx is None",
                    from,
                    client_id,
                    topic_filter,
                    publish.topic
                );
                errs.push((To::from(0, client_id), from.clone(), p, Reason::from_static("Tx is None")));
                continue;
            };

            if let Err(e) = tx.unbounded_send(Message::Forward(from.clone(), p)) {
                log::warn!(
                    "forwards_to,  from:{:?}, to:{:?}, topic_filter:{:?}, topic:{:?}, error:{:?}",
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
    fn iter(&self) -> Box<dyn Iterator<Item = Box<dyn Entry>> + Sync + Send> {
        Box::new(DefaultIter { shared: self, ptr: self.peers.iter() })
    }

    #[inline]
    fn random_session(&self) -> Option<Session> {
        let mut sessions = self.peers.iter().map(|p| (p.s.clone())).collect::<Vec<Session>>();
        let len = self.peers.len();
        if len > 0 {
            let idx = rand::prelude::random::<usize>() % len;
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
    async fn query_subscriptions(&self, q: SubsSearchParams) -> Vec<SubsSearchResult> {
        self._query_subscriptions(&q).await
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
            Some(Box::new(LockEntry::new(item.s.id.clone(), self.shared, None)))
        } else {
            None
        }
    }
}

#[allow(clippy::type_complexity)]
pub struct DefaultRouter {
    pub topics: RwLock<TopicTree<()>>,
    pub topics_count: Counter,
    pub relations: DashMap<TopicFilter, HashMap<ClientId, (Id, SubscriptionOptions)>>,
    pub relations_count: Counter,
}

impl DefaultRouter {
    #[inline]
    pub fn instance() -> &'static DefaultRouter {
        static INSTANCE: OnceCell<DefaultRouter> = OnceCell::new();
        INSTANCE.get_or_init(|| Self {
            topics: RwLock::new(TopicTree::default()),
            topics_count: Counter::new(),
            relations: DashMap::default(),
            relations_count: Counter::new(),
        })
    }

    #[inline]
    pub async fn _has_matches(&self, topic: &str) -> Result<bool> {
        let topic = Topic::from_str(topic)?;
        Ok(self.topics.read().await.is_match(&topic))
    }

    #[inline]
    pub async fn _get_routes(&self, topic: &str) -> Result<Vec<Route>> {
        let topic = Topic::from_str(topic)?;
        let node_id = Runtime::instance().node.id();
        let routes = self
            .topics
            .read()
            .await
            .matches(&topic)
            .iter()
            .unique()
            .map(|(topic_filter, _)| Route { node_id, topic: topic_filter.to_topic_filter() })
            .collect::<Vec<_>>();
        Ok(routes)
    }

    #[allow(clippy::type_complexity)]
    #[inline]
    pub async fn _matches(&self, this_id: Id, topic_name: &TopicName) -> Result<SubRelationsMap> {
        let mut collector_map: SubscriptioRelationsCollectorMap = HashMap::default();
        let topic = Topic::from_str(topic_name)?;
        for (topic_filter, _node_ids) in self.topics.read().await.matches(&topic).iter() {
            let topic_filter = topic_filter.to_topic_filter();

            let mut groups: HashMap<
                SharedGroup,
                Vec<(
                    NodeId,
                    ClientId,
                    SubscriptionOptions,
                    Option<Vec<SubscriptionIdentifier>>,
                    Option<IsOnline>,
                )>,
            > = HashMap::default();

            if let Some(rels) = self.relations.get(&topic_filter) {
                for (client_id, (id, opts)) in rels.iter() {
                    if let Some(no_local) = opts.no_local() {
                        //MQTT V5: No Local
                        if no_local && &this_id == id {
                            continue;
                        }
                    }
                    if let Some(group) = opts.shared_group() {
                        let router = Runtime::instance().extends.router().await;
                        groups.entry(group.clone()).or_default().push((
                            id.node_id,
                            client_id.clone(),
                            opts.clone(),
                            None,
                            Some(router.is_online(id.node_id, client_id).await),
                        ));
                    } else {
                        collector_map.entry(id.node_id).or_default().add(
                            &topic_filter,
                            client_id.clone(),
                            opts.clone(),
                            None,
                        );
                    }
                }
            }

            //select a subscriber from shared subscribe groups
            for (group, mut s_subs) in groups.drain() {
                log::debug!("group: {}, s_subs: {:?}", group, s_subs);
                let group_cids = s_subs.iter().map(|(_, cid, _, _, _)| cid.clone()).collect();
                if let Some((idx, is_online)) =
                    Runtime::instance().extends.shared_subscription().await.choice(&s_subs).await
                {
                    let (node_id, client_id, opts, _, _) = s_subs.remove(idx);
                    collector_map.entry(node_id).or_default().add(
                        &topic_filter,
                        client_id,
                        opts,
                        Some((group, is_online, group_cids)),
                    );
                }
            }
        }

        let mut rels_map: SubRelationsMap = HashMap::default();
        for (node_id, collector) in collector_map {
            rels_map.insert(node_id, collector.into());
        }

        log::debug!("{:?} this_subs: {:?}", topic_name, rels_map);
        Ok(rels_map)
    }

    #[inline]
    fn _query_subscriptions_filter(
        q: &SubsSearchParams,
        client_id: &str,
        opts: &SubscriptionOptions,
    ) -> bool {
        if let Some(ref q_clientid) = q.clientid {
            if q_clientid.as_bytes() != client_id.as_bytes() {
                return false;
            }
        }

        if let Some(ref q_qos) = q.qos {
            if *q_qos != opts.qos_value() {
                return false;
            }
        }

        match (&q.share, opts.shared_group()) {
            (Some(q_group), Some(group)) => {
                if q_group != group {
                    return false;
                }
            }
            (Some(_), None) => return false,
            _ => {}
        }

        true
    }

    #[inline]
    async fn _query_subscriptions_for_topic(
        &self,
        q: &SubsSearchParams,
        topic: &str,
    ) -> Vec<SubsSearchResult> {
        let limit = q._limit;
        let mut curr: usize = 0;
        let topic_filter = TopicFilter::from(topic);
        return self
            .relations
            .get(topic)
            .map(|e| {
                e.value()
                    .iter()
                    .filter(|(client_id, (_id, opts))| {
                        Self::_query_subscriptions_filter(q, client_id.as_ref(), opts)
                    })
                    .filter_map(|(client_id, (id, opts))| {
                        if curr < limit {
                            curr += 1;
                            Some(SubsSearchResult {
                                node_id: id.node_id,
                                clientid: client_id.clone(),
                                client_addr: id.remote_addr,
                                topic: topic_filter.clone(),
                                opts: opts.clone(),
                            })
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
    }

    #[inline]
    async fn _query_subscriptions_for_matches(
        &self,
        q: &SubsSearchParams,
        _match_topic: &str,
    ) -> Vec<SubsSearchResult> {
        let topic = if let Ok(t) = Topic::from_str(_match_topic) {
            t
        } else {
            return Vec::new();
        };

        let limit = q._limit;
        let mut curr: usize = 0;

        self.topics
            .read()
            .await
            .matches(&topic)
            .iter()
            .unique()
            .flat_map(|(topic_filter, _)| {
                let topic_filter = topic_filter.to_topic_filter();
                if let Some(entry) = self.relations.get(&topic_filter) {
                    entry
                        .iter()
                        .filter(|(client_id, (_id, opts))| {
                            Self::_query_subscriptions_filter(q, client_id.as_ref(), opts)
                        })
                        .filter_map(|(client_id, (id, opts))| {
                            if curr < limit {
                                curr += 1;
                                Some(SubsSearchResult {
                                    node_id: id.node_id,
                                    clientid: client_id.clone(),
                                    client_addr: id.remote_addr,
                                    topic: topic_filter.clone(),
                                    opts: opts.clone(),
                                })
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                }
            })
            .collect::<Vec<_>>()
    }

    #[inline]
    async fn _query_subscriptions_for_other(&self, q: &SubsSearchParams) -> Vec<SubsSearchResult> {
        let limit = q._limit;
        let mut curr: usize = 0;
        self.relations
            .iter()
            .flat_map(|e| {
                let topic_filter = e.key();
                e.value()
                    .iter()
                    .filter(|(client_id, (_id, opts))| {
                        Self::_query_subscriptions_filter(q, client_id.as_ref(), opts)
                    })
                    .filter_map(|(client_id, (id, opts))| {
                        if curr < limit {
                            curr += 1;
                            Some(SubsSearchResult {
                                node_id: id.node_id,
                                clientid: client_id.clone(),
                                client_addr: id.remote_addr,
                                topic: topic_filter.clone(),
                                opts: opts.clone(),
                            })
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    }

    #[inline]
    async fn _query_subscriptions(&self, q: &SubsSearchParams) -> Vec<SubsSearchResult> {
        //topic
        if let Some(ref topic) = q.topic {
            return self._query_subscriptions_for_topic(q, topic).await;
        }

        //_match_topic
        if let Some(ref _match_topic) = q._match_topic {
            return self._query_subscriptions_for_matches(q, _match_topic).await;
        }

        //other
        self._query_subscriptions_for_other(q).await
    }
}

#[async_trait]
impl Router for &'static DefaultRouter {
    #[inline]
    async fn add(&self, topic_filter: &str, id: Id, opts: SubscriptionOptions) -> Result<()> {
        log::debug!("{:?} add, topic_filter: {:?}", id, topic_filter);
        let topic = Topic::from_str(topic_filter)?;
        //add to topic tree
        self.topics.write().await.insert(&topic, ());

        //add to subscribe relations
        let old = self
            .relations
            .entry(TopicFilter::from(topic_filter))
            .or_insert_with(|| {
                self.topics_count.inc();
                HashMap::default()
            })
            .insert(id.client_id.clone(), (id, opts));

        if old.is_none() {
            self.relations_count.inc();
        }

        Ok(())
    }

    #[inline]
    async fn remove(&self, topic_filter: &str, id: Id) -> Result<bool> {
        log::debug!("{:?} remove, topic_filter: {:?}", id, topic_filter);
        //Remove subscription relationship from local
        let res = if let Some(mut rels) = self.relations.get_mut(topic_filter) {
            let remove_enable = rels.value().get(&id.client_id).map(|(s_id, _)| {
                if *s_id != id {
                    log::debug!("remove, input id not the same, input id: {:?}, current id: {:?}, topic_filter: {}", id, s_id, topic_filter);
                    false
                } else {
                    true
                }
            }).unwrap_or(false);
            if remove_enable {
                let remove_ok = rels.value_mut().remove(&id.client_id).is_some();
                if remove_ok {
                    self.relations_count.dec();
                }
                Some((rels.is_empty(), remove_ok))
            } else {
                None
            }
        } else {
            None
        };

        log::debug!("{:?} remove, topic_filter: {:?}, res: {:?}", id, topic_filter, res);

        let remove_ok = if let Some((is_empty, remove_ok)) = res {
            if is_empty {
                if self.relations.remove(topic_filter).is_some() {
                    self.topics_count.dec();
                }
                let topic = Topic::from_str(topic_filter)?;
                self.topics.write().await.remove(&topic, &());
            }
            remove_ok
        } else {
            false
        };
        Ok(remove_ok)
    }

    #[inline]
    async fn matches(&self, id: Id, topic: &TopicName) -> Result<SubRelationsMap> {
        Ok(self._matches(id, topic).await?)
    }

    #[inline]
    async fn gets(&self, limit: usize) -> Vec<Route> {
        let mut curr: usize = 0;
        self.relations
            .iter()
            .flat_map(|e| {
                let topic_filter = e.key();
                e.value()
                    .iter()
                    .map(|(_, (id, _))| (id.node_id, topic_filter))
                    .unique()
                    .filter_map(|(node_id, topic_filter)| {
                        if curr < limit {
                            curr += 1;
                            Some(Route { node_id, topic: topic_filter.clone() })
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    }

    #[inline]
    async fn get(&self, topic: &str) -> Result<Vec<Route>> {
        let topic = Topic::from_str(topic)?;
        let routes = self
            .topics
            .read()
            .await
            .matches(&topic)
            .iter()
            .unique()
            .flat_map(|(topic_filter, _)| {
                let topic_filter = topic_filter.to_topic_filter();
                if let Some(entry) = self.relations.get(&topic_filter) {
                    entry
                        .iter()
                        .map(|(_, (id, _))| id.node_id)
                        .unique()
                        .map(|node_id| Route { node_id, topic: topic_filter.clone() })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                }
            })
            .collect::<Vec<_>>();
        Ok(routes)
    }

    #[inline]
    async fn topics_tree(&self) -> usize {
        self.topics.read().await.values_size()
    }

    #[inline]
    fn topics(&self) -> Counter {
        self.topics_count.clone()
    }

    #[inline]
    fn routes(&self) -> Counter {
        self.relations_count.clone()
    }

    #[inline]
    fn merge_topics(&self, topics_map: &HashMap<NodeId, Counter>) -> Counter {
        let topics = Counter::new();
        for (i, (_, counter)) in topics_map.iter().enumerate() {
            if i == 0 {
                topics.set(counter);
            } else {
                topics.count_min(counter.count());
                topics.max_max(counter.max())
            }
        }
        topics
    }

    #[inline]
    fn merge_routes(&self, routes_map: &HashMap<NodeId, Counter>) -> Counter {
        let routes = Counter::new();
        for (i, (_, counter)) in routes_map.iter().enumerate() {
            if i == 0 {
                routes.set(counter);
            } else {
                routes.count_min(counter.count());
                routes.max_max(counter.max())
            }
        }
        routes
    }

    #[inline]
    async fn list_topics(&self, top: usize) -> Vec<String> {
        self.topics.read().await.list(top)
    }

    #[inline]
    async fn list_relations(&self, top: usize) -> Vec<serde_json::Value> {
        let mut count = 0;
        let mut rels = Vec::new();
        for entry in self.relations.iter() {
            let topic_filter = entry.key();
            for (client_id, (id, opts)) in entry.iter() {
                let item = json!({
                    "topic_filter": topic_filter,
                    "client_id": client_id,
                    "node_id": id.node_id,
                    "opts": opts.to_json(),
                });
                rels.push(item);
                count += 1;
                if count >= top {
                    return rels;
                }
            }
        }
        rels
    }
}

pub struct DefaultSharedSubscription {}

impl DefaultSharedSubscription {
    #[inline]
    pub fn instance() -> &'static DefaultSharedSubscription {
        static INSTANCE: OnceCell<DefaultSharedSubscription> = OnceCell::new();
        INSTANCE.get_or_init(|| Self {})
    }
}

#[async_trait]
impl SharedSubscription for &'static DefaultSharedSubscription {}

pub struct DefaultRetainStorage {
    messages: RwLock<RetainTree<TimedValue<Retain>>>,
}

impl DefaultRetainStorage {
    #[inline]
    pub fn instance() -> &'static DefaultRetainStorage {
        static INSTANCE: OnceCell<DefaultRetainStorage> = OnceCell::new();
        INSTANCE.get_or_init(|| Self { messages: RwLock::new(RetainTree::default()) })
    }

    #[inline]
    pub async fn remove_expired_messages(&self) {
        let mut messages = self.messages.write().await;
        messages.retain(|tv| {
            if tv.is_expired() {
                Runtime::instance().stats.retaineds.dec();
                false
            } else {
                true
            }
        });
    }

    #[inline]
    pub async fn set_with_timeout(
        &self,
        topic: &TopicName,
        retain: Retain,
        timeout: Option<Duration>,
    ) -> Result<()> {
        let topic = Topic::from_str(topic)?;
        let mut messages = self.messages.write().await;
        let old = messages.remove(&topic);
        if !retain.publish.is_empty() {
            messages.insert(&topic, TimedValue::new(retain, timeout));
            if old.is_none() {
                Runtime::instance().stats.retaineds.inc();
            }
        } else if old.is_some() {
            Runtime::instance().stats.retaineds.dec();
        }
        Ok(())
    }
}

#[async_trait]
impl RetainStorage for &'static DefaultRetainStorage {
    #[inline]
    async fn set(&self, topic: &TopicName, retain: Retain) -> Result<()> {
        self.set_with_timeout(topic, retain, None).await
    }

    #[inline]
    async fn get(&self, topic_filter: &TopicFilter) -> Result<Vec<(TopicName, Retain)>> {
        let topic = Topic::from_str(topic_filter)?;
        let retains = self
            .messages
            .read()
            .await
            .matches(&topic)
            .drain(..)
            .filter_map(|(t, r)| {
                if r.is_expired() {
                    None
                } else {
                    Some((TopicName::from(t.to_string()), r.into_value()))
                }
            })
            .collect::<Vec<(TopicName, Retain)>>();
        Ok(retains)
    }

    #[inline]
    fn count(&self) -> isize {
        Runtime::instance().stats.retaineds.count()
    }

    #[inline]
    fn max(&self) -> isize {
        Runtime::instance().stats.retaineds.max()
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
    fn create(&self, conn_info: Arc<ConnectInfo>, id: Id, listen_cfg: Listener) -> FitterType {
        Arc::new(DefaultFitter::new(conn_info, id, listen_cfg))
    }
}

#[derive(Clone)]
pub struct DefaultFitter {
    conn_info: Arc<ConnectInfo>,
    listen_cfg: Listener,
}

impl DefaultFitter {
    #[inline]
    pub fn new(conn_info: Arc<ConnectInfo>, _id: Id, listen_cfg: Listener) -> Self {
        Self { conn_info, listen_cfg }
    }
}

#[async_trait]
impl Fitter for DefaultFitter {
    #[inline]
    fn keep_alive(&self, keep_alive: &mut u16) -> Result<u16> {
        if self.conn_info.proto_ver() == MQTT_LEVEL_5 {
            if *keep_alive == 0 {
                return if self.listen_cfg.allow_zero_keepalive {
                    Ok(0)
                } else {
                    Err(MqttError::from("Keepalive must be greater than 0"))
                };
            } else if *keep_alive < self.listen_cfg.min_keepalive {
                *keep_alive = self.listen_cfg.min_keepalive;
            } else if *keep_alive > self.listen_cfg.max_keepalive {
                *keep_alive = self.listen_cfg.max_keepalive;
            }
        } else if *keep_alive == 0 {
            return if self.listen_cfg.allow_zero_keepalive {
                Ok(0)
            } else {
                Err(MqttError::from("Keepalive must be greater than 0"))
            };
        } else if *keep_alive < self.listen_cfg.min_keepalive {
            return Err(MqttError::from(format!(
                "Keepalive is too small and cannot be less than {}",
                self.listen_cfg.min_keepalive
            )));
        } else if *keep_alive > self.listen_cfg.max_keepalive {
            return Err(MqttError::from(format!(
                "Keepalive is too large and cannot be greater than {}",
                self.listen_cfg.max_keepalive
            )));
        }

        if *keep_alive < 6 {
            Ok(*keep_alive + 3)
        } else {
            Ok(((*keep_alive as f32 * self.listen_cfg.keepalive_backoff) * 2.0) as u16)
        }
    }

    #[inline]
    fn max_mqueue_len(&self) -> usize {
        self.listen_cfg.max_mqueue_len
    }

    #[inline]
    fn mqueue_rate_limit(&self) -> (NonZeroU32, Duration) {
        self.listen_cfg.mqueue_rate_limit
    }

    #[inline]
    fn max_inflight(&self) -> NonZeroU16 {
        let receive_max = if let ConnectInfo::V5(_, connect) = self.conn_info.as_ref() {
            connect.receive_max
        } else {
            None
        };

        if let Some(receive_max) = receive_max {
            self.listen_cfg.max_inflight.min(receive_max)
        } else {
            self.listen_cfg.max_inflight
        }
    }

    #[inline]
    async fn session_expiry_interval(&self, d: Option<&Disconnect>) -> Duration {
        let expiry_interval = || {
            if let ConnectInfo::V5(_, connect) = self.conn_info.as_ref() {
                Duration::from_secs(connect.session_expiry_interval_secs.unwrap_or_default() as u64)
            } else {
                self.listen_cfg.session_expiry_interval
            }
        };

        if let Some(Disconnect::V5(d)) = d {
            if let Some(interval_secs) = d.session_expiry_interval_secs {
                Duration::from_secs(interval_secs as u64)
            } else {
                expiry_interval()
            }
        } else {
            expiry_interval()
        }
    }

    #[inline]
    fn message_expiry_interval(&self, publish: &Publish) -> Duration {
        let expiry_interval = publish
            .properties
            .message_expiry_interval
            .map(|i| Duration::from_secs(i.get() as u64))
            .unwrap_or_else(|| self.listen_cfg.message_expiry_interval);
        log::debug!("{:?} message_expiry_interval: {:?}", self.conn_info.id(), expiry_interval);
        expiry_interval
    }

    #[inline]
    fn max_client_topic_aliases(&self) -> u16 {
        if let ConnectInfo::V5(_, _connect) = self.conn_info.as_ref() {
            self.listen_cfg.max_topic_aliases
        } else {
            0
        }
    }

    #[inline]
    fn max_server_topic_aliases(&self) -> u16 {
        if let ConnectInfo::V5(_, connect) = self.conn_info.as_ref() {
            connect.topic_alias_max.min(self.listen_cfg.max_topic_aliases)
        } else {
            0
        }
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
    handlers: Arc<DashMap<Type, Arc<sync::RwLock<BTreeMap<(Priority, HandlerId), HookEntry>>>>>,
}

impl DefaultHookManager {
    #[inline]
    pub fn instance() -> &'static DefaultHookManager {
        static INSTANCE: OnceCell<DefaultHookManager> = OnceCell::new();
        INSTANCE.get_or_init(|| Self { handlers: Arc::new(DashMap::default()) })
    }

    #[inline]
    async fn add(&self, typ: Type, priority: Priority, handler: Box<dyn Handler>) -> Result<HandlerId> {
        let id = Uuid::new_v4().as_simple().encode_lower(&mut Uuid::encode_buffer()).to_string();
        let type_handlers =
            self.handlers.entry(typ).or_insert(Arc::new(sync::RwLock::new(BTreeMap::default())));
        let mut type_handlers = type_handlers.write().await;
        let key = (priority, id.clone());
        let contains_key = type_handlers.contains_key(&key);
        if contains_key {
            Err(MqttError::from(format!("handler id is repetition, key is {:?}, type is {:?}", key, typ)))
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
impl HookManager for &'static DefaultHookManager {
    #[inline]
    fn hook(&self, s: &Session) -> std::rc::Rc<dyn Hook> {
        std::rc::Rc::new(DefaultHook::new(self, s))
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
    ) -> (ConnectAckReason, Superuser) {
        let proto_ver = connect_info.proto_ver();
        let ok = || match proto_ver {
            MQTT_LEVEL_31 => ConnectAckReason::V3(ConnectAckReasonV3::ConnectionAccepted),
            MQTT_LEVEL_311 => ConnectAckReason::V3(ConnectAckReasonV3::ConnectionAccepted),
            MQTT_LEVEL_5 => ConnectAckReason::V5(ConnectAckReasonV5::Success),
            _ => ConnectAckReason::V3(ConnectAckReasonV3::ConnectionAccepted),
        };

        log::debug!("{:?} username: {:?}", connect_info.id(), connect_info.username());
        if connect_info.username().is_none() && allow_anonymous {
            return (ok(), false);
        }

        let result = self.exec(Type::ClientAuthenticate, Parameter::ClientAuthenticate(connect_info)).await;
        log::debug!("{:?} result: {:?}", connect_info.id(), result);
        let (bad_user_or_pass, not_auth) = match result {
            Some(HookResult::AuthResult(AuthResult::BadUsernameOrPassword)) => (true, false),
            Some(HookResult::AuthResult(AuthResult::NotAuthorized)) => (false, true),
            Some(HookResult::AuthResult(AuthResult::Allow(superuser))) => return (ok(), superuser),
            _ => {
                //or AuthResult::NotFound
                if allow_anonymous {
                    return (ok(), false);
                } else {
                    (false, true)
                }
            }
        };

        if bad_user_or_pass {
            return (
                match proto_ver {
                    MQTT_LEVEL_31 => ConnectAckReason::V3(ConnectAckReasonV3::BadUserNameOrPassword),
                    MQTT_LEVEL_311 => ConnectAckReason::V3(ConnectAckReasonV3::BadUserNameOrPassword),
                    MQTT_LEVEL_5 => ConnectAckReason::V5(ConnectAckReasonV5::BadUserNameOrPassword),
                    _ => ConnectAckReason::V3(ConnectAckReasonV3::BadUserNameOrPassword),
                },
                false,
            );
        }

        if not_auth {
            return (
                match proto_ver {
                    MQTT_LEVEL_31 => ConnectAckReason::V3(ConnectAckReasonV3::NotAuthorized),
                    MQTT_LEVEL_311 => ConnectAckReason::V3(ConnectAckReasonV3::NotAuthorized),
                    MQTT_LEVEL_5 => ConnectAckReason::V5(ConnectAckReasonV5::NotAuthorized),
                    _ => ConnectAckReason::V3(ConnectAckReasonV3::NotAuthorized),
                },
                false,
            );
        }

        (ok(), false)
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
    async fn message_dropped(&self, to: Option<To>, from: From, publish: Publish, reason: Reason) {
        let _ = self.exec(Type::MessageDropped, Parameter::MessageDropped(to, from, publish, reason)).await;
    }

    ///Publish message nonsubscribed
    async fn message_nonsubscribed(&self, from: From) {
        let _ = self.exec(Type::MessageNonsubscribed, Parameter::MessageNonsubscribed(from)).await;
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
    type_ids: Arc<DashSet<(Type, (Priority, HandlerId))>>,
}

impl DefaultHookRegister {
    #[inline]
    fn new(manager: &'static DefaultHookManager) -> Self {
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
    manager: &'static DefaultHookManager,
    s: Session,
}

impl DefaultHook {
    #[inline]
    pub fn new(manager: &'static DefaultHookManager, s: &Session) -> Self {
        Self { manager, s: s.clone() }
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
        if let Some(HookResult::TopicFilter(tf)) = reply {
            tf
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

        if let Some(HookResult::TopicFilter(topic_filter)) = reply {
            topic_filter
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
            .message_expiry_interval
            .map(|i| (i.get() * 1000) as i64)
            .unwrap_or_else(|| self.s.listen_cfg().message_expiry_interval.as_millis() as i64);
        log::debug!("{:?} expiry_interval: {:?}", self.s.id, expiry_interval);
        if expiry_interval == 0 {
            return MessageExpiryCheckResult::Remaining(None);
        }
        let remaining = chrono::Local::now().timestamp_millis() - publish.create_time();
        if remaining < expiry_interval {
            return MessageExpiryCheckResult::Remaining(NonZeroU32::new(
                ((expiry_interval - remaining) / 1000) as u32,
            ));
        }
        MessageExpiryCheckResult::Expiry
    }
}

pub struct DefaultSessionManager {}

impl DefaultSessionManager {
    #[inline]
    pub fn instance() -> &'static DefaultSessionManager {
        static INSTANCE: OnceCell<DefaultSessionManager> = OnceCell::new();
        INSTANCE.get_or_init(|| Self {})
    }
}

impl SessionManager for &'static DefaultSessionManager {
    #[allow(clippy::too_many_arguments)]
    fn create(
        &self,
        id: Id,
        listen_cfg: Listener,
        subscriptions: SessionSubs,
        deliver_queue: MessageQueueType,
        inflight_win: InflightType,
        conn_info: ConnectInfoType,

        created_at: TimestampMillis,
        connected_at: TimestampMillis,
        session_present: bool,
        superuser: bool,
        connected: bool,
        disconnect_info: Option<DisconnectInfo>,
    ) -> Arc<dyn SessionLike> {
        Arc::new(DefaultSession::new(
            id,
            listen_cfg,
            subscriptions,
            deliver_queue,
            inflight_win,
            conn_info,
            created_at,
            connected_at,
            session_present,
            superuser,
            connected,
            disconnect_info,
        ))
    }
}

pub struct DefaultSession {
    id: Id,
    listen_cfg: Listener,
    subscriptions: SessionSubs,
    deliver_queue: MessageQueueType,
    inflight_win: InflightType,
    conn_info: ConnectInfoType,

    created_at: TimestampMillis,
    state_flags: SessionStateFlags,
    connected_at: TimestampMillis,

    disconnect_info: RwLock<DisconnectInfo>,
}

impl DefaultSession {
    #[allow(clippy::too_many_arguments)]
    fn new(
        id: Id,
        listen_cfg: Listener,
        subscriptions: SessionSubs,
        deliver_queue: MessageQueueType,
        inflight_win: InflightType,
        conn_info: ConnectInfoType,

        created_at: TimestampMillis,
        connected_at: TimestampMillis,
        session_present: bool,
        superuser: bool,
        connected: bool,
        disconnect_info: Option<DisconnectInfo>,
    ) -> Self {
        let mut state_flags = SessionStateFlags::empty();
        if session_present {
            state_flags.insert(SessionStateFlags::SessionPresent);
        }
        if superuser {
            state_flags.insert(SessionStateFlags::Superuser);
        }
        if connected {
            state_flags.insert(SessionStateFlags::Connected);
        }
        let disconnect_info = disconnect_info.unwrap_or_default();
        Self {
            id,
            listen_cfg,
            subscriptions,
            deliver_queue,
            inflight_win,
            conn_info,

            created_at,
            state_flags,
            connected_at,

            disconnect_info: RwLock::new(disconnect_info),
        }
    }
}

#[async_trait]
impl SessionLike for DefaultSession {
    #[inline]
    fn listen_cfg(&self) -> &Listener {
        &self.listen_cfg
    }

    #[inline]
    fn deliver_queue(&self) -> &MessageQueueType {
        &self.deliver_queue
    }

    #[inline]
    fn inflight_win(&self) -> &InflightType {
        &self.inflight_win
    }

    #[inline]
    async fn subscriptions(&self) -> Result<SessionSubs> {
        Ok(self.subscriptions.clone())
    }

    #[inline]
    async fn subscriptions_add(
        &self,
        topic_filter: TopicFilter,
        opts: SubscriptionOptions,
    ) -> Result<Option<SubscriptionOptions>> {
        Ok(self.subscriptions._add(topic_filter, opts).await)
    }

    #[inline]
    async fn subscriptions_remove(
        &self,
        topic_filter: &str,
    ) -> Result<Option<(TopicFilter, SubscriptionOptions)>> {
        Ok(self.subscriptions._remove(topic_filter).await)
    }

    #[inline]
    async fn subscriptions_drain(&self) -> Result<Subscriptions> {
        Ok(self.subscriptions._drain().await)
    }

    #[inline]
    async fn subscriptions_extend(&self, other: Subscriptions) -> Result<()> {
        self.subscriptions._extend(other).await;
        Ok(())
    }

    #[inline]
    async fn created_at(&self) -> Result<TimestampMillis> {
        Ok(self.created_at)
    }

    #[inline]
    async fn session_present(&self) -> Result<bool> {
        Ok(self.state_flags.contains(SessionStateFlags::SessionPresent))
    }

    async fn connect_info(&self) -> Result<Arc<ConnectInfo>> {
        Ok(self.conn_info.clone())
    }
    fn username(&self) -> Option<&UserName> {
        self.id.username.as_ref()
    }
    fn password(&self) -> Option<&Password> {
        self.conn_info.password()
    }
    async fn protocol(&self) -> Result<u8> {
        Ok(self.conn_info.proto_ver())
    }
    async fn superuser(&self) -> Result<bool> {
        Ok(self.state_flags.contains(SessionStateFlags::Superuser))
    }
    async fn connected(&self) -> Result<bool> {
        Ok(self.state_flags.contains(SessionStateFlags::Connected)
            && !self.disconnect_info.read().await.is_disconnected())
    }
    async fn connected_at(&self) -> Result<TimestampMillis> {
        Ok(self.connected_at)
    }
    async fn disconnected_at(&self) -> Result<TimestampMillis> {
        Ok(self.disconnect_info.read().await.disconnected_at)
    }
    async fn disconnected_reasons(&self) -> Result<Vec<Reason>> {
        Ok(self.disconnect_info.read().await.reasons.clone())
    }
    async fn disconnected_reason(&self) -> Result<Reason> {
        Ok(Reason::Reasons(self.disconnect_info.read().await.reasons.clone()))
    }
    async fn disconnected_reason_has(&self) -> bool {
        !self.disconnect_info.read().await.reasons.is_empty()
    }
    async fn disconnect(&self) -> Result<Option<Disconnect>> {
        Ok(self.disconnect_info.read().await.mqtt_disconnect.clone())
    }
    async fn disconnected_set(&self, d: Option<Disconnect>, reason: Option<Reason>) -> Result<()> {
        let mut disconnect_info = self.disconnect_info.write().await;

        if !disconnect_info.is_disconnected() {
            disconnect_info.disconnected_at = chrono::Local::now().timestamp_millis();
        }

        if let Some(d) = d {
            disconnect_info.reasons.push(d.reason());
            disconnect_info.mqtt_disconnect.replace(d);
        }

        if let Some(reason) = reason {
            disconnect_info.reasons.push(reason);
        }

        Ok(())
    }
    async fn disconnected_reason_add(&self, r: Reason) -> Result<()> {
        self.disconnect_info.write().await.reasons.push(r);
        Ok(())
    }
    async fn disconnected_reason_take(&self) -> Result<Reason> {
        Ok(Reason::Reasons(self.disconnect_info.write().await.reasons.drain(..).collect()))
    }

    #[inline]
    async fn on_drop(&self) -> Result<()> {
        self.subscriptions._clear().await;
        Ok(())
    }
}

pub struct DefaultMessageManager {
    inner: Arc<DefaultMessageManagerInner>,
    exec: TaskExecQueue,
}

#[derive(Default)]
struct DefaultMessageManagerInner {
    messages: scc::HashMap<PMsgID, PersistedMsg>,
    subs_tree: RwLock<RetainTree<PMsgID>>,
    forwardeds: scc::HashMap<PMsgID, BTreeMap<ClientId, Option<(TopicFilter, SharedGroup)>>>,
    expiries: RwLock<BinaryHeap<(Reverse<Timestamp>, PMsgID)>>,
    id_gen: AtomicUsize,
}

impl DefaultMessageManager {
    #[inline]
    pub fn instance() -> &'static DefaultMessageManager {
        static INSTANCE: OnceCell<DefaultMessageManager> = OnceCell::new();
        INSTANCE.get_or_init(|| {
            let (exec, task_runner) = Builder::default().workers(1000).queue_max(300_000).build();

            tokio::spawn(async move {
                futures::future::join(task_runner, async move {
                    loop {
                        sleep(Duration::from_secs(30)).await;
                        if let Err(e) = Self::instance().remove_expired_messages().await {
                            log::warn!("Cleanup expired messages failed! {:?}", e);
                        }
                    }
                })
                .await;
            });

            Self { inner: Arc::new(DefaultMessageManagerInner::default()), exec }
        })
    }

    #[inline]
    fn next_id(&self) -> usize {
        self.inner.id_gen.fetch_add(1, Ordering::SeqCst)
    }

    #[inline]
    async fn remove_expired_messages(&self) -> Result<()> {
        let now = timestamp_secs();
        let inner = self.inner.as_ref();
        let mut expiries = inner.expiries.write().await;
        let mut subs_tree = inner.subs_tree.write().await;
        while let Some((expiry_time, _)) = expiries.peek() {
            let expiry_time = expiry_time.0;
            if expiry_time > now {
                break;
            }
            if let Some((_, msg_id)) = expiries.pop() {
                if let Some((_, pmsg)) = inner.messages.remove_async(&msg_id).await {
                    let mut topic =
                        Topic::from_str(&pmsg.publish.topic).map_err(|e| anyhow!(format!("{:?}", e)))?;
                    topic.push(TopicLevel::Normal(msg_id.to_string()));
                    let _ = subs_tree.remove(&topic);
                    inner.forwardeds.remove(&msg_id);
                }
            }
        }
        Ok(())
    }

    #[inline]
    async fn _set(
        &self,
        from: From,
        publish: Publish,
        expiry_interval: Duration,
        msg_id: PMsgID,
    ) -> Result<()> {
        let mut topic = Topic::from_str(&publish.topic).map_err(|e| anyhow!(format!("{:?}", e)))?;
        let expiry_time = timestamp_secs() + expiry_interval.as_secs() as i64;
        let inner = &self.inner;
        let pmsg = PersistedMsg { msg_id, from, publish };
        topic.push(TopicLevel::Normal(msg_id.to_string()));
        inner.messages.insert_async(msg_id, pmsg).await.map_err(|_| anyhow!("messages insert error"))?;
        inner.subs_tree.write().await.insert(&topic, msg_id);
        inner.expiries.write().await.push((Reverse(expiry_time), msg_id));
        Ok(())
    }

    #[allow(dead_code)]
    async fn sprint_status(&self) -> String {
        let inner = self.inner.as_ref();
        let (vals_size, nodes_size) = {
            let subs_tree = inner.subs_tree.read().await;
            (subs_tree.values_size(), subs_tree.nodes_size())
        };
        format!(
            "vals_size: {}, nodes_size: {}, messages.len(): {}, expiries.len(): {}, \
         forwardeds.len(): {}, id_gen: {}, waittings: {}, active_count: {}, rate: {:?}",
            vals_size,
            nodes_size,
            inner.messages.len(),
            inner.expiries.read().await.len(),
            inner.forwardeds.len(),
            inner.id_gen.load(Ordering::Relaxed),
            self.exec.waiting_count(),
            self.exec.active_count(),
            self.exec.rate().await
        )
    }
}

#[async_trait]
impl MessageManager for &'static DefaultMessageManager {
    #[inline]
    async fn count(&self) -> isize {
        self.inner.messages.len() as isize
    }

    #[inline]
    async fn max(&self) -> isize {
        self.exec.completed_count().await
    }

    #[inline]
    async fn set(&self, from: From, p: Publish, expiry_interval: Duration) -> Result<PMsgID> {
        let msg_id = self.next_id();
        async move {
            if let Err(e) = DefaultMessageManager::instance()._set(from, p, expiry_interval, msg_id).await {
                log::warn!("Persistence of the Publish message failed! {:?}", e);
            }
        }
        .spawn(&self.exec)
        .await
        .map_err(|e| anyhow!(e.to_string()))?;
        Ok(msg_id)
    }

    #[inline]
    async fn get(
        &self,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
    ) -> Result<Vec<(PMsgID, From, Publish)>> {
        let inner = &self.inner;
        let mut topic = Topic::from_str(&topic_filter).map_err(|e| anyhow!(format!("{:?}", e)))?;
        if !topic.levels().last().map(|l| matches!(l, TopicLevel::MultiWildcard)).unwrap_or_default() {
            topic.push(TopicLevel::SingleWildcard);
        }

        let matcheds = {
            inner.subs_tree.read().await.matches(&topic).iter().map(|(_, msg_id)| *msg_id).collect::<Vec<_>>()
        };

        let matcheds = matcheds
            .into_iter()
            .filter_map(|msg_id| {
                if let Some(pmsg) = inner.messages.get(&msg_id) {
                    let mut clientids = self.inner.forwardeds.entry(msg_id).or_default();
                    let is_forwarded = if clientids.get().contains_key(client_id) {
                        true
                    } else {
                        if let Some(group) = group {
                            //Check if subscription is shared
                            clientids
                                .get()
                                .iter()
                                .filter(|(_, g)| {
                                    if let Some((tf, g)) = g.as_ref() {
                                        g == group && tf == topic_filter
                                    } else {
                                        false
                                    }
                                })
                                .next()
                                .is_some()
                        } else {
                            false
                        }
                    };
                    if !is_forwarded {
                        clientids.get_mut().insert(
                            ClientId::from(client_id),
                            group.map(|g| (TopicFilter::from(topic_filter), g.clone())),
                        );
                    }
                    log::debug!("is_forwarded: {}", is_forwarded);
                    if is_forwarded {
                        None
                    } else {
                        let pmsg = pmsg.get();
                        Some((msg_id, pmsg.from.clone(), pmsg.publish.clone()))
                    }
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        Ok(matcheds)
    }

    #[inline]
    fn set_forwardeds(
        &self,
        msg_id: PMsgID,
        sub_client_ids: Vec<(ClientId, Option<(TopicFilter, SharedGroup)>)>,
    ) {
        let mut clientids = self.inner.forwardeds.entry(msg_id).or_default();
        clientids.get_mut().extend(sub_client_ids);
    }
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
pub fn timestamp_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|t| t.as_secs() as i64)
        .unwrap_or_else(|_| chrono::Local::now().timestamp())
}

#[inline]
pub fn timestamp_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|t| t.as_millis() as i64)
        .unwrap_or_else(|_| chrono::Local::now().timestamp_millis())
}

#[test]
fn test_message_manager() {
    let runner = async move {
        let pmsg_mgr = DefaultMessageManager::instance();
        sleep(Duration::from_millis(10)).await;
        let f = From::from_custom(Id::from(1, ClientId::from("test-001")));
        let mut p = Publish {
            dup: false,
            retain: false,
            qos: QoS::try_from(1).unwrap(),
            topic: TopicName::from(""),
            packet_id: Some(NonZeroU16::try_from(1).unwrap()),
            payload: bytes::Bytes::from("test ..."),
            properties: PublishProperties::default(),
            create_time: chrono::Local::now().timestamp_millis(),
        };

        let now = std::time::Instant::now();
        for i in 0..5 {
            p.topic = TopicName::from("/xx/yy/zz");
            pmsg_mgr.set(f.clone(), p.clone(), Duration::from_secs(i + 2)).await.unwrap();
        }

        for i in 0..5 {
            p.topic = TopicName::from("/xx/yy/cc");
            pmsg_mgr.set(f.clone(), p.clone(), Duration::from_secs(i + 2)).await.unwrap();
        }

        for i in 0..5 {
            p.topic = TopicName::from("/xx/yy/");
            pmsg_mgr.set(f.clone(), p.clone(), Duration::from_secs(i + 2)).await.unwrap();
        }

        for i in 0..5 {
            p.topic = TopicName::from("/xx/yy/ee/ff");
            pmsg_mgr.set(f.clone(), p.clone(), Duration::from_secs(i + 2)).await.unwrap();
        }

        for i in 0..5 {
            p.topic = TopicName::from("/foo/yy/ee");
            pmsg_mgr.set(f.clone(), p.clone(), Duration::from_secs(i + 2)).await.unwrap();
        }

        println!("cost time: {:?}", now.elapsed());
        sleep(Duration::from_millis(10)).await;
        println!("{}", pmsg_mgr.sprint_status().await);

        let tf = TopicFilter::from("/xx/yy/#");
        let msgs = pmsg_mgr.get("c-id-001", &tf, None).await.unwrap();
        println!("===>>> msgs len: {}", msgs.len());
        assert_eq!(msgs.len(), 20);
        //for (f, p) in msgs {
        //    println!("> from: {:?}, publish: {:?}", f, p);
        //}

        let tf = TopicFilter::from("/xx/yy/cc");
        let msgs = pmsg_mgr.get("c-id-002", &tf, None).await.unwrap();
        println!("===>>> msgs len: {}", msgs.len());
        assert_eq!(msgs.len(), 5);

        let tf = TopicFilter::from("/foo/yy/ee");
        let msgs = pmsg_mgr.get("", &tf, None).await.unwrap();
        assert_eq!(msgs.len(), 5);
        println!("msgs len: {}", msgs.len());
        //for (f, p) in msgs {
        //    println!("from: {:?}, publish: {:?}", f, p);
        //}

        sleep(Duration::from_millis(1000 * 5)).await;
        println!("{}", pmsg_mgr.sprint_status().await);
    };

    tokio::runtime::Runtime::new().unwrap().block_on(runner);
}
