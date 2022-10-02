use std::fmt;
use std::sync::atomic::{AtomicIsize, Ordering};

use once_cell::sync::OnceCell;
use ntex_mqtt::handshakings;
use crate::{HashMap, NodeId, Runtime};
use crate::broker::executor::{get_active_count, get_rate};

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

    topics_map: HashMap<NodeId, Counter>,
    routes_map: HashMap<NodeId, Counter>,
    clinet_states_map: HashMap<NodeId, usize>,
    topics_tree_map: HashMap<NodeId, usize>,
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

            topics_map: HashMap::default(),
            routes_map: HashMap::default(),
            clinet_states_map: HashMap::default(),
            topics_tree_map: HashMap::default(),
        })
    }

    #[inline]
    pub async fn clone(&self) -> Self {
        let router = Runtime::instance().extends.router().await;
        let shared = Runtime::instance().extends.shared().await;

        let node_id = Runtime::instance().node.id();
        let mut topics_map = HashMap::default();
        topics_map.insert(node_id, router.topics());
        let mut routes_map = HashMap::default();
        routes_map.insert(node_id, router.routes());
        let mut clinet_states_map = HashMap::default();
        clinet_states_map.insert(node_id, shared.clinet_states_count().await);
        let mut topics_tree_map = HashMap::default();
        topics_tree_map.insert(node_id, router.topics_tree().await);

        self.handshakings.current_set(handshakings());
        self.handshakings_active.current_set(get_active_count());
        self.handshakings_rate.sets((get_rate() * 100.0) as isize);

        self.sessions.current_set(shared.sessions_count() as isize);

        Self {
            handshakings: self.handshakings.clone(),
            handshakings_active: self.handshakings_active.clone(),
            handshakings_rate: self.handshakings_rate.clone(),
            connections: self.connections.clone(),
            sessions: self.sessions.clone(),
            subscriptions: self.subscriptions.clone(),
            subscriptions_shared: self.subscriptions_shared.clone(),
            retaineds: self.retaineds.clone(), //retained messages

            topics_map,
            routes_map,
            clinet_states_map,
            topics_tree_map,
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

        self.topics_map.extend(other.topics_map);
        self.routes_map.extend(other.routes_map);
        self.clinet_states_map.extend(other.clinet_states_map);
        self.topics_tree_map.extend(other.topics_tree_map);
    }

    #[inline]
    pub async fn to_json(&self) -> serde_json::Value {
        let router = Runtime::instance().extends.router().await;
        let topics = router.merge_topics(&self.topics_map);
        let routes = router.merge_routes(&self.routes_map);

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
            "retained.count": self.retaineds.count(),
            "retained.max": self.retaineds.max(),

            "topics.count": topics.count(),
            "topics.max": topics.max(),
            "routes.count": routes.count(),
            "routes.max": routes.max(),

            "clinet_states_map": self.clinet_states_map,
            "topics_tree_map": self.topics_tree_map,
        })
    }
}
