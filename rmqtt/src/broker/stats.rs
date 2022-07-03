use std::sync::atomic::{AtomicIsize, Ordering};

use once_cell::sync::OnceCell;

type Current = AtomicIsize;
type Max = AtomicIsize;

#[derive(Serialize, Deserialize)]
pub struct Counter(Current, Max);

impl Clone for Counter{
    fn clone(&self) -> Self{
        Counter(AtomicIsize::new(self.0.load(Ordering::SeqCst)),
                AtomicIsize::new(self.1.load(Ordering::SeqCst)))
    }
}

use std::fmt;

impl fmt::Debug for Counter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, r#"{{ "count":{}, "max":{} }}"#, self.count(), self.max())
    }
}

impl Counter {
    fn new() -> Self{
        Counter(AtomicIsize::new(0), AtomicIsize::new(0))
    }

    #[inline]
    pub fn inc(&self) {
        let prev  = self.0.fetch_add(1, Ordering::SeqCst);
        self.1.fetch_max(prev + 1, Ordering::SeqCst);
    }

    #[inline]
    pub fn dec(&self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }

    #[inline]
    pub fn count_min(&self, count: isize){
        self.0.fetch_min(count, Ordering::SeqCst);
    }

    #[inline]
    pub fn max_max(&self, max: isize){
        self.1.fetch_max(max, Ordering::SeqCst);
    }

    #[inline]
    pub fn count(&self) -> isize{
        self.0.load(Ordering::SeqCst)
    }

    #[inline]
    pub fn max(&self) -> isize{
        self.1.load(Ordering::SeqCst)
    }

    #[inline]
    fn add(&self, other: &Self){
        self.0.fetch_add(other.0.load(Ordering::SeqCst), Ordering::SeqCst);
        self.1.fetch_add(other.1.load(Ordering::SeqCst), Ordering::SeqCst);
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Stats {
    pub connections: Counter,
    pub sessions: Counter,
    pub subscriptions: Counter,
    pub subscriptions_shared: Counter,
    pub retaineds: Counter, //retained messages

    pub topics: Counter,   //subscribed_topics
    pub routes: Counter, //subscribe to the relationship
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

            topics: Counter::new(),
            routes: Counter::new(),
        })
    }

    #[inline]
    pub fn add(&mut self, other: Self) {
        self.connections.add(&other.connections);
        self.sessions.add(&other.sessions);
        self.subscriptions.add(&other.subscriptions);
        self.subscriptions_shared.add(&other.subscriptions_shared);
        self.retaineds.add(&other.retaineds);

        self.topics.count_min(other.topics.count());
        self.topics.max_max(other.topics.max());
        self.routes.count_min(other.routes.count());
        self.routes.max_max(other.routes.max());
    }

    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
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

            "topics.count": self.topics.count(),
            "topics.max": self.topics.max(),
            "routes.count": self.routes.count(),
            "routes.max": self.routes.max(),
        })
    }

}
