//! Shared subscription selection strategy implementations.
//!
//! Provides the concrete [`SharedSubscriptionImpl`] with multiple strategies:
//! random, round-robin, per-group round-robin, sticky, local,
//! hash-by-client-id, and hash-by-topic.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use lru::LruCache;
use tokio::sync::Mutex;

use rmqtt::{context::ServerContext, subscribe::SharedSubscription, types::*};

use super::Strategy;

/// Sticky binding: (shared_group, publisher_client_id) -> subscriber_client_id.
/// Uses an LRU cache so stale bindings are automatically evicted.
pub(crate) type StickyMap = Arc<Mutex<LruCache<(SharedGroup, ClientId), ClientId>>>;

/// A concrete implementation of [`SharedSubscription`] that dispatches
/// subscriber selection to the configured strategy.
pub(crate) struct SharedSubscriptionImpl {
    /// Which strategy to use for subscriber selection.
    strategy: Strategy,
    /// Global atomic counter for round_robin.
    pub(crate) rr_counter: Arc<AtomicUsize>,
    /// Per-group counters for round_robin_per_group.
    pub(crate) rr_group_counters: Arc<Mutex<HashMap<SharedGroup, usize>>>,
    /// Sticky bindings: (shared_group_name, publisher_client_id) -> (sub_idx, subscriber_client_id).
    pub(crate) sticky_map: StickyMap,
}

impl SharedSubscriptionImpl {
    /// Creates a new instance with shared internal state.
    pub(crate) fn new(
        strategy: Strategy,
        rr_counter: Arc<AtomicUsize>,
        rr_group_counters: Arc<Mutex<HashMap<SharedGroup, usize>>>,
        sticky_map: StickyMap,
    ) -> Self {
        Self { strategy, rr_counter, rr_group_counters, sticky_map }
    }
}

#[async_trait]
impl SharedSubscription for SharedSubscriptionImpl {
    #[inline]
    fn is_supported(&self, _listen_cfg: &rmqtt::types::ListenerConfig) -> bool {
        true
    }

    #[inline]
    async fn choice(
        &self,
        scx: &ServerContext,
        group: &SharedGroup,
        publisher_id: &Id,
        topic: &TopicName,
        ncs: &[(
            NodeId,
            ClientId,
            SubscriptionOptions,
            Option<Vec<SubscriptionIdentifier>>,
            Option<IsOnline>,
        )],
    ) -> Option<(usize, IsOnline)> {
        if ncs.is_empty() {
            return None;
        }

        match self.strategy {
            Strategy::Random => self.choice_random(scx, publisher_id, ncs).await,
            Strategy::RoundRobin => self.choice_round_robin(scx, publisher_id, ncs).await,
            Strategy::RoundRobinPerGroup => {
                self.choice_round_robin_per_group(scx, publisher_id, group, ncs).await
            }
            Strategy::Sticky => self.choice_sticky(scx, group, publisher_id, ncs).await,
            Strategy::Local => self.choice_local(scx, publisher_id, ncs).await,
            Strategy::HashClientId => self.choice_hash_clientid(scx, publisher_id, ncs).await,
            Strategy::HashTopic => self.choice_hash_topic(scx, publisher_id, topic, ncs).await,
        }
    }
}

/// The subscriber list type alias used across all choice methods.
type Subscribers<'a> =
    &'a [(NodeId, ClientId, SubscriptionOptions, Option<Vec<SubscriptionIdentifier>>, Option<IsOnline>)];

impl SharedSubscriptionImpl {
    /// When no online subscribers are found, prefer a subscriber on the same node
    /// as the publisher to minimize cross-node forwarding.
    fn find_local_fallback(publisher_id: &Id, ncs: Subscribers<'_>) -> Option<(usize, bool)> {
        let pub_node = publisher_id.node_id;
        // First pass: prefer local node subscriber
        for (idx, (node_id, _, _, _, _)) in ncs.iter().enumerate() {
            if *node_id == pub_node {
                return Some((idx, false));
            }
        }
        // Fallback: first subscriber
        if !ncs.is_empty() {
            return Some((0, false));
        }
        None
    }

    /// Random selection
    async fn choice_random(
        &self,
        scx: &ServerContext,
        publisher_id: &Id,
        ncs: Subscribers<'_>,
    ) -> Option<(usize, IsOnline)> {
        let mut tmp_ncs: Vec<(usize, NodeId, &ClientId, &Option<IsOnline>)> = ncs
            .iter()
            .enumerate()
            .map(|(idx, (node_id, client_id, _, _, is_online))| (idx, *node_id, client_id, is_online))
            .collect();

        while !tmp_ncs.is_empty() {
            let r_idx = if tmp_ncs.len() == 1 { 0 } else { (rand::random::<u64>() as usize) % tmp_ncs.len() };
            let (idx, node_id, client_id, is_online) = tmp_ncs.remove(r_idx);
            let is_online = if let Some(is_online) = is_online {
                *is_online
            } else {
                scx.extends.router().await.is_online(node_id, client_id).await
            };
            if is_online {
                return Some((idx, true));
            }
        }

        // No online subscriber: prefer local node, otherwise first
        Self::find_local_fallback(publisher_id, ncs)
    }

    /// Round-robin selection using global atomic counter
    async fn choice_round_robin(
        &self,
        scx: &ServerContext,
        publisher_id: &Id,
        ncs: Subscribers<'_>,
    ) -> Option<(usize, IsOnline)> {
        let len = ncs.len();
        let counter = self.rr_counter.fetch_add(1, Ordering::Relaxed);
        let start_idx = counter % len;

        for offset in 0..len {
            let idx = (start_idx + offset) % len;
            let (node_id, client_id, _, _, is_online) = &ncs[idx];
            let is_online = if let Some(is_online) = is_online {
                *is_online
            } else {
                scx.extends.router().await.is_online(*node_id, client_id).await
            };
            if is_online {
                return Some((idx, true));
            }
        }

        // No online subscriber: prefer local node, otherwise first
        Self::find_local_fallback(publisher_id, ncs)
    }

    /// Per-group round-robin selection
    async fn choice_round_robin_per_group(
        &self,
        scx: &ServerContext,
        publisher_id: &Id,
        group: &SharedGroup,
        ncs: Subscribers<'_>,
    ) -> Option<(usize, IsOnline)> {
        let group_key = group.clone();
        let len = ncs.len();

        let mut counters = self.rr_group_counters.lock().await;
        let counter = counters.entry(group_key).or_insert(0);
        let start_idx = *counter % len;
        *counter = counter.wrapping_add(1);

        for offset in 0..len {
            let idx = (start_idx + offset) % len;
            let (node_id, client_id, _, _, is_online) = &ncs[idx];
            let is_online = if let Some(is_online) = is_online {
                *is_online
            } else {
                scx.extends.router().await.is_online(*node_id, client_id).await
            };
            if is_online {
                return Some((idx, true));
            }
        }

        // No online subscriber: prefer local node, otherwise first
        Self::find_local_fallback(publisher_id, ncs)
    }

    /// Sticky: once a subscriber is chosen for a (group, publisher), keep using it
    async fn choice_sticky(
        &self,
        scx: &ServerContext,
        group: &SharedGroup,
        publisher_id: &Id,
        ncs: Subscribers<'_>,
    ) -> Option<(usize, IsOnline)> {
        let pub_client_id = publisher_id.client_id.clone();
        let group_name = group.clone();
        let key = (group_name, pub_client_id);

        let mut map = self.sticky_map.lock().await;

        // Check if we have a sticky binding
        if let Some(sticky_cid) = map.get(&key) {
            // Find the subscriber by client_id (it may have moved index)
            for (idx, (node_id, client_id, _, _, is_online)) in ncs.iter().enumerate() {
                if client_id == sticky_cid {
                    let is_online = if let Some(is_online) = is_online {
                        *is_online
                    } else {
                        scx.extends.router().await.is_online(*node_id, client_id).await
                    };
                    if is_online {
                        return Some((idx, true));
                    }
                    // Sticky subscriber went offline - remove binding
                    map.pop(&key);
                    break;
                }
            }
        }

        // No valid sticky binding: select a new subscriber
        // Choose first online subscriber
        for (idx, (node_id, client_id, _, _, is_online)) in ncs.iter().enumerate() {
            let is_online = if let Some(is_online) = is_online {
                *is_online
            } else {
                scx.extends.router().await.is_online(*node_id, client_id).await
            };
            if is_online {
                // Create new sticky binding
                map.put(key, client_id.clone());
                return Some((idx, true));
            }
        }

        // No online subscriber: prefer local node, otherwise first
        if let Some((idx, _)) = Self::find_local_fallback(publisher_id, ncs) {
            map.put(key, ncs[idx].1.clone());
            return Some((idx, false));
        }
        None
    }

    /// Local: prefer subscribers on the same node as the publisher
    async fn choice_local(
        &self,
        scx: &ServerContext,
        publisher_id: &Id,
        ncs: Subscribers<'_>,
    ) -> Option<(usize, IsOnline)> {
        let pub_node = publisher_id.node_id;

        // First pass: prefer online subscribers on the publisher's node
        for (idx, (node_id, client_id, _, _, is_online)) in ncs.iter().enumerate() {
            if *node_id == pub_node {
                let is_online = if let Some(is_online) = is_online {
                    *is_online
                } else {
                    scx.extends.router().await.is_online(*node_id, client_id).await
                };
                if is_online {
                    return Some((idx, true));
                }
            }
        }

        // Second pass: any online subscriber (cross-node)
        for (idx, (node_id, client_id, _, _, is_online)) in ncs.iter().enumerate() {
            let is_online = if let Some(is_online) = is_online {
                *is_online
            } else {
                scx.extends.router().await.is_online(*node_id, client_id).await
            };
            if is_online {
                return Some((idx, true));
            }
        }

        // Third pass: local subscriber even if offline
        for (idx, (node_id, _, _, _, _)) in ncs.iter().enumerate() {
            if *node_id == pub_node {
                return Some((idx, false));
            }
        }

        // Last resort: cross-node, possibly offline
        if !ncs.is_empty() {
            return Some((0, false));
        }
        None
    }

    /// Hash by publisher's ClientId
    async fn choice_hash_clientid(
        &self,
        scx: &ServerContext,
        publisher_id: &Id,
        ncs: Subscribers<'_>,
    ) -> Option<(usize, IsOnline)> {
        use std::hash::{Hash, Hasher};

        if ncs.is_empty() {
            return None;
        }

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        publisher_id.client_id.hash(&mut hasher);
        let hash = hasher.finish();
        let idx = (hash as usize) % ncs.len();

        let (node_id, client_id, _, _, is_online) = &ncs[idx];
        let is_online = if let Some(is_online) = is_online {
            *is_online
        } else {
            scx.extends.router().await.is_online(*node_id, client_id).await
        };

        if is_online {
            return Some((idx, true));
        }

        // Hash-chosen subscriber offline: fallback to random online
        self.choice_random(scx, publisher_id, ncs).await
    }

    /// Hash by topic name
    async fn choice_hash_topic(
        &self,
        scx: &ServerContext,
        publisher_id: &Id,
        topic: &TopicName,
        ncs: Subscribers<'_>,
    ) -> Option<(usize, IsOnline)> {
        use std::hash::{Hash, Hasher};

        if ncs.is_empty() {
            return None;
        }

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        topic.hash(&mut hasher);
        let hash = hasher.finish();
        let idx = (hash as usize) % ncs.len();

        let (node_id, client_id, _, _, is_online) = &ncs[idx];
        let is_online = if let Some(is_online) = is_online {
            *is_online
        } else {
            scx.extends.router().await.is_online(*node_id, client_id).await
        };

        if is_online {
            return Some((idx, true));
        }

        // Hash-chosen subscriber offline: fallback to random online
        self.choice_random(scx, publisher_id, ncs).await
    }
}
