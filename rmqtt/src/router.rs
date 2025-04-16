//! MQTT Topic Routing & Subscription Management Core
//!
//! Provides distributed topic-based routing implementation with hierarchical subscription management
//! for MQTT brokers. The system combines trie-based pattern matching with concurrent data structures
//! to achieve O(log n) topic operations while maintaining thread safety across cluster nodes.
//!
//! ## Core Features
//! 1. **Topic Pattern Management**:
//!    - Trie-based storage with wildcard support (+/#) using `TopicTree`
//!    - Atomic subscription counters with `Counter` struct
//!    - Distributed subscription synchronization via `AllRelationsMap`
//!
//! 2. **Subscription Lifecycle**:
//!    - Add/remove operations protected by `RwLock`
//!    - Cross-node online status checking
//!    - Shared subscription support through conditional compilation
//!
//! 3. **Data Structures**:
//!    - `Arc<DashMap>` for thread-safe subscription relations storage
//!    - Hierarchical topic matching with `VecToTopic` conversions
//!    - JSON serialization for admin interfaces
//!
//! ## Design Characteristics
//! - **Concurrency Model**:
//!   - Lock hierarchy: Topic tree uses `RwLock` while relations use lock-free `DashMap`
//!   - Atomic counters for subscription statistics
//!   - Async-compatible through `#[async_trait]`
//!
//! - **Cluster Coordination**:
//!   - Node-specific subscription merging with `merge_topics/merge_routes`
//!   - Online status integration with server context
//!   - Paginated query support for large datasets
//!
//! Typical usage includes:
//! - Routing QoS 1/2 messages with session persistence
//! - Handling shared subscriptions with round-robin selection
//! - Performing cluster-wide subscription audits
//!
//! The implementation leverages Rust's type system to ensure:
//! - Zero-cost abstraction for topic matching
//! - Memory safety through ownership model
//! - Linear scalability with subscription count

use std::convert::From as _;
use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use itertools::Itertools;
use serde_json::json;
use tokio::sync::RwLock;

use crate::context::ServerContext;
use crate::trie::{TopicTree, VecToTopic};
use crate::types::*;
use crate::utils::Counter;
use crate::Result;

#[async_trait]
pub trait Router: Sync + Send {
    /// Id add with topic filter
    async fn add(&self, topic_filter: &str, id: Id, opts: SubscriptionOptions) -> Result<()>;

    /// Remove with id topic filter
    async fn remove(&self, topic_filter: &str, id: Id) -> Result<bool>;

    /// Match with id and topic
    async fn matches(&self, id: Id, topic: &TopicName) -> Result<SubRelationsMap>;

    ///Check online or offline
    async fn is_online(&self, node_id: NodeId, client_id: &str) -> bool;

    /// Gets by limit
    async fn gets(&self, limit: usize) -> Vec<Route>;

    /// Get by topic
    async fn get(&self, topic: &str) -> Result<Vec<Route>>;

    ///query subscriptions
    async fn query_subscriptions(&self, q: &SubsSearchParams) -> Vec<SubsSearchResult>;

    /// Topics tree
    async fn topics_tree(&self) -> usize;

    ///Return number of subscribed topics
    fn topics(&self) -> Counter;

    ///Returns the number of Subscription relationship
    fn routes(&self) -> Counter;

    /// merge Topics with another
    fn merge_topics(&self, topics_map: &HashMap<NodeId, Counter>) -> Counter;

    /// merge routes with another
    fn merge_routes(&self, routes_map: &HashMap<NodeId, Counter>) -> Counter;

    ///get topic tree
    async fn list_topics(&self, top: usize) -> Vec<String>;

    ///get subscription relations
    async fn list_relations(&self, top: usize) -> Vec<serde_json::Value>;

    ///all relations
    fn relations(&self) -> &AllRelationsMap;
}

#[allow(clippy::type_complexity)]
#[derive(Clone)]
pub struct DefaultRouter {
    scx: Option<ServerContext>,
    pub topics: Arc<RwLock<TopicTree<()>>>,
    pub topics_count: Arc<Counter>,
    pub relations: Arc<AllRelationsMap>,
    pub relations_count: Arc<Counter>,
}

impl DefaultRouter {
    #[inline]
    pub fn new(scx: Option<ServerContext>) -> DefaultRouter {
        Self {
            scx,
            topics: Arc::new(RwLock::new(TopicTree::default())),
            topics_count: Arc::new(Counter::new()),
            relations: Arc::new(DashMap::default()),
            relations_count: Arc::new(Counter::new()),
        }
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
    pub async fn _has_matches(&self, topic: &str) -> Result<bool> {
        let topic = Topic::from_str(topic)?;
        Ok(self.topics.read().await.is_match(&topic))
    }

    #[inline]
    pub async fn _get_routes(&self, topic: &str) -> Result<Vec<Route>> {
        let topic = Topic::from_str(topic)?;
        let node_id = self.context().node.id();
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
        // let scx = self.context();
        let mut collector_map: SubscriptioRelationsCollectorMap = HashMap::default();
        let topic = Topic::from_str(topic_name)?;
        for (topic_filter, _node_ids) in self.topics.read().await.matches(&topic).iter() {
            let topic_filter = topic_filter.to_topic_filter();

            #[allow(clippy::mutable_key_type)]
            #[cfg(feature = "shared-subscription")]
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
                    #[cfg(feature = "shared-subscription")]
                    {
                        if let Some(group) = opts.shared_group() {
                            let router = self.context().extends.router().await;
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
                    #[cfg(not(feature = "shared-subscription"))]
                    {
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
            #[cfg(feature = "shared-subscription")]
            for (group, mut s_subs) in groups.drain() {
                log::debug!("group: {}, s_subs: {:?}", group, s_subs);
                let group_cids = s_subs.iter().map(|(_, cid, _, _, _)| cid.clone()).collect();
                if let Some((idx, is_online)) =
                    self.context().extends.shared_subscription().await.choice(self.context(), &s_subs).await
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

        #[cfg(feature = "shared-subscription")]
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
        self.relations
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
            .unwrap_or_default()
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
impl Router for DefaultRouter {
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
    async fn is_online(&self, node_id: NodeId, client_id: &str) -> bool {
        self.context()
            .extends
            .shared()
            .await
            .entry(Id::from(node_id, ClientId::from(client_id)))
            .is_connected()
            .await
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
    async fn query_subscriptions(&self, q: &SubsSearchParams) -> Vec<SubsSearchResult> {
        self._query_subscriptions(q).await
    }

    #[inline]
    async fn topics_tree(&self) -> usize {
        self.topics.read().await.values_size()
    }

    #[inline]
    fn topics(&self) -> Counter {
        self.topics_count.as_ref().clone()
    }

    #[inline]
    fn routes(&self) -> Counter {
        self.relations_count.as_ref().clone()
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

    #[inline]
    fn relations(&self) -> &AllRelationsMap {
        &self.relations
    }
}
