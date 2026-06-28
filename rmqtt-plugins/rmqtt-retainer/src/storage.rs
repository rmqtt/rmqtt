//! Persistent retainer implementation.
//!
//! Stores retained messages in an external storage backend (Sled/Redis)
//! with batch writes, prefix-based topic matching, and optional TTL
//! expiry. Also provides [`ValueCached`] for caching storage query results.

use std::borrow::Cow;
use std::convert::From as _;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use async_trait::async_trait;
use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};
use futures_time::{self, future::FutureExt};
use rust_box::task_exec_queue::{SpawnExt, TaskExecQueue};
use tokio::sync::RwLock;

use rmqtt_storage::{DefaultStorageDB, StorageType};

#[cfg(feature = "rate-counter")]
use rmqtt::utils::RateCounter;
use rmqtt::{
    retain::RetainStorage,
    retain::{RetainSyncMode, RetainTree},
    types::{NodeId, Retain, TimedValue, TimestampMillis, Topic, TopicFilter, TopicName},
    utils::{timestamp_millis, CircuitBreaker, StatsMergeMode},
    Result,
};

use crate::config::PluginConfig;
use crate::value_cached::ValueCached;

type Msg = (TopicName, Retain, Option<Duration>);

/// Cluster sync message: updates the in-memory topics tree only (no Redis).
/// Sent via a dedicated unbounded channel to avoid blocking on batch_store I/O.
enum SyncMsg {
    Add(TopicName, Option<Duration>),
    Remove(TopicName),
}

type StoredMsg = (Retain, Option<TimestampMillis>);

const RETAIN_MESSAGES_MAX: &[u8] = b"m|";

const RETAIN_MESSAGES_PREFIX: &[u8] = b"p|";

#[derive(Clone)]
pub(crate) struct Retainer {
    inner: Arc<RetainerInner>,
}

impl Retainer {
    #[inline]
    pub(crate) async fn new(
        _node_id: NodeId,
        cfg: Arc<RwLock<PluginConfig>>,
        storage_db: DefaultStorageDB,
        storage_type: StorageType,
        exec: TaskExecQueue,
        batch_messages_limit: usize,
        circuit_breaker: CircuitBreaker,
    ) -> Result<Retainer> {
        let (msg_tx, msg_rx) = mpsc::channel::<Msg>(5_000);
        let (sync_tx, sync_rx) = mpsc::channel::<SyncMsg>(10_000);
        let storage_messages_count = ValueCached::new(Duration::from_millis(3000));
        let storage_messages_max = ValueCached::new(Duration::from_millis(3000));
        let abstract_info = ValueCached::new(Duration::from_millis(3000));

        let inner = Arc::new(RetainerInner {
            cfg,
            storage_db,
            msg_tx,
            sync_tx,
            storage_messages_count,
            storage_messages_max,
            storage_type,
            exec: exec.clone(),
            topics: Arc::new(RwLock::new(RetainTree::default())),
            circuit_breaker,
            #[cfg(feature = "rate-counter")]
            set_rate_counter: RateCounter::new(),
            #[cfg(feature = "rate-counter")]
            sync_rate_counter: RateCounter::new(),
            abstract_info,
        });
        let retainer = Self { inner };
        retainer.serve(exec, msg_rx, sync_rx, batch_messages_limit);

        // Rebuild topics tree from storage on startup.
        let rebuild = retainer.clone();
        // tokio::spawn(async move {
        if let Err(e) = rebuild.rebuild_topics().await {
            log::error!("rebuild_topics error: {e:?}");
        }
        // });

        Ok(retainer)
    }

    #[inline]
    async fn get_retain_count(&self) -> usize {
        let this = self.inner.clone();
        let cached = self.storage_messages_count.clone();
        let count = cached
            .call_timeout(async move { this._len().await }, Duration::from_millis(3000))
            .await
            .get()
            .map(|c| c.copied().unwrap_or_default())
            .unwrap_or_default();
        if count > 0 {
            count - 1
        } else {
            count
        }
    }

    #[inline]
    pub(crate) async fn info(&self) -> serde_json::Value {
        if self.circuit_breaker.is_blocked() {
            return serde_json::Value::Null;
        }

        let this = self.inner.clone();
        let cached = self.abstract_info.clone();
        let info = cached
            .call_timeout(async move { this._abstract_info().await }, Duration::from_millis(3000))
            .await
            .get()
            .map(|c| c.cloned().unwrap_or_default())
            .unwrap_or_default();
        info
    }

    fn serve(
        &self,
        _exec: TaskExecQueue,
        mut msg_rx: mpsc::Receiver<Msg>,
        mut sync_rx: mpsc::Receiver<SyncMsg>,
        batch_messages_limit: usize,
    ) {
        let msg_mgr = self.clone();
        let sync_topics = self.topics.clone();

        #[cfg(feature = "rate-counter")]
        let rate_counter = self.set_rate_counter.clone();
        #[cfg(feature = "rate-counter")]
        let rc = rate_counter.clone();
        #[cfg(feature = "rate-counter")]
        let sync_rc = self.sync_rate_counter.clone();

        // Rate sampling loop: compute throughput once per second.
        #[cfg(feature = "rate-counter")]
        tokio::spawn(async move {
            let interval = Duration::from_millis(1500);
            loop {
                tokio::time::sleep(interval).await;
                rc.tick(interval);
            }
        });

        // Sync rate sampling loop.
        #[cfg(feature = "rate-counter")]
        {
            let sync_rc_tick = sync_rc.clone();
            tokio::spawn(async move {
                let interval = Duration::from_millis(1500);
                loop {
                    tokio::time::sleep(interval).await;
                    sync_rc_tick.tick(interval);
                }
            });
        }

        // Dedicated task: processes retain-sync messages.
        // This is the ONLY task that writes to `topics` for sync messages,
        // eliminating lock contention with the raft exec queue (which used to
        // Sync operations are pure memory (no Redis), so this task never blocks.
        tokio::spawn(async move {
            while let Some(msg) = sync_rx.next().await {
                match msg {
                    SyncMsg::Add(topic_name, expiry) => {
                        if let Ok(topic) = Topic::from_str(&topic_name) {
                            let mut guard = sync_topics.write().await;
                            let _h = WriteHold::new("sync_task_add");
                            guard.insert(&topic, TimedValue::new((), expiry));
                        }
                    }
                    SyncMsg::Remove(topic_name) => {
                        if let Ok(topic) = Topic::from_str(&topic_name) {
                            let mut guard = sync_topics.write().await;
                            let _h = WriteHold::new("sync_task_remove");
                            guard.remove(&topic);
                        }
                    }
                }
                #[cfg(feature = "rate-counter")]
                sync_rc.dec();
            }
        });

        tokio::spawn(async move {
            log::info!("Starting Retainer serve ...");
            let mut merger_msgs = Vec::new();
            while let Some(msg) = msg_rx.next().await {
                merger_msgs.push(msg);
                while merger_msgs.len() < batch_messages_limit {
                    match tokio::time::timeout(Duration::from_millis(0), msg_rx.next()).await {
                        Ok(Some(msg)) => {
                            merger_msgs.push(msg);
                        }
                        _ => break,
                    }
                }

                log::debug!("merger_msgs.len: {}", merger_msgs.len());
                //merge and send
                let msgs = std::mem::take(&mut merger_msgs);
                let batch_size = msgs.len();

                // When the circuit breaker is OPEN, skip all storage operations
                // without touching Redis/Sled. This prevents the channel backlog
                // (cap=5000) from blocking the serve loop while each batch waits
                // for a storage timeout.
                // is_blocked() also handles OPEN→HALF_OPEN transitions, so when a
                // probe is due, the next batch will be processed normally.
                if msg_mgr.circuit_breaker.is_blocked() {
                    #[cfg(feature = "rate-counter")]
                    rate_counter.decs(batch_size as i64);
                    continue;
                }

                let start = Instant::now();
                match msg_mgr._batch_store_new(msgs).await {
                    Ok(()) => msg_mgr.circuit_breaker.record_success(),
                    Err(e) => {
                        log::warn!("batch store failed, {e:?}");
                        msg_mgr.circuit_breaker.record_failure();
                    }
                }

                #[cfg(feature = "rate-counter")]
                rate_counter.decs(batch_size as i64);

                if start.elapsed().as_secs() > 3 {
                    log::info!("batch stored {} msgs in {:?}", batch_size, start.elapsed());
                }
            }
            log::error!("Recv failed because receiver is gone");
        });
    }
}

impl Deref for Retainer {
    type Target = RetainerInner;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.inner.deref()
    }
}

pub struct RetainerInner {
    cfg: Arc<RwLock<PluginConfig>>,
    pub(crate) storage_db: DefaultStorageDB,
    msg_tx: mpsc::Sender<Msg>,
    /// Dedicated channel for retain-sync messages (no Redis, no blocking).
    sync_tx: mpsc::Sender<SyncMsg>,
    storage_messages_count: ValueCached<usize>,
    storage_messages_max: ValueCached<isize>,
    storage_type: StorageType,
    exec: TaskExecQueue,
    /// In-memory topic name trie index — tracks which topics have active
    /// retained messages. The value type `()` means only the topic name is
    /// stored (no retain data). Used for fast wildcard matching and
    /// cluster-aware topic synchronization.
    pub(crate) topics: Arc<RwLock<RetainTree<TimedValue<()>>>>,
    /// Circuit breaker for storage failure detection and fast-fail degradation.
    pub(crate) circuit_breaker: CircuitBreaker,
    /// Rate counter for tracking message processing throughput.
    #[cfg(feature = "rate-counter")]
    pub(crate) set_rate_counter: RateCounter,
    /// Rate counter for sync message processing throughput.
    #[cfg(feature = "rate-counter")]
    pub(crate) sync_rate_counter: RateCounter,
    abstract_info: ValueCached<serde_json::Value>,
}

/// Tracks how long the `topics.write()` lock is HELD (after acquisition).
/// Create immediately after `write().await` returns, keep alive until guard drop.
struct WriteHold {
    label: &'static str,
    start: Instant,
}

impl WriteHold {
    #[inline]
    fn new(label: &'static str) -> Self {
        Self { label, start: Instant::now() }
    }
}

impl Drop for WriteHold {
    fn drop(&mut self) {
        let held = self.start.elapsed();
        if held > Duration::from_millis(100) {
            log::warn!("Topics WRITE lock HELD {:?} by [{}]", held, self.label);
        }
    }
}

impl RetainerInner {
    /// Original per-message version (逐个写入，用于对比重现)
    #[inline]
    async fn _batch_store(&self, msgs: Vec<Msg>) -> Result<()> {
        let (max_retained_messages, max_payload_size, retained_message_ttl) = {
            let cfg = self.cfg.read().await;
            (cfg.max_retained_messages as usize, *cfg.max_payload_size, cfg.retained_message_ttl)
        };

        let mut count = 0;
        for (topic_name, retain, expiry_interval) in msgs {
            let store_topic_name = [RETAIN_MESSAGES_PREFIX, topic_name.as_bytes().as_ref()].concat();
            if retain.publish.payload.is_empty() {
                //remove retain messagge
                if let Err(e) = self
                    .storage_db
                    .remove(store_topic_name.as_slice())
                    .timeout(futures_time::time::Duration::from_millis(5000))
                    .await
                    .map_err(|_e| anyhow!("storage_db.remove timeout"))?
                {
                    log::warn!("remove from db error, remove(..), {e:?}, topic_name: {topic_name:?}");
                    return Err(e);
                };
                // Remove from in-memory topics tree
                if let Ok(topic) = Topic::from_str(topic_name.as_ref()) {
                    let mut guard = self.topics.write().await;
                    let _h = WriteHold::new("batch_remove");
                    guard.remove(&topic);
                }
            } else {
                match self
                    .check_constraints(topic_name.as_ref(), &retain, max_retained_messages, max_payload_size)
                    .await
                {
                    Ok(false) => {
                        continue;
                    }
                    Ok(true) => {}
                    Err(e) => {
                        log::warn!("store to db error, check_constraints(..), {e:?}, message: {retain:?}");
                        continue;
                    }
                }

                //add retain messagge
                let expiry_interval_millis = retained_message_ttl
                    .map(|ttl| if ttl.is_zero() { None } else { Some(ttl.as_millis() as i64) })
                    .unwrap_or_else(|| {
                        expiry_interval.map(|expiry_interval| expiry_interval.as_millis() as i64)
                    });

                let expiry_time_at = expiry_interval_millis
                    .map(|expiry_interval_millis| timestamp_millis() + expiry_interval_millis);

                let smsg: StoredMsg = (retain, expiry_time_at);
                if let Err(e) = self
                    .storage_db
                    .insert(store_topic_name.as_slice(), &smsg)
                    .timeout(futures_time::time::Duration::from_millis(5000))
                    .await
                    .map_err(|_e| anyhow!("storage_db.insert timeout"))?
                {
                    log::warn!("store to db error, insert(..), {e:?}, message: {smsg:?}");
                    return Err(e);
                };

                if let Some(expiry_interval_millis) = expiry_interval_millis {
                    if let Err(e) = self
                        .storage_db
                        .expire(store_topic_name.as_slice(), expiry_interval_millis)
                        .timeout(futures_time::time::Duration::from_millis(5000))
                        .await
                        .map_err(|_e| anyhow!("set expire timeout"))?
                    {
                        log::warn!("store to db error, expire(..), {e:?}, message: {smsg:?}");
                        return Err(e);
                    }
                }

                // Update in-memory topics tree
                if let Ok(topic) = Topic::from_str(topic_name.as_ref()) {
                    let timeout = expiry_interval_millis.map(|millis| Duration::from_millis(millis as u64));
                    let mut guard = self.topics.write().await;
                    let _h = WriteHold::new("batch_insert");
                    guard.insert(&topic, TimedValue::new((), timeout));
                }

                count += 1;
            }
        }

        if let Err(e) = self
            .storage_messages_max_add(count)
            .timeout(futures_time::time::Duration::from_millis(5000))
            .await
            .map_err(|_e| anyhow!("storage_messages_max_add timeout"))?
        {
            log::warn!("messages_received_counter add error, {e:?}");
            return Err(e);
        }

        Ok(())
    }

    /// Batch-optimized version (batch_insert / batch_remove)
    #[inline]
    async fn _batch_store_new(&self, msgs: Vec<Msg>) -> Result<()> {
        let (max_retained_messages, max_payload_size, retained_message_ttl) = {
            let cfg = self.cfg.read().await;
            (cfg.max_retained_messages as usize, *cfg.max_payload_size, cfg.retained_message_ttl)
        };

        let mut insert_batch: Vec<(Vec<u8>, StoredMsg)> = Vec::with_capacity(msgs.len());
        let mut remove_keys: Vec<Vec<u8>> = Vec::with_capacity(msgs.len());
        let mut expire_tasks: Vec<(Vec<u8>, i64)> = Vec::new();
        let mut count = 0;
        for (topic_name, retain, expiry_interval) in msgs {
            let store_topic_name = [RETAIN_MESSAGES_PREFIX, topic_name.as_bytes().as_ref()].concat();
            if retain.publish.payload.is_empty() {
                //remove retain messagge
                remove_keys.push(store_topic_name);
                // Remove from in-memory topics tree
                if let Ok(topic) = Topic::from_str(topic_name.as_ref()) {
                    let mut guard = self.topics.write().await;
                    let _h = WriteHold::new("batch_remove");
                    guard.remove(&topic);
                }
            } else {
                match self
                    .check_constraints(topic_name.as_ref(), &retain, max_retained_messages, max_payload_size)
                    .await
                {
                    Ok(false) => {
                        continue;
                    }
                    Ok(true) => {}
                    Err(e) => {
                        log::warn!("store to db error, check_constraints(..), {e:?}, message: {retain:?}");
                        continue;
                    }
                }

                //add retain messagge
                let expiry_interval_millis = retained_message_ttl
                    .map(|ttl| if ttl.is_zero() { None } else { Some(ttl.as_millis() as i64) })
                    .unwrap_or_else(|| {
                        expiry_interval.map(|expiry_interval| expiry_interval.as_millis() as i64)
                    });

                let expiry_time_at = expiry_interval_millis
                    .map(|expiry_interval_millis| timestamp_millis() + expiry_interval_millis);

                let smsg: StoredMsg = (retain, expiry_time_at);
                if let Some(expiry_interval_millis) = expiry_interval_millis {
                    expire_tasks.push((store_topic_name.clone(), expiry_interval_millis));
                }
                insert_batch.push((store_topic_name.clone(), smsg));

                // Update in-memory topics tree
                if let Ok(topic) = Topic::from_str(topic_name.as_ref()) {
                    let timeout = expiry_interval_millis.map(|millis| Duration::from_millis(millis as u64));
                    let mut guard = self.topics.write().await;
                    let _h = WriteHold::new("batch_insert");
                    guard.insert(&topic, TimedValue::new((), timeout));
                }

                count += 1;
            }
        }

        // Batch remove — failure propagates to the serve loop so the
        // circuit breaker can record the failure and eventually trip OPEN.
        if !remove_keys.is_empty() {
            self.storage_db
                .batch_remove(remove_keys)
                .timeout(futures_time::time::Duration::from_millis(5000))
                .await
                .map_err(|_e| anyhow!("storage_db.batch_remove timeout"))?
                .map_err(|e| anyhow!("batch_remove error: {e:?}"))?;
        }

        // Batch insert — failure propagates to the serve loop.
        if !insert_batch.is_empty() {
            self.storage_db
                .batch_insert(insert_batch)
                .timeout(futures_time::time::Duration::from_millis(5000))
                .await
                .map_err(|_e| anyhow!("storage_db.batch_insert timeout"))?
                .map_err(|e| anyhow!("batch_insert error: {e:?}"))?;

            // Set TTL for inserted messages
            for (key, ms) in expire_tasks {
                if let Err(e) = self
                    .storage_db
                    .expire(key.as_slice(), ms)
                    .timeout(futures_time::time::Duration::from_millis(5000))
                    .await
                    .map_err(|_e| anyhow!("set expire timeout"))?
                {
                    log::warn!("store to db error, expire(..), {e:?}");
                    return Err(e);
                }
            }
        }

        if let Err(e) = self
            .storage_messages_max_add(count)
            .timeout(futures_time::time::Duration::from_millis(5000))
            .await
            .map_err(|_e| anyhow!("storage_messages_max_add timeout"))?
        {
            log::warn!("messages_received_counter add error, {e:?}");
            return Err(e);
        }

        Ok(())
    }

    #[inline]
    async fn check_constraints(
        &self,
        topic: &str,
        retain: &Retain,
        max_retained_messages: usize,
        max_payload_size: usize,
    ) -> Result<bool> {
        if retain.publish.payload.len() > max_payload_size {
            log::warn!("Retain message payload exceeding limit, topic: {topic:?}, retain: {retain:?}");
            return Ok(false);
        }

        if max_retained_messages > 0 && self.topics.read().await.values_size() >= max_retained_messages {
            log::warn!(
                "The retained message has exceeded the maximum limit of: {max_retained_messages}, topic: {topic:?}, retain: {retain:?}"
            );
            return Ok(false);
        }

        Ok(true)
    }

    #[inline]
    async fn _len(&self) -> Result<usize> {
        let len = self
            .storage_db
            .len()
            .timeout(futures_time::time::Duration::from_millis(3000))
            .await
            .map_err(|_e| anyhow!("db.len timeout"))
            .flatten();

        match len {
            Ok(len) => {
                self.circuit_breaker.record_success();
                Ok(len)
            }
            Err(_) => {
                self.circuit_breaker.record_failure();
                Ok(0)
            }
        }
    }

    #[inline]
    async fn storage_messages_max_add(&self, vals: isize) -> Result<()> {
        self.storage_db.counter_incr(RETAIN_MESSAGES_MAX, vals).await?;
        Ok(())
    }

    #[inline]
    async fn storage_messages_max_get(&self) -> Result<isize> {
        let val = self
            .storage_db
            .counter_get(RETAIN_MESSAGES_MAX)
            .timeout(futures_time::time::Duration::from_millis(3000))
            .await
            .map_err(|_e| anyhow!("storage_messages_max_get timeout"))
            .flatten();
        match val {
            Ok(val) => {
                self.circuit_breaker.record_success();
                Ok(val.unwrap_or_default())
            }
            Err(_) => {
                self.circuit_breaker.record_failure();
                Ok(0)
            }
        }
    }

    #[inline]
    #[allow(dead_code)]
    fn topic_filter_to_pattern(t: &str) -> Cow<'_, str> {
        if t.len() == 1 && (t == "#" || t == "+") {
            return Cow::Borrowed("*");
        }

        let t = t.replace('*', "\\*").replace('?', "\\?").replace('+', "*");

        if t.len() > 1 && t.ends_with("/#") {
            Cow::Owned([&t[0..(t.len() - 2)], "*"].concat())
        } else {
            Cow::Owned(t)
        }
    }

    #[inline]
    async fn get_message(&self, topic_filter: &TopicFilter) -> Result<Vec<(TopicName, Retain)>> {
        let topic = Topic::from_str(topic_filter)?;

        // Precise topic short circuit - GET directly without wildcard, O (1) single Redis query
        if !topic_filter.contains(['+', '#']) {
            let exact_key = [RETAIN_MESSAGES_PREFIX, topic_filter.as_bytes()].concat();
            match self.storage_db.get::<_, StoredMsg>(exact_key.as_slice()).await {
                Ok(Some((retain, expiry_time_at))) => {
                    let retains = if let Some(expiry_time_at) = expiry_time_at {
                        if expiry_time_at > timestamp_millis() {
                            vec![(TopicName::from(topic_filter.as_ref()), retain)]
                        } else {
                            Vec::new()
                        }
                    } else {
                        vec![(TopicName::from(topic_filter.as_ref()), retain)]
                    };
                    Ok(retains)
                }
                Ok(None) => Ok(Vec::new()),
                Err(e) => {
                    log::error!("get_message exact get error: {e:?}");
                    Err(e)
                }
            }
        } else {
            // Wildcard path: use in-memory topics tree for fast trie matching,
            // then fetch the actual retain data from storage for each matching topic.
            let matched_topics = {
                let topics = self.topics.read().await;
                topics
                    .matches(&topic)
                    .into_iter()
                    .filter_map(|(t, tv)| if tv.is_expired() { None } else { Some(t.to_string()) })
                    .collect::<Vec<_>>()
            };

            let mut retains = Vec::new();
            for topic_str in &matched_topics {
                let store_key = [RETAIN_MESSAGES_PREFIX, topic_str.as_bytes()].concat();
                match self.storage_db.get::<_, StoredMsg>(store_key.as_slice()).await {
                    Ok(Some((retain, expiry_time_at))) => {
                        let topic_name = TopicName::from(topic_str.as_str());
                        if let Some(expiry_time_at) = expiry_time_at {
                            if expiry_time_at > timestamp_millis() {
                                retains.push((topic_name, retain));
                            }
                        } else {
                            retains.push((topic_name, retain));
                        }
                    }
                    Ok(None) => {
                        // Stale topic entry in topics tree — clean it up
                        // if let Ok(t) = Topic::from_str(topic_str.as_str()) {
                        //     self.topics.write().await.remove(&t);
                        // }
                    }
                    Err(e) => {
                        log::error!("get_message get error: {e:?}");
                        return Err(e);
                    }
                }
            }
            Ok(retains)
        }
    }

    #[inline]
    async fn _get_all_paginated(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<(TopicName, Retain, Option<Duration>)>, bool)> {
        let mut db = self.storage_db.clone();
        let topic_filter_pattern = Self::topic_filter_to_pattern("#");
        let mut iter = match db.scan([RETAIN_MESSAGES_PREFIX, topic_filter_pattern.as_bytes()].concat()).await
        {
            Err(e) => {
                log::error!("get_all_paginated scan error: {e:?}");
                return Ok((Vec::new(), false));
            }
            Ok(iter) => iter,
        };

        // Collect all matching keys first, then drop the iterator to release
        // the mutable borrow on `db`, so we can call `db.get` later.
        let mut matched_keys = Vec::new();
        while let Some(key) = iter.next().await {
            match key {
                Ok(key) if key.starts_with(RETAIN_MESSAGES_PREFIX) => {
                    matched_keys.push(key);
                }
                Ok(_) => {}
                Err(e) => log::error!("get_all_paginated iter error: {e:?}"),
            }
        }
        drop(iter);

        let total = matched_keys.len();
        let has_more = offset + limit < total;

        let mut items = Vec::with_capacity(limit);
        for key in matched_keys.into_iter().skip(offset).take(limit) {
            match db.get::<_, StoredMsg>(key.as_slice()).await {
                Ok(Some((retain, expiry_time_at))) => {
                    let topic_name = TopicName::from(
                        String::from_utf8_lossy(&key[RETAIN_MESSAGES_PREFIX.len()..]).as_ref(),
                    );
                    let remaining = expiry_time_at.and_then(|exp| {
                        let now = timestamp_millis();
                        if exp > now {
                            Some(Duration::from_millis((exp - now) as u64))
                        } else {
                            None
                        }
                    });
                    items.push((topic_name, retain, remaining));
                }
                Ok(None) => {}
                Err(e) => log::error!("get_all_paginated get error: {e:?}"),
            }
        }

        Ok((items, has_more))
    }

    /// Rebuild the in-memory `topics` tree by scanning all stored retain
    /// messages. Called once at startup.
    #[inline]
    async fn rebuild_topics(&self) -> Result<()> {
        let start = std::time::Instant::now();
        log::info!("Rebuilding topics tree from storage ...");
        let mut db = self.storage_db.clone();
        let topic_filter_pattern = Self::topic_filter_to_pattern("#");
        let mut iter = match db.scan([RETAIN_MESSAGES_PREFIX, topic_filter_pattern.as_bytes()].concat()).await
        {
            Err(e) => {
                log::warn!("rebuild_topics scan error: {e:?}");
                return Err(e);
            }
            Ok(iter) => iter,
        };

        // Collect keys first, then drop the iterator to release the mutable
        // borrow on `db`.
        let mut keys = Vec::new();
        while let Some(key) = iter.next().await {
            let key = match key {
                Ok(k) => k,
                Err(e) => {
                    log::warn!("rebuild_topics iter error: {e:?}");
                    continue;
                }
            };
            if key.starts_with(RETAIN_MESSAGES_PREFIX) {
                keys.push(key);
            }
        }
        drop(iter);

        let mut topics = self.topics.write().await;
        let _h_rt = WriteHold::new("rebuild_topics");
        for key in keys {
            let topic_str = String::from_utf8_lossy(&key[RETAIN_MESSAGES_PREFIX.len()..]);
            let topic = match Topic::from_str(topic_str.as_ref()) {
                Ok(t) => t,
                Err(_) => continue,
            };
            match db.get::<_, StoredMsg>(key.as_slice()).await {
                Ok(Some((retain, expiry_time_at))) if !retain.publish.payload.is_empty() => {
                    let timeout = expiry_time_at
                        .filter(|exp| *exp > timestamp_millis())
                        .map(|exp| Duration::from_millis((exp - timestamp_millis()) as u64));
                    topics.insert(&topic, TimedValue::new((), timeout));
                }
                _ => {}
            }
        }
        log::info!(
            "Rebuild topics tree done, nodes: {}, values: {}, elapsed: {:?}",
            topics.nodes_size(),
            topics.values_size(),
            start.elapsed()
        );
        Ok(())
    }

    /// Handle a topic-only retain sync notification from a cluster peer
    /// (Redis mode). Unconditionally inserts or removes the topic from the
    /// in-memory `topics` tree **without** querying shared storage —
    /// notification carries the operation direction.
    #[inline]
    async fn _abstract_info(&self) -> Result<serde_json::Value> {
        let info = self
            .storage_db
            .info()
            .timeout(futures_time::time::Duration::from_millis(3000))
            .await
            .map_err(|_e| anyhow!("get info timeout"))
            .flatten();

        match info {
            Ok(info) => {
                self.circuit_breaker.record_success();
                Ok(info)
            }
            Err(e) => {
                self.circuit_breaker.record_failure();
                Err(e)
            }
        }
    }
}

#[async_trait]
impl RetainStorage for Retainer {
    #[inline]
    fn enable(&self) -> bool {
        true
    }

    #[inline]
    fn merge_on_read(&self) -> bool {
        match self.storage_type {
            #[cfg(feature = "sled")]
            StorageType::Sled => false,
            #[cfg(feature = "redis")]
            StorageType::Redis => false,
            #[allow(unreachable_patterns)]
            _ => false,
        }
    }

    #[inline]
    fn need_sync(&self) -> bool {
        match self.storage_type {
            #[cfg(feature = "redis")]
            StorageType::Redis => false,
            #[allow(unreachable_patterns)]
            _ => true,
        }
    }

    ///topic - concrete topic
    async fn set(&self, topic: &TopicName, retain: Retain, expiry_interval: Option<Duration>) -> Result<()> {
        if self.circuit_breaker.is_blocked() {
            return Ok(());
        }

        let res =
            self.msg_tx.clone().send((topic.clone(), retain, expiry_interval)).await.map_err(|e| anyhow!(e));
        match res {
            Ok(()) => {
                #[cfg(feature = "rate-counter")]
                self.set_rate_counter.inc();
                Ok(())
            }
            Err(e) => {
                log::error!("Retainer set error, {e:?}");
                Err(anyhow!(e.to_string()))
            }
        }
    }

    ///topic_filter - Topic filter
    async fn get(&self, topic_filter: &TopicFilter) -> Result<Vec<(TopicName, Retain)>> {
        // Circuit breaker OPEN: return empty without calling get_message.
        if self.circuit_breaker.is_blocked() {
            return Ok(Vec::new());
        }

        let this = self.clone();
        let tf = topic_filter.clone();
        let result = async move { this.get_message(&tf).await }
            .spawn(&self.exec)
            .result()
            .timeout(futures_time::time::Duration::from_millis(5000))
            .await;

        match result {
            Ok(inner) => {
                // inner is the output of .result(), which is Result<T, JoinError>
                // where T is the output of get_message (Result<Vec<...>, anyhow::Error>)
                // So inner is Result<Result<Vec<...>, anyhow::Error>, JoinError>
                match inner {
                    Ok(Ok(retains)) => {
                        // Task succeeded, get_message returned Ok
                        self.circuit_breaker.record_success();
                        Ok(retains)
                    }
                    Ok(Err(e)) => {
                        // Task succeeded, but get_message returned Err (storage failure)
                        log::warn!("retainer get_message error: {:?}", e);
                        self.circuit_breaker.record_failure();
                        Ok(Vec::new())
                    }
                    Err(_e) => {
                        // Task failed (panic, etc.)
                        log::warn!("retainer get task error");
                        self.circuit_breaker.record_failure();
                        Ok(Vec::new())
                    }
                }
            }
            Err(_) => {
                // Timeout
                log::warn!("retainer get timeout");
                self.circuit_breaker.record_failure();
                Ok(Vec::new())
            }
        }
    }

    async fn get_all_paginated(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<(Vec<(TopicName, Retain, Option<Duration>)>, bool)> {
        if self.circuit_breaker.is_blocked() {
            return Ok((Vec::new(), false));
        }
        self._get_all_paginated(offset, limit)
            .timeout(futures_time::time::Duration::from_secs(60))
            .await
            .map_err(|_e| anyhow!("get_all_paginated timeout"))?
    }

    #[inline]
    async fn count(&self) -> isize {
        if self.circuit_breaker.is_blocked() {
            return -1;
        }
        self.get_retain_count().await as isize
    }

    #[inline]
    async fn max(&self) -> isize {
        if self.circuit_breaker.is_blocked() {
            return -1;
        }
        let this = self.inner.clone();

        let val = self.storage_messages_max.clone();
        let value_ref = val
            .call_timeout(async move { this.storage_messages_max_get().await }, Duration::from_millis(3000))
            .await;
        // value_ref and val are now distinct locals; the borrow is visible
        // to the async state machine and drop order is well-defined.
        match value_ref.get() {
            Ok(max) => max.copied().unwrap_or(-1),
            Err(e) => {
                log::warn!("retainer max error: {e:?}");
                -1
            }
        }
    }

    #[inline]
    fn stats_merge_mode(&self) -> StatsMergeMode {
        match self.storage_type {
            #[cfg(feature = "sled")]
            StorageType::Sled => StatsMergeMode::Max,
            #[cfg(feature = "redis")]
            StorageType::Redis => StatsMergeMode::Max,
            #[allow(unreachable_patterns)]
            _ => StatsMergeMode::None,
        }
    }

    #[inline]
    fn retain_sync_mode(&self) -> RetainSyncMode {
        match self.storage_type {
            #[cfg(feature = "redis")]
            StorageType::Redis => RetainSyncMode::TopicOnly,
            #[allow(unreachable_patterns)]
            _ => RetainSyncMode::Full,
        }
    }

    async fn sync_retain_topic(
        &self,
        topic: &TopicName,
        expiry_interval: Option<Duration>,
        is_set: bool,
    ) -> Result<()> {
        let msg = if is_set {
            SyncMsg::Add(topic.clone(), expiry_interval)
        } else {
            SyncMsg::Remove(topic.clone())
        };
        self.inner.sync_tx.clone().send(msg).await.map_err(|e| anyhow!(e))?;
        #[cfg(feature = "rate-counter")]
        self.sync_rate_counter.inc();
        Ok(())
    }
}
