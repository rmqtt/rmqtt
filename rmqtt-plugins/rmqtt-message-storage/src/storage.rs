use std::collections::BTreeSet;
use std::convert::From as _;
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
use once_cell::sync::OnceCell;
use rust_box::task_exec_queue::{Builder, SpawnExt, TaskExecQueue};
use tokio::{runtime::Handle, sync::RwLock, task::spawn_blocking, time::sleep};

use rmqtt::{
    message::MessageManager,
    retain::RetainTree,
    types::{
        ClientId, From, MsgID, NodeId, Publish, SharedGroup, StoredMessage, TimestampMillis, Topic,
        TopicFilter,
    },
    utils::timestamp_millis,
    Result,
};

use rmqtt::topic::Level;
use rmqtt_storage::{DefaultStorageDB, Map, StorageMap};

use crate::config::PluginConfig;

type TopicTreeType = Arc<RwLock<RetainTree<MsgID>>>;
type TopicListType = Arc<RwLock<BTreeSet<(TimestampMillis, Topic)>>>;

const DATA: &[u8] = b"data";
const FORWARDED_PREFIX: &[u8] = b"fwd_";

type Msg = ((From, Publish, Duration, MsgID), Option<Vec<(ClientId, Option<(TopicFilter, SharedGroup)>)>>);

static INSTANCE: OnceCell<StorageMessageManager> = OnceCell::new();

#[inline]
pub(crate) async fn get_or_init(
    node_id: NodeId,
    cfg: Arc<PluginConfig>,
    storage_db: DefaultStorageDB,
    should_merge_on_get: bool,
) -> Result<&'static StorageMessageManager> {
    if let Some(msg_mgr) = INSTANCE.get() {
        return Ok(msg_mgr);
    }
    let msg_mgr = StorageMessageManager::new(node_id, cfg, storage_db, should_merge_on_get).await?;
    INSTANCE.set(msg_mgr).map_err(|_| anyhow!("init error!"))?;
    if let Some(msg_mgr) = INSTANCE.get() {
        Ok(msg_mgr)
    } else {
        unreachable!()
    }
}

pub struct StorageMessageManager {
    inner: Arc<StorageMessageManagerInner>,
    pub(crate) exec: TaskExecQueue,
}

impl StorageMessageManager {
    #[inline]
    async fn new(
        _node_id: NodeId,
        cfg: Arc<PluginConfig>,
        storage_db: DefaultStorageDB,
        should_merge_on_get: bool,
    ) -> Result<StorageMessageManager> {
        let id_generater = StorageMessageManagerInner::storage_new_msg_id_generater(&storage_db).await?;
        log::info!("current msg_id: {}", id_generater.load(Ordering::SeqCst));
        let messages_received_max =
            StorageMessageManagerInner::storage_new_messages_counter(&storage_db).await?;
        log::info!("messages_received_max: {}", messages_received_max.load(Ordering::SeqCst));
        let (exec, msg_tx, msg_queue_count) = Self::serve(cfg)?;

        let inner = Arc::new(StorageMessageManagerInner {
            storage_db,
            topic_tree: Default::default(),
            topic_list: Default::default(),
            messages_received_max,
            msg_tx,
            msg_queue_count,
            id_generater,
            should_merge_on_get,
        });
        Ok(Self { inner, exec })
    }

    fn serve(cfg: Arc<PluginConfig>) -> Result<(TaskExecQueue, mpsc::Sender<Msg>, Arc<AtomicIsize>)> {
        let queue_max = 300_000;
        let (exec, task_runner) = Builder::default().workers(1000).queue_max(queue_max).build();

        tokio::spawn(async move {
            task_runner.await;
        });

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
                        if let Err(e) = msg_mgr._batch_msg_forwardeds(msgs).await {
                            log::warn!("{:?}", e);
                        }
                        msg_fwds_count1.fetch_sub(1, Ordering::SeqCst);
                    });
                }
                log::error!("Recv failed because receiver is gone");
            }
        });

        //cleanup ...
        tokio::spawn(async move {
            let max_limit = cfg.cleanup_count;
            sleep(Duration::from_secs(20)).await;
            let mut now = std::time::Instant::now();
            let mut total_removeds = 0;
            loop {
                let removeds = if let Some(msg_mgr) = INSTANCE.get() {
                    spawn_blocking(move || {
                        let curr_time = timestamp_millis();
                        Handle::current().block_on(async move {
                            let removed_topics = {
                                let mut topic_list = msg_mgr.topic_list.write().await;
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
                                msg_mgr.topic_tree.write().await.remove(t);
                            }
                            removed_topics.len()
                        })
                    })
                    .await
                } else {
                    Ok(0)
                }
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
                sleep(Duration::from_secs(30)).await; //@TODO config enable
                now = std::time::Instant::now();
                total_removeds = 0;
            }
        });

        Ok((exec, msg_tx, msg_queue_count))
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
    pub(crate) storage_db: DefaultStorageDB,
    pub(crate) topic_tree: TopicTreeType,
    topic_list: TopicListType,

    messages_received_max: AtomicIsize,

    msg_tx: mpsc::Sender<Msg>,
    pub(crate) msg_queue_count: Arc<AtomicIsize>,

    id_generater: AtomicUsize,
    should_merge_on_get: bool,
}

impl StorageMessageManagerInner {
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
                                    log::warn!("Topic::from_str error, {:?}", e);
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
                        log::warn!("Restore topic tree error, {:?}", e);
                    }
                },
                Err(e) => {
                    log::warn!("Restore topic tree error, {:?}", e);
                }
            }
        }
        log::info!("restore count_all: {}, count: {}", count_all, count);
        Ok(())
    }

    #[inline]
    async fn storage_save_msg_id(&self) -> Result<()> {
        let curr_msg_id = self.id_generater.load(Ordering::SeqCst);
        self.storage_db.insert("id_generater", &curr_msg_id).await?;
        Ok(())
    }

    #[inline]
    async fn storage_new_msg_id_generater(storage_db: &DefaultStorageDB) -> Result<AtomicUsize> {
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
    async fn storage_new_messages_counter(storage_db: &DefaultStorageDB) -> Result<AtomicIsize> {
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
    async fn _batch_msg_forwardeds(&self, msgs: Vec<Msg>) -> Result<()> {
        if let Err(e) = self
            .storage_save_msg_id()
            .timeout(futures_time::time::Duration::from_millis(5000))
            .await
            .map_err(|_e| anyhow!("storage_save_msg_id timeout"))?
        {
            log::warn!("save message id error, {:?}", e);
            return Ok(());
        }

        let mut count = 0;
        for ((from, publish, expiry_interval, msg_id), forwardeds) in msgs {
            let mut topic = match Topic::from_str(&publish.topic) {
                Err(e) => {
                    log::warn!("Topic::from_str error, {:?}", e);
                    continue;
                }
                Ok(topic) => topic,
            };
            let expiry_time_at = timestamp_millis() + expiry_interval.as_millis() as i64;

            let smsg = StoredMessage { msg_id, from, publish, expiry_time_at };

            //received messages
            let msg_key = msg_id.to_be_bytes();
            let msg_map = match self
                .storage_db
                .map(msg_key, Some(expiry_interval.as_millis() as TimestampMillis))
                .timeout(futures_time::time::Duration::from_millis(5000))
                .await
                .map_err(|_e| anyhow!("storage_db.map timeout"))?
            {
                Ok(map) => map,
                Err(e) => {
                    log::warn!("store to db error, map_expire(..), {:?}, message: {:?}", e, smsg);
                    continue;
                }
            };
            if let Err(e) = msg_map
                .insert(DATA, &smsg)
                .timeout(futures_time::time::Duration::from_millis(5000))
                .await
                .map_err(|_e| anyhow!("map.insert timeout"))?
            {
                log::warn!("store to db error, {:?}, message: {:?}", e, smsg);
                continue;
            }

            if let Some(forwardeds) = forwardeds {
                self._forwardeds(&msg_map, forwardeds).await?;
            }

            //topic
            topic.push(Level::Normal(msg_id.to_string()));
            self.topic_tree.write().await.insert(&topic, msg_id);
            self.topic_list.write().await.insert((expiry_time_at, topic));

            count += 1;
        }
        self.messages_received_count_add(count);
        if let Err(e) = self
            .storage_messages_counter_add(count)
            .timeout(futures_time::time::Duration::from_millis(5000))
            .await
            .map_err(|_e| anyhow!("storage_messages_counter_add timeout"))?
        {
            log::warn!("messages_received_counter add error, {:?}", e);
        }

        Ok(())
    }

    #[inline]
    async fn _forwardeds(
        &self,
        msg_map: &StorageMap,
        forwardeds: Vec<(ClientId, Option<(TopicFilter, SharedGroup)>)>,
    ) -> Result<()> {
        for (client_id, opts) in forwardeds {
            if let Err(e) = msg_map
                .insert(Self::make_forwarded_key(&client_id), &opts)
                .timeout(futures_time::time::Duration::from_millis(5000))
                .await
                .map_err(|_e| anyhow!("_forwardeds insert timeout"))?
            {
                log::warn!(
                    "_forwardeds error, client_id: {:?}, msg_map name: {:?}, error: {:?}",
                    client_id,
                    String::from_utf8_lossy(msg_map.name()),
                    e,
                );
            }
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

        log::debug!("_get matcheds msg_ids: {:?}", matcheds);
        let matcheds = futures::future::join_all(matcheds.into_iter().map(|msg_id| async move {
            let msg_key = msg_id.to_be_bytes();
            let msg_map = self.storage_db.map(msg_key, None).await;
            match msg_map {
                Ok(mut msg_map) => {
                    let is_forwarded = self
                        ._is_forwarded(&mut msg_map, client_id, topic_filter, group)
                        .await
                        .unwrap_or_default();

                    if is_forwarded {
                        None
                    } else if let Ok(Some(msg)) = inner._get_message(&msg_map).await {
                        log::debug!("_get msg: {:?}, msg.is_expiry(): {}", msg, msg.is_expiry());
                        if msg.is_expiry() {
                            None
                        } else {
                            let opts = group.map(|g| (TopicFilter::from(topic_filter), g.clone()));
                            if let Err(e) = msg_map.insert(Self::make_forwarded_key(client_id), &opts).await {
                                log::warn!("_get::insert error, {:?}", e);
                            }
                            Some((msg_id, msg.from, msg.publish))
                        }
                    } else {
                        None
                    }
                }
                Err(e) => {
                    log::warn!("_get new map error, {:?}", e);
                    None
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
            log::debug!("_is_forwarded contains_key client_id: {:?}", client_id);
            return Ok(true);
        }
        if let Some(group) = group {
            let mut iter =
                msg_map.prefix_iter::<_, Option<(TopicFilter, SharedGroup)>>(FORWARDED_PREFIX).await?;
            while let Some(item) = iter.next().await {
                log::debug!("_is_forwarded item: {:?}", item);
                match item {
                    Ok((_, Some((tf, g)))) => {
                        if g == group && tf == topic_filter {
                            return Ok(true);
                        }
                    }
                    Ok((_, None)) => {}
                    Err(e) => {
                        log::warn!("traverse forwardeds error, {:?}", e);
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
}

#[async_trait]
impl MessageManager for &'static StorageMessageManager {
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
        sub_client_ids: Option<Vec<(ClientId, Option<(TopicFilter, SharedGroup)>)>>,
    ) -> Result<()> {
        let res = self
            .msg_tx
            .clone()
            .send(((from, p, expiry_interval, msg_id), sub_client_ids))
            .timeout(futures_time::time::Duration::from_millis(3500))
            .await
            .map_err(|e| anyhow!(e));
        match res {
            Ok(Ok(())) => {
                self.msg_queue_count.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Ok(Err(e)) => {
                log::warn!("StorageMessageManager set error, {:?}", e);
                Err(anyhow!(e))
            }
            Err(e) => {
                log::warn!("StorageMessageManager store timeout, {:?}", e);
                Err(anyhow!(e))
            }
        }
    }

    #[inline]
    async fn get(
        &self,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
    ) -> Result<Vec<(MsgID, From, Publish)>> {
        let now = std::time::Instant::now();
        let inner = self.inner.clone();
        let client_id = ClientId::from(client_id);
        let topic_filter = TopicFilter::from(topic_filter);
        let group = group.cloned();
        let matcheds = async move { inner._get(&client_id, &topic_filter, group.as_ref()).await }
            .spawn(&self.exec)
            .result()
            .timeout(futures_time::time::Duration::from_millis(3000))
            .await;
        let matcheds = match matcheds {
            Ok(Ok(Ok(res))) => res,
            Ok(Ok(Err(e))) => {
                log::error!("StorageMessageManager get error, {:?}", e.to_string());
                return Err(e);
            }
            Ok(Err(e)) => {
                log::error!("StorageMessageManager get error, {:?}", e.to_string());
                return Err(anyhow!(e.to_string()));
            }
            Err(e) => {
                log::warn!("StorageMessageManager get timeout, {:?}", e);
                vec![]
            }
        };
        if now.elapsed().as_millis() > 900 {
            log::info!(
                "StorageMessageManager::get cost time: {:?}, waiting_count: {:?}",
                now.elapsed(),
                self.exec.waiting_count()
            );
        }
        Ok(matcheds)
    }

    #[inline]
    fn should_merge_on_get(&self) -> bool {
        self.should_merge_on_get
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
