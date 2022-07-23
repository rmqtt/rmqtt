use std::fmt;
use std::sync::atomic::{AtomicIsize, Ordering};

use once_cell::sync::OnceCell;

use crate::{NodeId, HashMap, Runtime};

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
        let prev = self.0.fetch_add(1, Ordering::SeqCst);
        self.1.fetch_max(prev + 1, Ordering::SeqCst);
    }

    #[inline]
    pub fn current_inc(&self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }


    #[inline]
    pub fn dec(&self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
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
    pub connections: Counter,
    pub sessions: Counter,
    pub subscriptions: Counter,
    pub subscriptions_shared: Counter,
    pub retaineds: Counter, //retained messages

    topics_map: HashMap<NodeId, Counter>,
    routes_map: HashMap<NodeId, Counter>,

}

impl Stats {
    #[inline]
    pub fn instance() -> &'static Self {
        static INSTANCE: OnceCell<Stats> = OnceCell::new();
        INSTANCE.get_or_init(|| Self {
            connections: Counter::new(),
            sessions: Counter::new(),
            subscriptions: Counter::new(),
            subscriptions_shared: Counter::new(),
            retaineds: Counter::new(),

            topics_map: HashMap::default(),
            routes_map: HashMap::default(),
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
        Self {
            connections: self.connections.clone(),
            sessions: self.sessions.clone(),
            subscriptions: self.subscriptions.clone(),
            subscriptions_shared: self.subscriptions_shared.clone(),
            retaineds: self.retaineds.clone(), //retained messages

            topics_map,
            routes_map,
        }
    }

    #[inline]
    pub fn add(&mut self, other: Self) {
        self.connections.add(&other.connections);
        self.sessions.add(&other.sessions);
        self.subscriptions.add(&other.subscriptions);
        self.subscriptions_shared.add(&other.subscriptions_shared);
        self.retaineds.add(&other.retaineds);

        self.topics_map.extend(other.topics_map);
        self.routes_map.extend(other.routes_map);
    }

    #[inline]
    pub async fn to_json(&self) -> serde_json::Value {
        let router = Runtime::instance().extends.router().await;
        let topics = router.merge_topics(&self.topics_map);
        let routes = router.merge_routes(&self.routes_map);
        json!({
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
        })
    }
}
