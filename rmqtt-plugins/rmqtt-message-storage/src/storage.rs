//! Persistent message storage implementation.
//!
//! Provides [`StorageMessageManager`] and [`StorageMessageManagerInner`]
//! for storing messages via the `rmqtt_storage` backend (Sled/Redis),
//! with batch writes, topic-tree restoration, and expiry cleanup.

use std::collections::BTreeSet;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use futures::{
    channel::mpsc,
    {SinkExt, StreamExt},
};
use futures_time::{self, future::FutureExt};
use tokio::{runtime::Handle, sync::RwLock, task::spawn_blocking, time::sleep};

use rmqtt::{
    message::MessageManager,
    retain::RetainTree,
    types::{
        ForwardedRecipients, From, MsgID, Publish, SharedGroup, StoredMessage, TimestampMillis, Topic,
        TopicFilter,
    },
    utils::timestamp_millis,
    Result,
};

use rmqtt::topic::Level;
#[cfg(feature = "circuit-breaker")]
pub(crate) use rmqtt_storage::{CircuitBrokenDB as StorageDb, CircuitBrokenMap as StorageMap};
#[cfg(not(feature = "circuit-breaker"))]
pub(crate) use rmqtt_storage::{DefaultStorageDB as StorageDb, StorageMap};

use rmqtt_storage::{Map, StorageDB as _};

use crate::config::PluginConfig;

type TopicTreeType = Arc<RwLock<RetainTree<MsgID>>>;
type TopicListType = Arc<RwLock<BTreeSet<(TimestampMillis, Topic)>>>;

const DATA: &[u8] = b"data";
const FORWARDED_PREFIX: &[u8] = b"fwd_";
/// How many worker tasks process storage messages.
/// Must be a power of two so that `% WORKER_COUNT` can be optimized to a bitmask.
const WORKER_COUNT: usize = 64;
/// Per-worker channel buffer size. Total capacity ≈ WORKER_COUNT * WORKER_CHANNEL_BOUND.
const WORKER_CHANNEL_BOUND: usize = 5_000;

enum Msg {
    Store {
        from: From,
        publish: Publish,
        expiry_interval: Duration,
        msg_id: MsgID,
        recipients: Option<ForwardedRecipients>,
    },
    MarkForwarded {
        msg_id: MsgID,
        recipients: ForwardedRecipients,
    },
}

/// A persistent message manager backed by `rmqtt_storage`.
///
/// Message routing: `msg_id % WORKER_COUNT` determines which worker processes the
/// message. Because each worker processes its messages **sequentially** (one batch
/// at a time), a `MarkForwarded` message is guaranteed to run after the
/// corresponding `Store` message for the same `msg_id`.
#[derive(Clone)]
pub struct StorageMessageManager {
    inner: Arc<StorageMessageManagerInner>,
    /// One sender per worker.  Index = `msg_id % WORKER_COUNT`.
    worker_txs: Arc<Vec<mpsc::Sender<Msg>>>,
    /// Timeout for storage I/O operations. 0 = no timeout. Examples: "5s", "500ms".
    timeout: Duration,
}

impl StorageMessageManager {
    /// Create a new `StorageMessageManager`.
    ///
    /// Spawns `WORKER_COUNT` worker tasks, each responsible for a disjoint
    /// subset of `msg_id` values (via `msg_id % WORKER_COUNT`).  Because
    /// every worker processes its messages **sequentially** (one batch at a time),
    /// a `MarkForwarded` message is guaranteed to execute **after** the
    /// corresponding `Store` message for the same `msg_id`.
    #[inline]
    pub(crate) async fn new(cfg: Arc<PluginConfig>, storage_db: StorageDb) -> Result<StorageMessageManager> {
        let id_generater = StorageMessageManagerInner::storage_new_msg_id_generater(&storage_db).await?;
        log::info!("current msg_id: {}", id_generater.load(Ordering::SeqCst));
        let messages_received_max =
            StorageMessageManagerInner::storage_new_messages_counter(&storage_db).await?;
        log::info!("messages_received_max: {}", messages_received_max.load(Ordering::SeqCst));

        let msg_queue_count = Arc::new(AtomicIsize::new(0));
        let timeout = cfg.timeout;

        // Build the shared inner state first, so workers can clone it.
        let inner = Arc::new(StorageMessageManagerInner {
            storage_db,
            topic_tree: Default::default(),
            topic_list: Default::default(),
            messages_received_max,
            msg_queue_count: msg_queue_count.clone(),
            id_generater,
            timeout,
        });

        // Spawn N worker tasks — one per routing bucket.
        let mut worker_txs = Vec::with_capacity(WORKER_COUNT);
        for i in 0..WORKER_COUNT {
            let (tx, rx) = mpsc::channel::<Msg>(WORKER_CHANNEL_BOUND);
            worker_txs.push(tx);

            let inner = inner.clone();

            tokio::spawn(async move {
                // Process messages one at a time, in strict order.
                // Because routing guarantees that Store and MarkForwarded for the
                // same msg_id land on the same worker, sequential processing
                // eliminates the race condition without batching.
                let mut rx = rx;
                while let Some(msg) = rx.next().await {
                    inner.msg_queue_count.fetch_sub(1, Ordering::Relaxed);

                    // When the backing storage is unreachable, storage operations
                    // will fast-fail through the rmqtt-storage circuit breaker.
                    // The worker simply logs the error and continues, preventing
                    // the channel backlog from blocking workers indefinitely.
                    if let Err(e) = inner._process_single(msg).await {
                        log::warn!("worker {} _process_single error: {:?}", i, e);
                    }
                }

                log::info!("worker {} exiting — all senders dropped", i);
            });
        }

        // --- cleanup task (same logic as the old `serve()`) -----------------
        let inner_cleanup = inner.clone();
        tokio::spawn(async move {
            let max_limit = cfg.cleanup_count;
            sleep(Duration::from_secs(20)).await;
            let mut now = std::time::Instant::now();
            let mut total_removeds = 0;
            loop {
                let inner = inner_cleanup.clone();
                let removeds = spawn_blocking(move || {
                    let curr_time = timestamp_millis();
                    Handle::current().block_on(async move {
                        let removed_topics = {
                            let mut topic_list = inner.topic_list.write().await;
                            let mut removeds = Vec::new();
                            while let Some((expiry_time_at, _)) = topic_list.first() {
                                if *expiry_time_at > curr_time || removeds.len() > max_limit {
                                    break;
                                }
                                if let Some((_, t)) = topic_list.pop_first() {
                                    removeds.push(t)
                                } else {
                                    break;
                                }
                            }
                            removeds
                        };
                        for t in removed_topics.iter() {
                            inner.topic_tree.write().await.remove(t);
                        }
                        removed_topics.len()
                    })
                })
                .await
                .unwrap_or_default();
                total_removeds += removeds;
                if removeds >= max_limit {
                    continue;
                }
                if removeds > 0 {
                    log::debug!(
                        "remove expired messages from topic tree, removeds: {:?} cost time: {:?}",
                        total_removeds,
                        now.elapsed()
                    );
                }
                sleep(Duration::from_secs(30)).await;
                now = std::time::Instant::now();
                total_removeds = 0;
            }
        });

        Ok(Self { inner, worker_txs: Arc::new(worker_txs), timeout })
    }

    /// Route a message to the worker responsible for its `msg_id`.
    #[inline]
    async fn route_msg(&self, msg: Msg) -> Result<()> {
        let msg_id = match &msg {
            Msg::Store { msg_id, .. } => *msg_id,
            Msg::MarkForwarded { msg_id, .. } => *msg_id,
        };
        let idx = msg_id & (WORKER_COUNT - 1); // power-of-two fast modulo
        let mut tx = self.worker_txs[idx].clone();
        let fut = tx.send(msg);
        let timeout = self.timeout;
        let res = if timeout > Duration::ZERO {
            fut.timeout(futures_time::time::Duration::from(timeout))
                .await
                .map_err(|e| anyhow::anyhow!("route_msg timeout (worker {}): {:?}", idx, e))
        } else {
            Ok(fut.await)
        };
        match res {
            Ok(Ok(())) => {
                // Count every message that successfully enters the channel.
                // The worker will fetch_sub(1) for each message it processes,
                // so increment and decrement are always balanced.
                self.msg_queue_count.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Ok(Err(e)) => {
                log::warn!("route_msg: send error (worker {}): {:?}", idx, e);
                Err(anyhow::anyhow!(e))
            }
            Err(e) => {
                log::warn!("route_msg: send timeout (worker {}): {:?}", idx, e);
                Err(e)
            }
        }
    }
}

impl Deref for StorageMessageManager {
    type Target = StorageMessageManagerInner;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.inner.deref()
    }
}

pub struct StorageMessageManagerInner {
    pub(crate) storage_db: StorageDb,
    pub(crate) topic_tree: TopicTreeType,
    topic_list: TopicListType,

    messages_received_max: AtomicIsize,

    pub(crate) msg_queue_count: Arc<AtomicIsize>,

    id_generater: AtomicUsize,

    /// Timeout for storage I/O operations. 0 = no timeout. Examples: "5s", "500ms".
    timeout: Duration,
}

impl StorageMessageManagerInner {
    /// Execute an async operation with an optional timeout.
    ///
    /// * When `timeout > Duration::ZERO`, wraps the future with the given timeout,
    ///   converting timeout errors to `anyhow::Error`.
    /// * When `timeout == Duration::ZERO`, runs the future directly.
    ///
    /// The inner error type `E` is converted to `anyhow::Error` via `Into<anyhow::Error>`.
    #[inline]
    async fn with_timeout<F, T, E>(fut: F, timeout: Duration, err_msg: &'static str) -> Result<T>
    where
        F: std::future::Future<Output = std::result::Result<T, E>>,
        E: Into<anyhow::Error>,
    {
        if timeout > Duration::ZERO {
            fut.timeout(futures_time::time::Duration::from(timeout))
                .await
                .map_err(|_e| anyhow!("{} timeout", err_msg))?
                .map_err(Into::into)
        } else {
            fut.await.map_err(Into::into)
        }
    }

    /// Execute an async operation with an optional timeout.
    ///
    /// Like [`with_timeout`], but accepts a future that does NOT return `Result`.
    /// The timeout error becomes the outer `anyhow::Error`.
    ///
    /// * When `timeout > Duration::ZERO`, wraps the future with the given timeout,
    ///   converting timeout errors to `anyhow::Error`.
    /// * When `timeout == Duration::ZERO`, runs the future directly.
    #[inline]
    async fn with_timeout_direct<F, T>(fut: F, timeout: Duration, err_msg: &'static str) -> Result<T>
    where
        F: std::future::Future<Output = T>,
    {
        if timeout > Duration::ZERO {
            fut.timeout(futures_time::time::Duration::from(timeout))
                .await
                .map_err(|_e| anyhow!("{} timeout", err_msg))
        } else {
            Ok(fut.await)
        }
    }

    #[inline]
    pub(crate) async fn restore_topic_tree(&self) -> Result<()> {
        let mut topic_tree = self.topic_tree.write().await;
        let mut topic_list = self.topic_list.write().await;
        let mut storage_db = self.storage_db.clone();
        let mut map_iter = storage_db.map_iter().await?;
        log::info!("restore topic tree ... ");
        let mut count = 0;
        let mut count_all = 0;
        while let Some(map) = map_iter.next().await {
            count_all += 1;
            match map {
                Ok(m) => match m.get::<_, StoredMessage>(DATA).await {
                    Ok(Some(smsg)) => {
                        count += 1;
                        log::debug!(
                            "Restore topic tree, smsg.msg_id: {:?}, smsg.is_expiry(): {}",
                            smsg.msg_id,
                            smsg.is_expiry()
                        );
                        if !smsg.is_expiry() {
                            let topic = match Topic::from_str(&smsg.publish.topic) {
                                Err(e) => {
                                    log::warn!("Topic::from_str error, {e}");
                                    continue;
                                }
                                Ok(mut topic) => {
                                    topic.push(Level::Normal(smsg.msg_id.to_string()));
                                    topic
                                }
                            };
                            topic_tree.insert(&topic, smsg.msg_id);
                            topic_list.insert((smsg.expiry_time_at, topic));
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        log::warn!("Restore topic tree error, {e}");
                    }
                },
                Err(e) => {
                    log::warn!("Restore topic tree error, {e}");
                }
            }
        }
        log::info!("restore count_all: {count_all}, count: {count}");
        Ok(())
    }

    #[inline]
    async fn storage_save_msg_id(&self) -> Result<()> {
        let curr_msg_id = self.id_generater.load(Ordering::SeqCst);
        self.storage_db.insert("id_generater", &curr_msg_id).await?;
        Ok(())
    }

    #[inline]
    async fn storage_new_msg_id_generater(storage_db: &StorageDb) -> Result<AtomicUsize> {
        if let Some(curr_msg_id) = storage_db.get::<_, usize>("id_generater").await? {
            Ok(AtomicUsize::new(curr_msg_id))
        } else {
            Ok(AtomicUsize::new(1))
        }
    }

    #[inline]
    fn storage_next_msg_id(&self) -> usize {
        self.id_generater.fetch_add(1, Ordering::SeqCst)
    }

    #[inline]
    async fn storage_messages_counter_add(&self, vals: isize) -> Result<()> {
        self.storage_db.counter_incr("messages_received_max", vals).await?;
        Ok(())
    }

    #[inline]
    async fn storage_new_messages_counter(storage_db: &StorageDb) -> Result<AtomicIsize> {
        let max = storage_db.counter_get("messages_received_max").await?.unwrap_or_default();
        Ok(AtomicIsize::new(max))
    }

    #[inline]
    fn messages_received_count_add(&self, len: isize) {
        self.messages_received_max.fetch_add(len, Ordering::SeqCst);
    }

    #[inline]
    fn make_forwarded_key(client_id: &str) -> Vec<u8> {
        [FORWARDED_PREFIX, client_id.as_bytes()].concat()
    }

    #[inline]
    /// Process a single message.
    ///
    /// Called by the worker loop — one message at a time, in strict order.
    /// Because routing guarantees that `Store` and `MarkForwarded` for the
    /// same `msg_id` land on the same worker, sequential processing
    /// eliminates the race condition that caused the "not found in storage" warning.
    async fn _process_single(&self, msg: Msg) -> Result<()> {
        let timeout = self.timeout;
        match msg {
            Msg::Store { from, publish, expiry_interval, msg_id, recipients } => {
                // Persist the msg_id counter so a restart can resume without reuse.
                Self::with_timeout(self.storage_save_msg_id(), timeout, "storage_save_msg_id")
                    .await
                    .map_err(|e| anyhow!("save message id error: {:?}", e))?;

                let mut topic = match Topic::from_str(&publish.topic) {
                    Err(e) => {
                        log::warn!("Topic::from_str error, {e}");
                        return Ok(());
                    }
                    Ok(topic) => topic,
                };
                let expiry_time_at = timestamp_millis() + expiry_interval.as_millis() as i64;

                let smsg = StoredMessage { msg_id, from, publish, expiry_time_at };

                // received messages
                let msg_key = msg_id.to_be_bytes();
                let msg_map = Self::with_timeout_direct(
                    self.storage_db.map(msg_key, Some(expiry_interval.as_millis() as TimestampMillis)),
                    timeout,
                    "storage_db.map",
                )
                .await
                .map_err(|e| {
                    log::warn!("store to db error, map_expire(..), {e}, message: {smsg:?}");
                    anyhow!("storage_db.map error: {:?}", e)
                })?;

                Self::with_timeout(msg_map.insert(DATA, &smsg), timeout, "map.insert").await.map_err(
                    |e| {
                        log::warn!("store to db error, {e}, message: {smsg:?}");
                        anyhow!("msg_map.insert error: {:?}", e)
                    },
                )?;

                if let Some(recipients) = recipients {
                    self._forwardeds(&msg_map, recipients).await?;
                }

                // topic
                topic.push(Level::Normal(msg_id.to_string()));
                self.topic_tree.write().await.insert(&topic, msg_id);
                self.topic_list.write().await.insert((expiry_time_at, topic));

                self.messages_received_count_add(1);
                Self::with_timeout(self.storage_messages_counter_add(1), timeout, "messages_counter_add")
                    .await?;
            }
            Msg::MarkForwarded { msg_id, recipients } => {
                let msg_key = msg_id.to_be_bytes();
                let msg_map = Self::with_timeout_direct(
                    self.storage_db.map(msg_key, None),
                    timeout,
                    "add_forwardeds map",
                )
                .await
                .map_err(|e| anyhow!("add_forwardeds map error: {:?}", e))?;

                let found =
                    Self::with_timeout(msg_map.contains_key(DATA), timeout, "add_forwardeds contains_key")
                        .await?;
                if found {
                    self._forwardeds(&msg_map, recipients).await?;
                } else {
                    log::warn!(
                        "add_forwardeds: msg_id {:?} not found in storage, recipients: {:?}",
                        msg_id,
                        recipients
                    );
                }
            }
        }
        Ok(())
    }

    #[inline]
    async fn _forwardeds(&self, msg_map: &StorageMap, forwardeds: ForwardedRecipients) -> Result<()> {
        let timeout = self.timeout;
        for (client_id, opts) in forwardeds {
            Self::with_timeout(
                msg_map.insert(Self::make_forwarded_key(&client_id), &opts),
                timeout,
                "forwardeds insert",
            )
            .await?;
        }
        Ok(())
    }

    #[inline]
    async fn _get(
        &self,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
    ) -> Result<Vec<(MsgID, From, Publish)>> {
        let inner = self;
        let mut topic = Topic::from_str(topic_filter).map_err(|e| anyhow!(format!("{:?}", e)))?;
        if !topic.levels().last().map(|l| matches!(l, Level::MultiWildcard)).unwrap_or_default() {
            topic.push(Level::SingleWildcard);
        }

        let matcheds: Vec<_> =
            inner.topic_tree.read().await.matches(&topic).into_iter().map(|(_t, msg_id)| msg_id).collect();

        log::debug!("_get matcheds msg_ids: {matcheds:?}");
        let matcheds: Vec<_> = futures::future::join_all(matcheds.into_iter().map(|msg_id| async move {
            let msg_key = msg_id.to_be_bytes();
            let mut msg_map = self.storage_db.map(msg_key, None).await;
            let is_forwarded = match self._is_forwarded(&mut msg_map, client_id, topic_filter, group).await {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("_get _is_forwarded error, {e}");
                    return None;
                }
            };

            if is_forwarded {
                None
            } else {
                match inner._get_message(&msg_map).await {
                    Ok(Some(msg)) => {
                        log::debug!("_get msg: {:?}, msg.is_expiry(): {}", msg, msg.is_expiry());
                        if msg.is_expiry()
                            || msg.publish.target_clientid.as_ref().is_some_and(|t_cid| t_cid != client_id)
                        {
                            None
                        } else {
                            Some((msg_id, msg.from, msg.publish))
                        }
                    }
                    Ok(None) => None,
                    Err(e) => {
                        log::warn!("_get_message error, {e}");
                        None
                    }
                }
            }
        }))
        .await
        .into_iter()
        .flatten()
        .collect();

        Ok(matcheds)
    }

    #[inline]
    async fn _is_forwarded(
        &self,
        msg_map: &mut StorageMap,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
    ) -> Result<bool> {
        let key = Self::make_forwarded_key(client_id);
        if msg_map.contains_key(key).await? {
            log::debug!("_is_forwarded contains_key client_id: {client_id:?}");
            return Ok(true);
        }
        if let Some(group) = group {
            let mut iter =
                msg_map.prefix_iter::<_, Option<(TopicFilter, SharedGroup)>>(FORWARDED_PREFIX).await?;
            while let Some(item) = iter.next().await {
                log::debug!("_is_forwarded item: {item:?}");
                match item {
                    Ok((_, Some((tf, g)))) => {
                        if g == group && tf == topic_filter {
                            return Ok(true);
                        }
                    }
                    Ok((_, None)) => {}
                    Err(e) => {
                        log::warn!("traverse forwardeds error, {e}");
                        return Err(anyhow!(e));
                    }
                }
            }
        }
        Ok(false)
    }

    #[inline]
    async fn _get_message(&self, msg_map: &StorageMap) -> Result<Option<StoredMessage>> {
        msg_map.get::<_, StoredMessage>(DATA).await
    }

    /// Return storage backend info, including circuit breaker state (when applicable)
    /// via `CircuitBrokenDB::info()`.
    pub(crate) async fn storage_info(&self) -> serde_json::Value {
        self.storage_db.info().await.unwrap_or_default()
    }
}

#[async_trait]
impl MessageManager for StorageMessageManager {
    #[inline]
    fn next_msg_id(&self) -> MsgID {
        self.storage_next_msg_id()
    }

    #[inline]
    async fn store(
        &self,
        msg_id: MsgID,
        from: From,
        p: Publish,
        expiry_interval: Duration,
        recipients: Option<ForwardedRecipients>,
    ) -> Result<()> {
        let msg = Msg::Store { from, publish: p, expiry_interval, msg_id, recipients };
        self.route_msg(msg).await
    }

    #[inline]
    async fn get(
        &self,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
    ) -> Result<Vec<(MsgID, From, Publish)>> {
        let now = std::time::Instant::now();
        let timeout = self.timeout;

        // Run _get directly — no exec queue indirection needed,
        // since _get is fully async (no blocking I/O).
        let result = {
            let fut = self._get(client_id, topic_filter, group);
            if timeout > Duration::ZERO {
                fut.timeout(futures_time::time::Duration::from(timeout)).await
            } else {
                Ok(fut.await)
            }
        };

        let matcheds = match result {
            Ok(Ok(res)) => res,
            Ok(Err(e)) => {
                log::error!("StorageMessageManager get error, {:?}", e);
                return Err(e);
            }
            Err(e) => {
                log::warn!("StorageMessageManager get timeout, {e}");
                vec![]
            }
        };

        if now.elapsed().as_millis() > 900 {
            log::info!("StorageMessageManager::get cost time: {:?}", now.elapsed());
        }
        Ok(matcheds)
    }

    #[inline]
    async fn mark_forwarded(&self, msg_id: MsgID, recipients: ForwardedRecipients) -> Result<()> {
        // Route to the worker responsible for this msg_id.
        // Because the worker processes messages sequentially (one batch at a time),
        // the corresponding `Store` for the same `msg_id` is guaranteed
        // to have been processed before this `MarkForwarded`.
        let msg = Msg::MarkForwarded { msg_id, recipients };
        self.route_msg(msg).await
    }

    #[inline]
    fn merge_on_read(&self) -> bool {
        true
    }

    #[inline]
    async fn count(&self) -> isize {
        self.topic_list.read().await.len() as isize
    }

    #[inline]
    async fn max(&self) -> isize {
        self.messages_received_max.load(Ordering::SeqCst)
    }

    #[inline]
    fn enable(&self) -> bool {
        true
    }
}
