use std::borrow::Cow;
use std::convert::From as _;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicIsize, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_time::future::FutureExt;

use rmqtt::{
    anyhow::anyhow,
    async_trait::async_trait,
    futures::channel::mpsc,
    futures::{SinkExt, StreamExt},
    log,
    once_cell::sync::OnceCell,
    timestamp_millis, tokio,
    tokio::sync::RwLock,
    tokio::time::sleep,
    NodeId, Retain, TimestampMillis, TopicName,
};

use rmqtt::{MqttError, Result, Topic, TopicFilter};

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
        let retain_count = Arc::new(AtomicUsize::new(storage_db.len().await?));
        let retain_count_utime = Arc::new(AtomicI64::new(timestamp_millis()));
        let inner = Arc::new(RetainerInner {
            cfg,
            storage_db,
            msg_tx,
            msg_queue_count,
            retain_enable,
            retain_count,
            retain_count_utime,
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
                            log::warn!("{:?}", e);
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
    retain_count: Arc<AtomicUsize>,
    retain_count_utime: Arc<AtomicI64>,
}

impl RetainerInner {
    #[inline]
    async fn _batch_store(&self, msgs: Vec<Msg>) -> Result<()> {
        let (max_retained_messages, max_payload_size, cfg_expiry_interval) = {
            let cfg = self.cfg.read().await;
            let expiry_interval = if cfg.expiry_interval.is_zero() {
                None
            } else {
                Some(cfg.expiry_interval.as_millis() as i64)
            };
            (cfg.max_retained_messages as usize, *cfg.max_payload_size, expiry_interval)
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
                    log::warn!("remove from db error, remove(..), {:?}, topic_name: {:?}", e, topic_name);
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
                        log::warn!(
                            "store to db error, check_constraints(..), {:?}, message: {:?}",
                            e,
                            retain
                        );
                        continue;
                    }
                }

                //add retain messagge
                let expiry_interval_millis = if let Some(expiry_interval) = expiry_interval {
                    Some(expiry_interval.as_millis() as i64)
                } else {
                    cfg_expiry_interval
                };

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
                    log::warn!("store to db error, insert(..), {:?}, message: {:?}", e, smsg);
                    continue;
                };
                if let Some(expiry_interval_millis) = expiry_interval_millis {
                    if let Err(e) =
                        self.storage_db.expire(store_topic_name.as_slice(), expiry_interval_millis).await
                    {
                        log::warn!("store to db error, expire(..), {:?}, message: {:?}", e, smsg);
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
            log::warn!("messages_received_counter add error, {:?}", e);
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
            log::warn!("Retain message payload exceeding limit, topic: {:?}, retain: {:?}", topic, retain);
            return Ok(false);
        }

        //@TODO ...
        if max_retained_messages > 0 && self.get_retain_count().await? >= max_retained_messages {
            log::warn!(
                "The retained message has exceeded the maximum limit of: {}, topic: {:?}, retain: {:?}",
                max_retained_messages,
                topic,
                retain
            );
            return Ok(false);
        }

        Ok(true)
    }

    #[inline]
    async fn get_retain_count(&self) -> Result<usize> {
        let retain_count = if (timestamp_millis() - self.retain_count_utime.load(Ordering::SeqCst)) < 3000 {
            self.retain_count.load(Ordering::SeqCst)
        } else {
            let retain_count = self.storage_db.len().await?;
            self.retain_count.store(retain_count, Ordering::SeqCst);
            self.retain_count_utime.store(timestamp_millis(), Ordering::SeqCst);
            retain_count
        };
        let retain_count = if retain_count > 0 { retain_count - 1 } else { retain_count };
        log::info!("retain_count: {}", retain_count);
        Ok(retain_count)
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
        let mut iter = db.scan([RETAIN_MESSAGES_PREFIX, topic_filter_pattern.as_bytes()].concat()).await?;
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
                    log::error!("{:?}", e);
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
                    log::error!("{:?}", e);
                }
            }
        }
        Ok(retains)
    }
}

#[async_trait]
impl RetainStorage for &'static Retainer {
    ///topic - concrete topic
    async fn set(&self, topic: &TopicName, retain: Retain) -> Result<()> {
        if !self.retain_enable.load(Ordering::SeqCst) {
            log::error!("{}", ERR_NOT_SUPPORTED);
            return Ok(());
        }
        let res = self
            .msg_tx
            .clone()
            .send((topic.clone(), retain, None))
            .timeout(futures_time::time::Duration::from_millis(3500))
            .await
            .map_err(|e| anyhow!(e));
        match res {
            Ok(Ok(())) => {
                self.msg_queue_count.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Ok(Err(e)) => {
                log::error!("Retainer set error, {:?}", e);
                Err(MqttError::from(e.to_string()))
            }
            Err(e) => {
                log::warn!("Retainer store timeout, {:?}", e);
                Err(MqttError::from(e.to_string()))
            }
        }
    }

    ///topic_filter - Topic filter
    async fn get(&self, topic_filter: &TopicFilter) -> Result<Vec<(TopicName, Retain)>> {
        if !self.retain_enable.load(Ordering::SeqCst) {
            log::error!("{}", ERR_NOT_SUPPORTED);
            Ok(Vec::new())
        } else {
            Ok(self.get_message(topic_filter).await?)
        }
    }

    #[inline]
    async fn count(&self) -> isize {
        -1
    }

    #[inline]
    async fn max(&self) -> isize {
        self.storage_messages_max_get().await.unwrap_or_default()
    }
}
