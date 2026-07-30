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
// #[cfg(feature = "debug")]
use crate::context::TaskExecStats;
use crate::types::{HashMap, NodeId};
use crate::utils::Counter;
#[cfg(feature = "rate-counter")]
use rmqtt_utils::{RateCounter, StatsMergeMode};

/// Aggregated runtime statistics for the broker.
///
/// Tracks key performance indicators across connections, sessions,
/// message flows, gRPC activity, and internal system state.
/// Supports cluster-wide aggregation via per-node counter maps.
///
/// # Thread Safety
///
/// Uses atomic counters for lock-free updates on hot paths.
/// Per-node maps provide distributed aggregation without contention.
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Stats {
    /// Total completed TCP/TLS/WS handshakes.
    pub handshakings: Counter,
    /// Currently active handshake negotiations.
    pub handshakings_active: Counter,
    /// Handshake completion rate (ops/sec × 100).
    pub handshakings_rate: Counter,
    /// Active MQTT connections.
    pub connections: Counter,
    /// Active MQTT sessions (persistent + transient).
    pub sessions: Counter,
    /// Current subscription count across all topics.
    pub subscriptions: Counter,
    /// Shared subscription (`$share`) group count.
    pub subscriptions_shared: Counter,
    /// Messages pending in offline delivery queues.
    pub message_queues: Counter,
    /// Outbound QoS 1/2 messages awaiting acknowledgment.
    pub out_inflights: Counter,
    /// Inbound QoS 2 messages awaiting completion.
    pub in_inflights: Counter,
    /// Cluster message forwarding operations.
    pub forwards: Counter,
    /// Stored offline messages (via message storage plugin).
    pub message_storages: Counter,
    /// Retained messages currently stored.
    pub retaineds: Counter,
    /// Delayed publish messages pending delivery.
    pub delayed_publishs: Counter,
    /// Active gRPC server request handlers.
    pub grpc_server_actives: Counter,
    /// Active gRPC client tasks, keyed by peer node ID.
    pub grpc_clients_actives: HashMap<NodeId, Counter>,

    /// Subscription topic tree size, keyed by node ID.
    pub topics_map: HashMap<NodeId, Counter>,
    /// Route table size, keyed by node ID.
    pub routes_map: HashMap<NodeId, Counter>,

    /// Active tasks per named executor.
    pub execs_actives: HashMap<String, TaskExecStats>,

    /// Per-variant gRPC message rate counters (set/retains, forwards, etc.).
    #[cfg(feature = "rate-counter")]
    pub grpc_message_counters: HashMap<u8, RateCounter>,

    #[cfg(feature = "debug")]
    /// Client state machine transitions per node.
    debug_client_states_map: HashMap<NodeId, usize>,
    #[cfg(feature = "debug")]
    /// Topic trie node count per node.
    debug_topics_tree_map: HashMap<NodeId, usize>,
    #[cfg(feature = "debug")]
    debug_shared_peers: Counter,
    #[cfg(feature = "debug")]
    debug_subscriptions: usize,
    #[cfg(feature = "debug")]
    /// Active debug session notification channels.
    pub debug_session_channels: Counter,
}

impl Stats {
    /// Creates a new `Stats` instance with all counters initialized to zero.
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
            grpc_server_actives: Counter::new(),
            grpc_clients_actives: HashMap::default(),

            topics_map: HashMap::default(),
            routes_map: HashMap::default(),

            execs_actives: HashMap::default(),

            #[cfg(feature = "rate-counter")]
            grpc_message_counters: {
                let mut m = HashMap::default();
                for id in 0..crate::grpc::Message::VARIANT_COUNT as u8 {
                    m.insert(id, RateCounter::new_with_mode(StatsMergeMode::Sum));
                }
                m
            },

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
            // #[cfg(feature = "debug")]
            // debug_server_exec_stats: None,
            // #[cfg(feature = "debug")]
            // debug_client_exec_stats: None,
        }
    }

    /// Clone the current stats snapshot, aggregating live data from the server context.
    ///
    /// Reads current values from the router, gRPC clients, and executors
    /// to produce a point-in-time snapshot of all metrics.
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

        #[allow(unused_mut)]
        let mut grpc_clients_actives = HashMap::default();
        #[cfg(feature = "grpc")]
        {
            let shared = scx.extends.shared().await;
            for (id, (_, grpc_client)) in shared.get_grpc_clients().iter() {
                grpc_clients_actives.insert(*id, grpc_client.active_tasks().clone());
            }
        }

        //execs
        let mut execs_actives = HashMap::default();
        for (key, exec) in scx.execs() {
            execs_actives.insert(key, TaskExecStats::from_exec(&exec).await);
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
        // #[cfg(feature = "debug")]
        // let debug_server_exec_stats = Some(TaskExecStats::from_exec(&scx.server_exec).await);
        // #[cfg(feature = "debug")]
        // let debug_client_exec_stats = Some(TaskExecStats::from_exec(&scx.client_exec).await);

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
            grpc_server_actives: self.grpc_server_actives.clone(),
            grpc_clients_actives,

            retaineds,
            topics_map,
            routes_map,

            execs_actives,

            #[cfg(feature = "rate-counter")]
            grpc_message_counters: self
                .grpc_message_counters
                .iter()
                .map(|(k, v)| (*k, v.snapshot()))
                .collect(),

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
            // #[cfg(feature = "debug")]
            // debug_server_exec_stats,
            // #[cfg(feature = "debug")]
            // debug_client_exec_stats,
        }
    }

    /// Merge another `Stats` instance into this one by adding all counters.
    ///
    /// Used for aggregating stats from multiple cluster nodes.
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
        self.grpc_server_actives.merge(&other.grpc_server_actives);
        self.grpc_clients_actives.extend(other.grpc_clients_actives);

        self.topics_map.extend(other.topics_map);
        self.routes_map.extend(other.routes_map);

        for (k, v) in other.execs_actives {
            self.execs_actives.entry(k).and_modify(|tes| tes.add(&v)).or_insert_with(|| v);
        }

        #[cfg(feature = "debug")]
        {
            self.debug_client_states_map.extend(other.debug_client_states_map);
            self.debug_topics_tree_map.extend(other.debug_topics_tree_map);
            self.debug_shared_peers.add(&other.debug_shared_peers);
            self.debug_subscriptions += other.debug_subscriptions;
            self.debug_session_channels.add(&other.debug_session_channels);
        }

        #[cfg(feature = "rate-counter")]
        for (k, v) in other.grpc_message_counters {
            self.grpc_message_counters.entry(k).and_modify(|self_v| self_v.merge(&v)).or_insert(v);
        }
    }

    /// Serialize the stats snapshot to a JSON value for the management API.
    ///
    /// Aggregates per-node counters (topics, routes) into cluster-wide
    /// totals and includes only the essential metrics for operators.
    #[allow(unused_mut)]
    #[inline]
    pub async fn to_json(&self, scx: &ServerContext) -> serde_json::Value {
        let router = scx.extends.router().await;
        let topics = router.merge_topics(&self.topics_map);
        let routes = router.merge_routes(&self.routes_map);

        let grpc_clients_actives = Counter::new();
        for c in self.grpc_clients_actives.values() {
            grpc_clients_actives.add(c);
        }

        json!({
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
        })
    }

    /// Serialize the system-level stats to a JSON value for the `$SYS` topic tree.
    ///
    /// Includes internal metrics such as gRPC activity and executor state.
    /// When compiled with `debug` feature, additional diagnostic fields are included.
    #[allow(unused_mut)]
    #[inline]
    pub async fn to_sys_json(&self, _scx: &ServerContext) -> serde_json::Value {
        let grpc_clients_actives = Counter::new();
        for c in self.grpc_clients_actives.values() {
            grpc_clients_actives.add(c);
        }

        let mut json_val = json!({
            "grpc_server_actives.count": self.grpc_server_actives.count(),
            "grpc_server_actives.max": self.grpc_server_actives.max(),
            "grpc_clients_actives.count": grpc_clients_actives.count(),
            "grpc_clients_actives.max": grpc_clients_actives.max(),

            "execs_actives": self.execs_actives,
        });

        #[cfg(feature = "rate-counter")]
        {
            if !self.grpc_message_counters.is_empty() {
                if let Some(obj) = json_val.as_object_mut() {
                    let counters_json: HashMap<String, serde_json::Value> = self
                        .grpc_message_counters
                        .iter()
                        .map(|(vid, c)| {
                            let name = crate::grpc::Message::variant_id_to_name(*vid);
                            (name.to_string(), c.to_json())
                        })
                        .collect();
                    obj.insert("grpc_message_rates".into(), json!(counters_json));
                }
            }
        }

        #[cfg(feature = "debug")]
        {
            if let Some(obj) = json_val.as_object_mut() {
                obj.insert("debug_grpc_clients_actives".into(), json!(self.grpc_clients_actives));
                obj.insert("debug_client_states_map".into(), json!(self.debug_client_states_map));
                obj.insert("debug_topics_tree_map".into(), json!(self.debug_topics_tree_map));
                obj.insert("debug_shared_peers.count".into(), json!(self.debug_shared_peers.count()));
                obj.insert("debug_subscriptions.count".into(), json!(self.debug_subscriptions));
                obj.insert("debug_session_channels.count".into(), json!(self.debug_session_channels.count()));
                obj.insert("debug_session_channels.max".into(), json!(self.debug_session_channels.max()));
                // obj.insert("debug_server_exec_stats".into(), json!(self.debug_server_exec_stats));
                // obj.insert("debug_client_exec_stats".into(), json!(self.debug_client_exec_stats));
            }
        }

        json_val
    }
}
