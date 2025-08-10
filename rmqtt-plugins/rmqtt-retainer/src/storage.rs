use std::borrow::Cow;
use std::convert::From as _;
use std::future::Future;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rmqtt::{
    anyhow::anyhow,
    async_trait::async_trait,
    futures::channel::mpsc,
    futures::{SinkExt, StreamExt},
    futures_time::{self, future::FutureExt},
    log,
    once_cell::sync::OnceCell,
    tokio,
    tokio::sync::RwLock,
    tokio::time::sleep,
};

use rmqtt::{
    timestamp_millis, MqttError, NodeId, Result, Retain, StatsMergeMode, TimestampMillis, Topic, TopicFilter,
    TopicName,
};

use rmqtt::broker::RetainStorage;
use rmqtt_storage::DefaultStorageDB;

use crate::config::PluginConfig;
use crate::ERR_NOT_SUPPORTED;

type Msg = (TopicName, Retain, Option<Duration>);

type StoredMsg = (Retain, Option<TimestampMillis>);

const RETAIN_MESSAGES_MAX: &[u8] = b"m|";

const RETAIN_MESSAGES_PREFIX: &[u8] = b"p|";

static INSTANCE: OnceCell<Retainer> = OnceCell::new();

#[inline]
pub(crate) async fn get_or_init(
    node_id: NodeId,
    cfg: Arc<RwLock<PluginConfig>>,
    storage_db: DefaultStorageDB,
    retain_enable: Arc<AtomicBool>,
) -> Result<&'static Retainer> {
    if let Some(msg_mgr) = INSTANCE.get() {
        return Ok(msg_mgr);
    }
    let msg_mgr = Retainer::new(node_id, cfg, storage_db, retain_enable).await?;
    INSTANCE.set(msg_mgr).map_err(|_| anyhow!("init error!"))?;
    if let Some(msg_mgr) = INSTANCE.get() {
        Ok(msg_mgr)
    } else {
        unreachable!()
    }
}

pub(crate) struct Retainer {
    inner: Arc<RetainerInner>,
}

impl Retainer {
    #[inline]
    async fn new(
        _node_id: NodeId,
        cfg: Arc<RwLock<PluginConfig>>,
        storage_db: DefaultStorageDB,
        retain_enable: Arc<AtomicBool>,
    ) -> Result<Retainer> {
        let (msg_tx, msg_queue_count) = Self::serve(cfg.clone())?;
        let storage_messages_count = ValueCached::new(Duration::from_millis(3000));
        let storage_messages_max = ValueCached::new(Duration::from_millis(3000));
        let inner = Arc::new(RetainerInner {
            cfg,
            storage_db,
            msg_tx,
            msg_queue_count,
            retain_enable,
            storage_messages_count,
            storage_messages_max,
        });
        Ok(Self { inner })
    }

    fn serve(_cfg: Arc<RwLock<PluginConfig>>) -> Result<(mpsc::Sender<Msg>, Arc<AtomicIsize>)> {
        let msg_queue_count = Arc::new(AtomicIsize::new(0));
        let msg_queue_count1 = msg_queue_count.clone();
        let (msg_tx, mut msg_rx) = mpsc::channel::<Msg>(300_000);
        tokio::spawn(async move {
            loop {
                if INSTANCE.get().is_some() {
                    break;
                }
                sleep(Duration::from_millis(10)).await;
            }
            if let Some(msg_mgr) = INSTANCE.get() {
                let msg_fwds_count = Arc::new(AtomicIsize::new(0));

                let mut merger_msgs = Vec::new();
                while let Some(msg) = msg_rx.next().await {
                    merger_msgs.push(msg);
                    while merger_msgs.len() < 500 {
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

                    msg_queue_count1.fetch_sub(msgs.len() as isize, Ordering::Relaxed);
                    msg_fwds_count.fetch_add(1, Ordering::SeqCst);
                    while msg_fwds_count.load(Ordering::SeqCst) > 500 {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    }

                    let msg_fwds_count1 = msg_fwds_count.clone();
                    tokio::spawn(async move {
                        if let Err(e) = msg_mgr._batch_store(msgs).await {
                            log::warn!("{e:?}");
                        }
                        msg_fwds_count1.fetch_sub(1, Ordering::SeqCst);
                    });
                }
                log::error!("Recv failed because receiver is gone");
            }
        });

        Ok((msg_tx, msg_queue_count))
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
    pub(crate) msg_queue_count: Arc<AtomicIsize>,
    retain_enable: Arc<AtomicBool>,
    // retain_count: Arc<AtomicUsize>,
    // retain_count_utime: Arc<AtomicI64>,
    storage_messages_count: ValueCached<usize>,
    storage_messages_max: ValueCached<isize>,
}

impl RetainerInner {
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
                    .map_err(|_e| MqttError::from("storage_db.remove timeout"))?
                {
                    log::warn!("remove from db error, remove(..), {e:?}, topic_name: {topic_name:?}");
                };
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
                    .map_err(|_e| MqttError::from("storage_db.insert timeout"))?
                {
                    log::warn!("store to db error, insert(..), {e:?}, message: {smsg:?}");
                    continue;
                };
                if let Some(expiry_interval_millis) = expiry_interval_millis {
                    if let Err(e) =
                        self.storage_db.expire(store_topic_name.as_slice(), expiry_interval_millis).await
                    {
                        log::warn!("store to db error, expire(..), {e:?}, message: {smsg:?}");
                        continue;
                    }
                }

                count += 1;
            }
        }

        if let Err(e) = self
            .storage_messages_max_add(count)
            .timeout(futures_time::time::Duration::from_millis(5000))
            .await
            .map_err(|_e| MqttError::from("storage_messages_max_add timeout"))?
        {
            log::warn!("messages_received_counter add error, {e:?}");
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

        if max_retained_messages > 0 && self.get_retain_count().await >= max_retained_messages {
            log::warn!(
                "The retained message has exceeded the maximum limit of: {max_retained_messages}, topic: {topic:?}, retain: {retain:?}"
            );
            return Ok(false);
        }

        Ok(true)
    }

    #[inline]
    async fn get_retain_count(&self) -> usize {
        let db = self.storage_db.clone();
        let count = self
            .storage_messages_count
            .call_timeout(async move { db.len().await.map_err(MqttError::from) }, Duration::from_millis(3000))
            .await
            .get()
            .copied()
            .unwrap_or_default();
        if count > 0 {
            count - 1
        } else {
            count
        }
    }

    #[inline]
    async fn storage_messages_max_add(&self, vals: isize) -> Result<()> {
        self.storage_db.counter_incr(RETAIN_MESSAGES_MAX, vals).await?;
        Ok(())
    }

    #[inline]
    async fn storage_messages_max_get(&self) -> Result<isize> {
        Ok(self.storage_db.counter_get(RETAIN_MESSAGES_MAX).await?.unwrap_or_default())
    }

    #[inline]
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
        let topic_filter_pattern = Self::topic_filter_to_pattern(topic_filter);
        let mut matched_topics = Vec::new();
        let mut db = self.storage_db.clone();
        let mut iter = match db.scan([RETAIN_MESSAGES_PREFIX, topic_filter_pattern.as_bytes()].concat()).await
        {
            Err(e) => {
                log::error!("{e:?}");
                return Ok(Vec::new());
            }
            Ok(iter) => iter,
        };
        while let Some(key) = iter.next().await {
            match key {
                Ok(key) => {
                    if !key.starts_with(RETAIN_MESSAGES_PREFIX) {
                        continue;
                    }
                    if topic.matches_str(&String::from_utf8_lossy(&key[RETAIN_MESSAGES_PREFIX.len()..])) {
                        matched_topics.push(key);
                    }
                }
                Err(e) => {
                    log::error!("{e:?}");
                }
            }
        }
        drop(iter);

        let mut retains = Vec::new();
        for key in matched_topics {
            match db.get::<_, StoredMsg>(key.as_slice()).await {
                Ok(Some((retain, expiry_time_at))) => {
                    let topic_name = TopicName::from(
                        String::from_utf8_lossy(&key[RETAIN_MESSAGES_PREFIX.len()..]).as_ref(),
                    );
                    if let Some(expiry_time_at) = expiry_time_at {
                        if expiry_time_at > timestamp_millis() {
                            retains.push((topic_name, retain));
                        }
                    } else {
                        retains.push((topic_name, retain))
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    log::error!("{e:?}");
                }
            }
        }
        Ok(retains)
    }
}

#[async_trait]
impl RetainStorage for &'static Retainer {
    #[inline]
    fn enable(&self) -> bool {
        true
    }

    ///topic - concrete topic
    async fn set(&self, topic: &TopicName, retain: Retain, expiry_interval: Option<Duration>) -> Result<()> {
        if !self.retain_enable.load(Ordering::SeqCst) {
            log::error!("{ERR_NOT_SUPPORTED}");
            return Ok(());
        }

        let res = self
            .msg_tx
            .clone()
            .send((topic.clone(), retain, expiry_interval))
            .timeout(futures_time::time::Duration::from_millis(3500))
            .await
            .map_err(|e| anyhow!(e));
        match res {
            Ok(Ok(())) => {
                self.msg_queue_count.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Ok(Err(e)) => {
                log::error!("Retainer set error, {e:?}");
                Err(MqttError::from(e.to_string()))
            }
            Err(e) => {
                log::warn!("Retainer store timeout, {e:?}");
                Err(MqttError::from(e.to_string()))
            }
        }
    }

    ///topic_filter - Topic filter
    async fn get(&self, topic_filter: &TopicFilter) -> Result<Vec<(TopicName, Retain)>> {
        if !self.retain_enable.load(Ordering::SeqCst) {
            log::error!("{ERR_NOT_SUPPORTED}");
            Ok(Vec::new())
        } else {
            Ok(self.get_message(topic_filter).await?)
        }
    }

    #[inline]
    async fn count(&self) -> isize {
        self.get_retain_count().await as isize
    }

    #[inline]
    async fn max(&self) -> isize {
        self.storage_messages_max
            .call_timeout(self.storage_messages_max_get(), Duration::from_millis(3000))
            .await
            .get()
            .copied()
            .unwrap_or(-1)
    }

    #[inline]
    fn stats_merge_mode(&self) -> StatsMergeMode {
        StatsMergeMode::Max
    }
}

#[derive(Clone)]
pub struct ValueCached<T> {
    inner: Arc<RwLock<ValueCachedInner<T>>>,
    guard: Arc<RwLock<()>>,
}

pub struct ValueCachedInner<T> {
    cached_val: Option<Result<T>>,
    expire_interval: Duration,
    instant: Instant,
}

impl<T> ValueCached<T> {
    #[inline]
    pub fn new(expire_interval: Duration) -> Self {
        Self {
            inner: Arc::new(RwLock::new(ValueCachedInner {
                cached_val: None,
                expire_interval,
                instant: Instant::now(),
            })),
            guard: Arc::new(RwLock::new(())),
        }
    }

    #[inline]
    #[allow(unused)]
    pub async fn call<F>(&self, f: F) -> ValueRef<'_, T>
    where
        F: Future<Output = Result<T>> + Send + 'static,
    {
        self._call_timeout(f, None).await
    }

    #[inline]
    pub async fn call_timeout<F>(&self, f: F, timeout: Duration) -> ValueRef<'_, T>
    where
        F: Future<Output = Result<T>> + Send + 'static,
    {
        self._call_timeout(f, Some(timeout)).await
    }

    #[inline]
    async fn _call_timeout<F>(&self, f: F, timeout: Option<Duration>) -> ValueRef<'_, T>
    where
        F: Future<Output = Result<T>> + Send + 'static,
    {
        let inst = std::time::Instant::now();
        let (call_enable, updating) = {
            let mut enable = false;
            #[allow(unused_assignments)]
            let mut updating = false;
            loop {
                if let Ok(_guard) = self.guard.try_read() {
                    let inner_rl = self.inner.read().await;
                    enable = inner_rl.cached_val.is_none() || inner_rl.is_expired();
                    updating = false;
                    break;
                }
                updating = true;

                if self.inner.read().await.cached_val.is_some() {
                    break;
                }

                tokio::time::sleep(Duration::from_millis(10)).await;
                if let Some(t) = timeout.as_ref() {
                    if inst.elapsed() > *t {
                        break;
                    }
                }
            }
            (enable, updating)
        };

        let (cached, updating) = if call_enable {
            if let Ok(_guard) = self.guard.try_write() {
                let val = if let Some(t) = timeout {
                    match tokio::time::timeout(t, f).await {
                        Ok(Ok(v)) => Ok(v),
                        Ok(Err(e)) => Err(e),
                        Err(e) => Err(MqttError::from(anyhow!(e))),
                    }
                } else {
                    f.await
                };
                let mut inner_wl = self.inner.write().await;
                inner_wl.cached_val = Some(val);
                inner_wl.instant = Instant::now();
                (false, false)
            } else {
                #[allow(unused_assignments)]
                let mut updating = false;
                loop {
                    if let Ok(_guard) = self.guard.try_read() {
                        updating = false;
                        break;
                    }
                    updating = true;
                    if self.inner.read().await.cached_val.is_some() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    if let Some(t) = timeout.as_ref() {
                        if inst.elapsed() > *t {
                            break;
                        }
                    }
                }
                (true, updating)
            }
        } else {
            (true, updating)
        };
        ValueRef { val_guard: self.inner.read().await, cached, updating }
    }
}

impl<T> ValueCachedInner<T> {
    #[inline]
    fn is_expired(&self) -> bool {
        self.instant.elapsed() > self.expire_interval
    }
}

pub struct ValueRef<'a, T> {
    val_guard: tokio::sync::RwLockReadGuard<'a, ValueCachedInner<T>>,
    cached: bool,
    updating: bool,
}

impl<T> ValueRef<'_, T> {
    #[inline]
    pub fn get(&self) -> Result<&T> {
        if let Some(val) = self.val_guard.cached_val.as_ref() {
            Ok(val.as_ref().map_err(|e| anyhow!(e.to_string()))?)
        } else {
            Err(MqttError::from("Timeout"))
        }
    }

    #[inline]
    #[allow(unused)]
    pub fn is_cached(&self) -> bool {
        self.cached
    }

    #[inline]
    #[allow(unused)]
    pub fn is_updating(&self) -> bool {
        self.updating
    }
}
