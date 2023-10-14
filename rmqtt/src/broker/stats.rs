use std::fmt;
use std::sync::atomic::{AtomicIsize, Ordering};

use ntex_mqtt::{handshakings, in_inflights};
use once_cell::sync::OnceCell;

use crate::broker::executor::{get_active_count, get_rate};
use crate::{HashMap, NodeId, Runtime};

type Current = AtomicIsize;
type Max = AtomicIsize;

#[derive(Serialize, Deserialize)]
pub struct Counter(Current, Max);

impl Clone for Counter {
    fn clone(&self) -> Self {
        Counter(
            AtomicIsize::new(self.0.load(Ordering::SeqCst)),
            AtomicIsize::new(self.1.load(Ordering::SeqCst)),
        )
    }
}

impl fmt::Debug for Counter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, r#"{{ "count":{}, "max":{} }}"#, self.count(), self.max())
    }
}

impl Default for Counter {
    fn default() -> Self {
        Self::new()
    }
}

impl Counter {
    #[inline]
    pub fn new() -> Self {
        Counter(AtomicIsize::new(0), AtomicIsize::new(0))
    }

    #[inline]
    pub fn inc(&self) {
        self.incs(1);
    }

    #[inline]
    pub fn incs(&self, c: isize) {
        let prev = self.0.fetch_add(c, Ordering::SeqCst);
        self.1.fetch_max(prev + c, Ordering::SeqCst);
    }

    #[inline]
    pub fn current_inc(&self) {
        self.current_incs(1);
    }

    #[inline]
    pub fn current_incs(&self, c: isize) {
        self.0.fetch_add(c, Ordering::SeqCst);
    }

    #[inline]
    pub fn current_set(&self, c: isize) {
        self.0.store(c, Ordering::SeqCst);
    }

    #[inline]
    pub fn sets(&self, c: isize) {
        self.current_set(c);
        self.1.fetch_max(c, Ordering::SeqCst);
    }

    #[inline]
    pub fn dec(&self) {
        self.decs(1)
    }

    #[inline]
    pub fn decs(&self, c: isize) {
        self.0.fetch_sub(c, Ordering::SeqCst);
    }

    #[inline]
    pub fn count_min(&self, count: isize) {
        self.0.fetch_min(count, Ordering::SeqCst);
    }

    #[inline]
    pub fn max_max(&self, max: isize) {
        self.1.fetch_max(max, Ordering::SeqCst);
    }

    #[inline]
    pub fn count(&self) -> isize {
        self.0.load(Ordering::SeqCst)
    }

    #[inline]
    pub fn max(&self) -> isize {
        self.1.load(Ordering::SeqCst)
    }

    #[inline]
    pub fn add(&self, other: &Self) {
        self.0.fetch_add(other.0.load(Ordering::SeqCst), Ordering::SeqCst);
        self.1.fetch_add(other.1.load(Ordering::SeqCst), Ordering::SeqCst);
    }

    #[inline]
    pub fn set(&self, other: &Self) {
        self.0.store(other.0.load(Ordering::SeqCst), Ordering::SeqCst);
        self.1.store(other.1.load(Ordering::SeqCst), Ordering::SeqCst);
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Stats {
    pub handshakings: Counter,
    pub handshakings_active: Counter,
    pub handshakings_rate: Counter,
    pub connections: Counter,
    pub sessions: Counter,
    pub subscriptions: Counter,
    pub subscriptions_shared: Counter,
    pub retaineds: Counter,
    pub message_queues: Counter,
    pub out_inflights: Counter,
    pub in_inflights: Counter,
    pub forwards: Counter,

    topics_map: HashMap<NodeId, Counter>,
    routes_map: HashMap<NodeId, Counter>,

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
    debug_runtime_exec_stats: Option<RuntimeExecStats>,
}

impl Stats {
    #[inline]
    pub fn instance() -> &'static Self {
        static INSTANCE: OnceCell<Stats> = OnceCell::new();
        INSTANCE.get_or_init(|| Self {
            handshakings: Counter::new(),
            handshakings_active: Counter::new(),
            handshakings_rate: Counter::new(),
            connections: Counter::new(),
            sessions: Counter::new(),
            subscriptions: Counter::new(),
            subscriptions_shared: Counter::new(),
            retaineds: Counter::new(),
            message_queues: Counter::new(),
            out_inflights: Counter::new(),
            in_inflights: Counter::new(),
            forwards: Counter::new(),

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
            debug_runtime_exec_stats: None,
        })
    }

    #[inline]
    pub async fn clone(&self) -> Self {
        let router = Runtime::instance().extends.router().await;

        let node_id = Runtime::instance().node.id();
        let mut topics_map = HashMap::default();
        topics_map.insert(node_id, router.topics());
        let mut routes_map = HashMap::default();
        routes_map.insert(node_id, router.routes());

        self.handshakings.current_set(handshakings());
        self.handshakings_active.current_set(get_active_count());
        self.handshakings_rate.sets((get_rate() * 100.0) as isize);

        let (curr, max) = in_inflights();
        self.in_inflights.current_set(curr);
        self.in_inflights.max_max(max);

        #[cfg(feature = "debug")]
        let shared = Runtime::instance().extends.shared().await;

        #[cfg(feature = "debug")]
        let mut debug_client_states_map = HashMap::default();
        #[cfg(feature = "debug")]
        let mut debug_topics_tree_map = HashMap::default();
        #[cfg(feature = "debug")]
        {
            debug_client_states_map.insert(node_id, shared.client_states_count().await);
            debug_topics_tree_map.insert(node_id, router.topics_tree().await);
        }
        #[cfg(feature = "debug")]
        self.debug_shared_peers.current_set(shared.sessions_count() as isize);
        #[cfg(feature = "debug")]
        let debug_subscriptions = shared.subscriptions_count().await;
        #[cfg(feature = "debug")]
        let debug_runtime_exec_stats = Some(RuntimeExecStats::new());

        Self {
            handshakings: self.handshakings.clone(),
            handshakings_active: self.handshakings_active.clone(),
            handshakings_rate: self.handshakings_rate.clone(),
            connections: self.connections.clone(),
            sessions: self.sessions.clone(),
            subscriptions: self.subscriptions.clone(),
            subscriptions_shared: self.subscriptions_shared.clone(),
            retaineds: self.retaineds.clone(), //retained messages
            message_queues: self.message_queues.clone(),
            out_inflights: self.out_inflights.clone(),
            in_inflights: self.in_inflights.clone(),
            forwards: self.forwards.clone(),

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
            debug_runtime_exec_stats,
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
        self.retaineds.add(&other.retaineds);
        self.message_queues.add(&other.message_queues);
        self.out_inflights.add(&other.out_inflights);
        self.in_inflights.add(&other.in_inflights);
        self.forwards.add(&other.forwards);

        self.topics_map.extend(other.topics_map);
        self.routes_map.extend(other.routes_map);

        #[cfg(feature = "debug")]
        {
            self.debug_client_states_map.extend(other.debug_client_states_map);
            self.debug_topics_tree_map.extend(other.debug_topics_tree_map);
            self.debug_shared_peers.add(&other.debug_shared_peers);
            self.debug_subscriptions += other.debug_subscriptions;
            self.debug_session_channels.add(&other.debug_session_channels);

            if let Some(other) = other.debug_runtime_exec_stats.as_ref() {
                if let Some(stats) = self.debug_runtime_exec_stats.as_mut() {
                    stats.add(other);
                } else {
                    self.debug_runtime_exec_stats.replace(other.clone());
                }
            }
        }
    }

    #[allow(unused_mut)]
    #[inline]
    pub async fn to_json(&self) -> serde_json::Value {
        let router = Runtime::instance().extends.router().await;
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
            "retained.count": self.retaineds.count(),
            "retained.max": self.retaineds.max(),

            "message_queues.count": self.message_queues.count(),
            "message_queues.max": self.message_queues.max(),
            "out_inflights.count": self.out_inflights.count(),
            "out_inflights.max": self.out_inflights.max(),
            "in_inflights.count": self.in_inflights.count(),
            "in_inflights.max": self.in_inflights.max(),
            "forwards.count": self.forwards.count(),
            "forwards.max": self.forwards.max(),

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
                obj.insert("debug_runtime_exec_stats".into(), json!(self.debug_runtime_exec_stats));
            }
        }

        json_val
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RuntimeExecStats {
    active_count: isize,
    completed_count: isize,
    pending_wakers_count: usize,
    waiting_count: isize,
    rate: f64,
}

impl RuntimeExecStats {
    #[allow(dead_code)]
    pub async fn new() -> Self {
        let exec = &Runtime::instance().exec;
        Self {
            active_count: exec.active_count(),
            completed_count: exec.completed_count().await,
            pending_wakers_count: exec.pending_wakers_count(),
            waiting_count: exec.waiting_count(),
            rate: exec.rate().await,
        }
    }

    #[allow(dead_code)]
    #[inline]
    fn add(&mut self, other: &Self) {
        self.active_count += other.active_count;
        self.completed_count += other.completed_count;
        self.pending_wakers_count += other.pending_wakers_count;
        self.waiting_count += other.waiting_count;
        self.rate += other.rate;
    }
}
