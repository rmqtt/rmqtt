use std::sync::atomic::{AtomicUsize, Ordering};
use once_cell::sync::OnceCell;
use crate::{HashMap, NodeId, Runtime};

#[async_trait]
pub trait Stats: Sync + Send {
    fn handshakings(&self) -> isize;

    fn connections_max(&self) -> usize;
    fn sessions_max(&self) -> usize;
    async fn subscribed_topics_max(&self) -> usize;
    fn subscriptions_max(&self) -> usize;
    fn subscriptions_shared_max(&self) -> usize;
    async fn routes_max(&self) -> usize;
    async fn retained_max(&self) -> usize;

    async fn data(&self) -> State;

    // fn publishs(&self) -> usize;
    // fn delivers(&self) -> usize;
    // fn ackeds(&self) -> usize;
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct State{
    id: NodeId,
    connections_count: usize,
    connections_max: usize,
    sessions_count: usize,
    sessions_max: usize,
    subscribed_topics_count: usize,
    subscribed_topics_max: usize,
    subscriptions_count: usize,
    subscriptions_max: usize,
    subscriptions_shared_count: usize,
    subscriptions_shared_max: usize,
    routes_max: usize,
    routes_count: usize,
    retained_count: usize,
    retained_max: usize,

    subscribed_topics_counts: HashMap<NodeId, usize>,
    subscribed_topics_maxs: HashMap<NodeId, usize>,
    routes_counts: HashMap<NodeId, usize>,
    routes_maxs: HashMap<NodeId, usize>,
}

impl State {

    #[inline]
    pub fn add(&mut self, other: &Self){
        self.connections_count += other.connections_count;
        self.connections_max += other.connections_max;
        self.sessions_count += other.sessions_count;
        self.sessions_max += other.sessions_max;
        self.subscriptions_count += other.subscriptions_count;
        self.subscriptions_max += other.subscriptions_max;
        self.subscriptions_shared_count += other.subscriptions_shared_count;
        self.subscriptions_shared_max += other.subscriptions_shared_max;
        self.retained_count += other.retained_count;
        self.retained_max += other.retained_max;

        self.subscribed_topics_counts.insert(other.id, other.subscribed_topics_count);
        self.subscribed_topics_maxs.insert(other.id, other.subscribed_topics_max);
        self.routes_counts.insert(other.id, other.routes_count);
        self.routes_maxs.insert(other.id, other.routes_max);
    }

    #[inline]
    pub fn to_sum_json(&self) -> serde_json::Value{
        json!({
            "connections.count": self.connections_count,
            "connections.max": self.connections_max,
            "sessions.count": self.sessions_count,
            "sessions.max": self.sessions_max,
            "subscriptions.count": self.subscriptions_count,
            "subscriptions.max": self.subscriptions_max,
            "subscriptions_shared.count": self.subscriptions_shared_count,
            "subscriptions_shared.max": self.subscriptions_shared_max,
            "retained.count": self.retained_count,
            "retained.max": self.retained_max,

            "subscribed_topics_counts": self.subscribed_topics_counts,
            "subscribed_topics_maxs": self.subscribed_topics_maxs,
            "routes_counts": self.routes_counts,
            "routes_maxs": self.routes_maxs,
        })
    }

    #[inline]
    pub fn to_json(&self) -> serde_json::Value{
        json!({
            "connections.count": self.connections_count,
            "connections.max": self.connections_max,
            "sessions.count": self.sessions_count,
            "sessions.max": self.sessions_max,
            "subscribed_topics.count": self.subscribed_topics_count,
            "subscribed_topics.max": self.subscribed_topics_max,
            "subscriptions.count": self.subscriptions_count,
            "subscriptions.max": self.subscriptions_max,
            "subscriptions_shared.count": self.subscriptions_shared_count,
            "subscriptions_shared.max": self.subscriptions_shared_max,
            "routes.count": self.routes_count,
            "routes.max": self.routes_max,
            "retained.count": self.retained_count,
            "retained.max": self.retained_max,
        })
    }
}

pub struct DefaultStats {
    connections_max: AtomicUsize,
    sessions_max: AtomicUsize,
    subscriptions_max: AtomicUsize,
    subscriptions_shared_max: AtomicUsize,
    // routes_max: AtomicUsize,
    // publishs: AtomicUsize,
    // delivers: AtomicUsize,
    // ackeds: AtomicUsize,
}

impl DefaultStats {
    #[inline]
    pub fn instance() -> &'static DefaultStats {
        static INSTANCE: OnceCell<DefaultStats> = OnceCell::new();
        INSTANCE.get_or_init(|| Self {
            connections_max: AtomicUsize::new(0),
            sessions_max: AtomicUsize::new(0),
            subscriptions_max: AtomicUsize::new(0),
            subscriptions_shared_max: AtomicUsize::new(0),
            // routes_max: AtomicUsize::new(0),
            // publishs: AtomicUsize::new(0),
            // delivers: AtomicUsize::new(0),
            // ackeds: AtomicUsize::new(0),
        })
    }


    #[inline]
    pub fn connections_max_inc(&self) -> usize {
        self.connections_max.fetch_add(1, Ordering::SeqCst)
    }

    #[inline]
    pub fn sessions_max_inc(&self) -> usize {
        self.sessions_max.fetch_add(1, Ordering::SeqCst)
    }

    #[inline]
    pub fn subscriptions_max_inc(&self) -> usize {
        self.subscriptions_max.fetch_add(1, Ordering::SeqCst)
    }

    #[inline]
    pub fn subscriptions_shared_max_inc(&self) -> usize {
        self.subscriptions_shared_max.fetch_add(1, Ordering::SeqCst)
    }

    // #[inline]
    // pub fn routes_max_inc(&self) -> usize {
    //     self.routes_max.fetch_add(1, Ordering::SeqCst)
    // }


    // #[inline]
    // pub fn publishs_inc(&self) -> usize {
    //     self.publishs.fetch_add(1, Ordering::SeqCst)
    // }
    //
    // #[inline]
    // pub fn delivers_inc(&self) -> usize {
    //     self.delivers.fetch_add(1, Ordering::SeqCst)
    // }
    //
    // #[inline]
    // pub fn ackeds_inc(&self) -> usize {
    //     self.ackeds.fetch_add(1, Ordering::SeqCst)
    // }
}

#[async_trait]
impl Stats for &'static DefaultStats {
    #[inline]
    fn handshakings(&self) -> isize {
        ntex_mqtt::handshakings()
    }

    #[inline]
    fn connections_max(&self) -> usize{
        self.connections_max.load(Ordering::SeqCst)
    }

    #[inline]
    fn sessions_max(&self) -> usize{
        self.sessions_max.load(Ordering::SeqCst)
    }

    #[inline]
    async fn subscribed_topics_max(&self) -> usize{
        Runtime::instance().extends.router().await.subscribed_topics_max()
    }

    #[inline]
    fn subscriptions_max(&self) -> usize{
        self.subscriptions_max.load(Ordering::SeqCst)
    }

    #[inline]
    fn subscriptions_shared_max(&self) -> usize{
        self.subscriptions_shared_max.load(Ordering::SeqCst)
    }

    #[inline]
    async fn routes_max(&self) -> usize{
        Runtime::instance().extends.router().await.relations_max()
    }

    #[inline]
    async fn retained_max(&self) -> usize{
        Runtime::instance().extends.router().await.relations_max()
    }


    // #[inline]
    // fn publishs(&self) -> usize{
    //     self.publishs.load(Ordering::SeqCst)
    // }
    //
    // #[inline]
    // fn delivers(&self) -> usize{
    //     self.delivers.load(Ordering::SeqCst)
    // }
    //
    // #[inline]
    // fn ackeds(&self) -> usize{
    //     self.ackeds.load(Ordering::SeqCst)
    // }

    #[inline]
    async fn data(&self) -> State{
        let shared = Runtime::instance().extends.shared().await;
        let router = Runtime::instance().extends.router().await;
        let retain = Runtime::instance().extends.retain().await;
        let id = Runtime::instance().node.id();

        let subscribed_topics = router.subscribed_topics();
        let subscribed_topics_max = router.subscribed_topics_max();

        let routes_count = router.relations();
        let routes_max = router.relations_max();

        let mut subscribed_topics_counts = HashMap::default(); //[(id, subscribed_topics)].into();
        subscribed_topics_counts.insert(id, subscribed_topics);
        let mut subscribed_topics_maxs = HashMap::default(); //[(id, subscribed_topics_max)].into();
        subscribed_topics_maxs.insert(id, subscribed_topics_max);
        let mut routes_counts = HashMap::default(); //[(id, routes_count)].into();
        routes_counts.insert(id, routes_count);
        let mut routes_maxs = HashMap::default(); //[(id, routes_max)].into();
        routes_maxs.insert(id, routes_max);

        State{
            id,
            connections_count: shared.clients().await,
            connections_max: self.connections_max(),
            sessions_count: shared.sessions().await,
            sessions_max: self.sessions_max(),
            subscribed_topics_count: subscribed_topics,
            subscribed_topics_max,
            subscriptions_count: shared.subscriptions(),
            subscriptions_max: self.subscriptions_max(),
            subscriptions_shared_count: shared.subscriptions_shared(),
            subscriptions_shared_max: self.subscriptions_shared_max(),
            routes_count,
            routes_max,
            retained_count: retain.count(),
            retained_max: retain.count_max(),

            subscribed_topics_counts,
            subscribed_topics_maxs,
            routes_counts,
            routes_maxs,
        }
    }
}