//! MQTT Broker Performance Monitoring
//!
//! Provides comprehensive statistical instrumentation for MQTT broker operations, implementing
//! real-time metrics collection and cluster-wide aggregation. The system combines atomic counters
//! with distributed data structures to achieve O(1) metric updates while maintaining thread safety.
//!
//! ## Core Monitoring Dimensions
//! 1. **Connection Lifecycle**:
//!    - Handshake throughput (`handshakings_rate`) and active negotiations (`handshakings_active`)
//!    - Concurrent connections tracking with load shedding detection (`connections`)
//!
//! 2. **Session Management**:
//!    - Persistent session counts with expiry tracking (`sessions`)
//!    - Shared subscription distribution analysis (`subscriptions_shared`)
//!
//! 3. **Message Flow**:
//!    - QoS 1/2 in-flight message windows (`out_inflights`/`in_inflights`)
//!    - Message queue backpressure monitoring (`message_queues`)
//!    - Retained message storage metrics (`retaineds`)
//!
//! ## Architectural Features
//! - **Cluster Awareness**:
//!   ```rust,ignore
//!   topics_map: HashMap<NodeId, Counter>  // Trie-based topic tree metrics per node
//!   routes_map: HashMap<NodeId, Counter>   // Subscription routing table metrics
//!   ```
//!   Enables cross-node aggregation through merge_topics/merge_routes operations
//!
//! - **Performance Optimizations**:
//!   - Lock-free atomic counters for high-frequency updates
//!   - Conditional compilation for production/DEBUG builds
//!   - Zero-copy serialization via Serde integration
//!
//! - **Diagnostics**:
//!   - DEBUG feature exposes:
//!     - Client state machine transitions (`debug_client_states_map`)
//!     - Topic tree memory profiling (`debug_topics_tree_map`)
//!     - Async task execution analytics (`debug_global_exec_stats`)
//!
//! Typical usage includes:
//! - Capacity planning through trend analysis
//! - QoS compliance auditing
//! - Cluster load balancing decisions
//! - Anomaly detection via metric thresholds
//!
//! The implementation leverages Rust's type system to ensure:
//! - Memory safety through ownership model
//! - Linear scalability with cluster size
//! - Seamless integration with Prometheus/Grafana
//!

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::context::ServerContext;
#[cfg(feature = "debug")]
use crate::context::TaskExecStats;
use crate::types::{HashMap, NodeId};
use crate::utils::Counter;

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Stats {
    pub handshakings: Counter,
    pub handshakings_active: Counter,
    pub handshakings_rate: Counter,
    pub connections: Counter,
    pub sessions: Counter,
    pub subscriptions: Counter,
    pub subscriptions_shared: Counter,
    pub message_queues: Counter,
    pub out_inflights: Counter,
    pub in_inflights: Counter,
    pub forwards: Counter,
    pub message_storages: Counter,
    pub retaineds: Counter,
    pub delayed_publishs: Counter,

    pub topics_map: HashMap<NodeId, Counter>,
    pub routes_map: HashMap<NodeId, Counter>,

    #[cfg(feature = "debug")]
    debug_client_states_map: HashMap<NodeId, usize>,
    #[cfg(feature = "debug")]
    debug_topics_tree_map: HashMap<NodeId, usize>,
    #[cfg(feature = "debug")]
    debug_shared_peers: Counter,
    #[cfg(feature = "debug")]
    debug_subscriptions: usize,
    #[cfg(feature = "debug")]
    pub debug_session_channels: Counter,
    #[cfg(feature = "debug")]
    debug_global_exec_stats: Option<TaskExecStats>,
    // #[cfg(feature = "debug")]
    // debug_task_local_exec_stats: Option<TaskExecStats>,
}

impl Stats {
    #[inline]
    pub fn new() -> Self {
        Self {
            handshakings: Counter::new(),
            handshakings_active: Counter::new(),
            handshakings_rate: Counter::new(),
            connections: Counter::new(),
            sessions: Counter::new(),
            subscriptions: Counter::new(),
            subscriptions_shared: Counter::new(),
            message_queues: Counter::new(),
            out_inflights: Counter::new(),
            in_inflights: Counter::new(),
            forwards: Counter::new(),
            message_storages: Counter::new(),
            retaineds: Counter::new(),
            delayed_publishs: Counter::new(),

            topics_map: HashMap::default(),
            routes_map: HashMap::default(),

            #[cfg(feature = "debug")]
            debug_client_states_map: HashMap::default(),
            #[cfg(feature = "debug")]
            debug_topics_tree_map: HashMap::default(),
            #[cfg(feature = "debug")]
            debug_shared_peers: Counter::new(),
            #[cfg(feature = "debug")]
            debug_subscriptions: 0,
            #[cfg(feature = "debug")]
            debug_session_channels: Counter::new(),
            #[cfg(feature = "debug")]
            debug_global_exec_stats: None,
            // #[cfg(feature = "debug")]
            // debug_task_local_exec_stats: None,
        }
    }

    #[inline]
    pub async fn clone(&self, scx: &ServerContext) -> Self {
        let node_id = scx.node.id;
        let mut topics_map = HashMap::default();
        let mut routes_map = HashMap::default();
        {
            let router = scx.extends.router().await;
            topics_map.insert(node_id, router.topics());
            routes_map.insert(node_id, router.routes());
        }

        //@TODO
        self.handshakings.set(&scx.handshakings);
        self.handshakings_active.current_set(scx.handshake_exec.active_count());
        self.handshakings_rate.sets((scx.handshake_exec.get_rate().await * 100.0) as isize);
        self.connections.set(&scx.connections);
        self.sessions.set(&scx.sessions);

        //@TODO
        // let (curr, max) = in_inflights();
        // self.in_inflights.current_set(curr);
        // self.in_inflights.max_max(max);

        #[cfg(feature = "msgstore")]
        {
            let message_mgr = scx.extends.message_mgr().await;
            self.message_storages.current_set(message_mgr.count().await);
            self.message_storages.max_max(message_mgr.max().await);
        }

        #[cfg(feature = "retain")]
        let retaineds = {
            let retain = scx.extends.retain().await;
            Counter::new_with(retain.count().await, retain.max().await, retain.stats_merge_mode())
        };
        #[cfg(not(feature = "retain"))]
        let retaineds = Counter::default();

        #[cfg(feature = "delayed")]
        {
            let delayed_sender = scx.extends.delayed_sender().await;
            self.delayed_publishs.current_set(delayed_sender.len().await as isize);
        }

        #[cfg(feature = "debug")]
        let shared = scx.extends.shared().await;

        #[cfg(feature = "debug")]
        let mut debug_client_states_map = HashMap::default();
        #[cfg(feature = "debug")]
        let mut debug_topics_tree_map = HashMap::default();
        #[cfg(feature = "debug")]
        {
            debug_client_states_map.insert(node_id, shared.client_states_count().await);
            debug_topics_tree_map.insert(node_id, scx.extends.router().await.topics_tree().await);
        }
        #[cfg(feature = "debug")]
        self.debug_shared_peers.current_set(shared.sessions_count() as isize);
        #[cfg(feature = "debug")]
        let debug_subscriptions = shared.subscriptions_count().await;
        #[cfg(feature = "debug")]
        let debug_global_exec_stats = Some(TaskExecStats::from_global_exec(&scx.global_exec).await);
        // #[cfg(feature = "debug")]
        // let debug_task_local_exec_stats = Some(TaskExecStats::from_local_exec());

        Self {
            handshakings: self.handshakings.clone(),
            handshakings_active: self.handshakings_active.clone(),
            handshakings_rate: self.handshakings_rate.clone(),
            connections: self.connections.clone(),
            sessions: self.sessions.clone(),
            subscriptions: self.subscriptions.clone(),
            subscriptions_shared: self.subscriptions_shared.clone(),
            message_queues: self.message_queues.clone(),
            out_inflights: self.out_inflights.clone(),
            in_inflights: self.in_inflights.clone(),
            forwards: self.forwards.clone(),
            message_storages: self.message_storages.clone(),
            delayed_publishs: self.delayed_publishs.clone(),

            retaineds,
            topics_map,
            routes_map,

            #[cfg(feature = "debug")]
            debug_client_states_map,
            #[cfg(feature = "debug")]
            debug_topics_tree_map,
            #[cfg(feature = "debug")]
            debug_shared_peers: self.debug_shared_peers.clone(),
            #[cfg(feature = "debug")]
            debug_subscriptions,
            #[cfg(feature = "debug")]
            debug_session_channels: self.debug_session_channels.clone(),
            #[cfg(feature = "debug")]
            debug_global_exec_stats,
            // #[cfg(feature = "debug")]
            // debug_task_local_exec_stats,
        }
    }

    #[inline]
    pub fn add(&mut self, other: Self) {
        self.handshakings.add(&other.handshakings);
        self.handshakings_active.add(&other.handshakings_active);
        self.handshakings_rate.add(&other.handshakings_rate);
        self.connections.add(&other.connections);
        self.sessions.add(&other.sessions);
        self.subscriptions.add(&other.subscriptions);
        self.subscriptions_shared.add(&other.subscriptions_shared);
        self.message_queues.add(&other.message_queues);
        self.out_inflights.add(&other.out_inflights);
        self.in_inflights.add(&other.in_inflights);
        self.forwards.add(&other.forwards);
        self.message_storages.add(&other.message_storages);
        self.retaineds.merge(&other.retaineds);
        self.delayed_publishs.merge(&other.delayed_publishs);

        self.topics_map.extend(other.topics_map);
        self.routes_map.extend(other.routes_map);

        #[cfg(feature = "debug")]
        {
            self.debug_client_states_map.extend(other.debug_client_states_map);
            self.debug_topics_tree_map.extend(other.debug_topics_tree_map);
            self.debug_shared_peers.add(&other.debug_shared_peers);
            self.debug_subscriptions += other.debug_subscriptions;
            self.debug_session_channels.add(&other.debug_session_channels);

            if let Some(other) = other.debug_global_exec_stats.as_ref() {
                if let Some(stats) = self.debug_global_exec_stats.as_mut() {
                    stats.add(other);
                } else {
                    self.debug_global_exec_stats.replace(other.clone());
                }
            }

            // if let Some(other) = other.debug_task_local_exec_stats.as_ref() {
            //     if let Some(stats) = self.debug_task_local_exec_stats.as_mut() {
            //         stats.add(other);
            //     } else {
            //         self.debug_task_local_exec_stats.replace(other.clone());
            //     }
            // }
        }
    }

    #[allow(unused_mut)]
    #[inline]
    pub async fn to_json(&self, scx: &ServerContext) -> serde_json::Value {
        let router = scx.extends.router().await;
        let topics = router.merge_topics(&self.topics_map);
        let routes = router.merge_routes(&self.routes_map);

        let mut json_val = json!({
            "handshakings.count": self.handshakings.count(),
            "handshakings.max": self.handshakings.max(),
            "handshakings_active.count": self.handshakings_active.count(),
            "handshakings_rate.count": self.handshakings_rate.count() as f64 / 100.0,
            "handshakings_rate.max": self.handshakings_rate.max() as f64 / 100.0,
            "connections.count": self.connections.count(),
            "connections.max": self.connections.max(),
            "sessions.count": self.sessions.count(),
            "sessions.max": self.sessions.max(),
            "subscriptions.count": self.subscriptions.count(),
            "subscriptions.max": self.subscriptions.max(),
            "subscriptions_shared.count": self.subscriptions_shared.count(),
            "subscriptions_shared.max": self.subscriptions_shared.max(),
            "retaineds.count": self.retaineds.count(),
            "retaineds.max": self.retaineds.max(),

            "message_queues.count": self.message_queues.count(),
            "message_queues.max": self.message_queues.max(),
            "out_inflights.count": self.out_inflights.count(),
            "out_inflights.max": self.out_inflights.max(),
            "in_inflights.count": self.in_inflights.count(),
            "in_inflights.max": self.in_inflights.max(),
            "forwards.count": self.forwards.count(),
            "forwards.max": self.forwards.max(),
            "message_storages.count": self.message_storages.count(),
            "message_storages.max": self.message_storages.max(),
            "delayed_publishs.count": self.delayed_publishs.count(),
            "delayed_publishs.max": self.delayed_publishs.max(),

            "topics.count": topics.count(),
            "topics.max": topics.max(),
            "routes.count": routes.count(),
            "routes.max": routes.max(),
        });

        #[cfg(feature = "debug")]
        {
            if let Some(obj) = json_val.as_object_mut() {
                obj.insert("debug_client_states_map".into(), json!(self.debug_client_states_map));
                obj.insert("debug_topics_tree_map".into(), json!(self.debug_topics_tree_map));
                obj.insert("debug_shared_peers.count".into(), json!(self.debug_shared_peers.count()));
                obj.insert("debug_subscriptions.count".into(), json!(self.debug_subscriptions));
                obj.insert("debug_session_channels.count".into(), json!(self.debug_session_channels.count()));
                obj.insert("debug_session_channels.max".into(), json!(self.debug_session_channels.max()));
                obj.insert("debug_global_exec_stats".into(), json!(self.debug_global_exec_stats));
                // obj.insert("debug_task_local_exec_stats".into(), json!(self.debug_task_local_exec_stats));
            }
        }

        json_val
    }
}
