use std::collections::BTreeSet;
use std::convert::From as _;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures_time::future::FutureExt;

use rmqtt::{
    anyhow::anyhow,
    async_trait::async_trait,
    futures,
    futures::channel::mpsc,
    futures::{SinkExt, StreamExt},
    log,
    ntex_mqtt::TopicLevel,
    once_cell::sync::OnceCell,
    rust_box::task_exec_queue::{Builder, SpawnExt, TaskExecQueue},
    timestamp_millis, tokio,
    tokio::sync::RwLock,
    tokio::time::sleep,
    NodeId, TimestampMillis,
};

use rmqtt::{
    broker::retain::RetainTree, broker::MessageManager, ClientId, From, MqttError, MsgID, Publish, Result,
    SharedGroup, StoredMessage, Topic, TopicFilter,
};

use rmqtt::tokio::runtime::Handle;
use rmqtt::tokio::task::spawn_blocking;
use rmqtt_storage::{DefaultStorageDB, Map, StorageMap};

use crate::config::PluginConfig;

type Msg = (From, Publish, Duration, MsgID);
type TopicTreeType = Arc<RwLock<RetainTree<MsgID>>>;
type TopicListType = Arc<RwLock<BTreeSet<(TimestampMillis, Topic)>>>;

const DATA: &[u8] = b"data";
const FORWARDED_PREFIX: &[u8] = b"fwd_";

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
                let mut merger_msgs = Vec::new();
                while let Some((from, publish, expiry_interval, msg_id)) = msg_rx.next().await {
                    merger_msgs.push((from, publish, expiry_interval, msg_id));
                    while merger_msgs.len() < 1000 {
                        match tokio::time::timeout(Duration::from_millis(0), msg_rx.next()).await {
                            Ok(Some((from, publish, expiry_interval, msg_id))) => {
                                merger_msgs.push((from, publish, expiry_interval, msg_id));
                            }
                            _ => break,
                        }
                    }
                    log::debug!("merger_msgs.len: {}", merger_msgs.len());
                    //merge and send
                    let msgs = std::mem::take(&mut merger_msgs);
                    msg_queue_count1.fetch_sub(msgs.len() as isize, Ordering::SeqCst);
                    tokio::spawn(msg_mgr._batch_set_v2(msgs));
                }
                log::error!("Recv failed because receiver is gone");
            }
        });

        //cleanup ...
        tokio::spawn(async move {
            let max_limit = cfg.cleanup_count;
            sleep(Duration::from_secs(20)).await;
            loop {
                let now = std::time::Instant::now();
                let removeds = if let Some(msg_mgr) = INSTANCE.get() {
                    spawn_blocking(move || {
                        let curr_time = timestamp_millis();
                        Handle::current().block_on(async move {
                            let removed_topics = {
                                let mut topic_list = msg_mgr.topic_list.write().await;
                                let mut removeds = Vec::new();
                                loop {
                                    if let Some((expiry_time_at, _)) = topic_list.first() {
                                        if *expiry_time_at > curr_time || removeds.len() > max_limit {
                                            break;
                                        }
                                        if let Some((_, t)) = topic_list.pop_first() {
                                            removeds.push(t)
                                        } else {
                                            break;
                                        }
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
                if removeds > 0 {
                    log::info!(
                        "remove_expired_messages, removeds: {:?} cost time: {:?}",
                        removeds,
                        now.elapsed()
                    );
                }
                if removeds >= max_limit {
                    continue;
                }
                sleep(Duration::from_secs(30)).await; //@TODO config enable
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
    msg_queue_count: Arc<AtomicIsize>,

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
        while let Some(map) = map_iter.next().await {
            match map {
                Ok(m) => match m.get::<_, StoredMessage>(DATA).await {
                    Ok(Some(smsg)) => {
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
                                    topic.push(TopicLevel::Normal(smsg.msg_id.to_string()));
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
    fn make_forwarded_key_v2(client_id: &str) -> Vec<u8> {
        [FORWARDED_PREFIX, client_id.as_bytes()].concat()
    }

    #[inline]
    async fn _batch_set_v2(&self, msgs: Vec<(From, Publish, Duration, MsgID)>) {
        if let Err(e) = self.storage_save_msg_id().await {
            log::warn!("save message id error, {:?}", e);
            return;
        }
        let mut count = 0;
        for (from, publish, expiry_interval, msg_id) in msgs {
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
            let msg_key = msg_id.to_be_bytes(); //self.make_msg_key(msg_id);
            let msg_map = self.storage_db.map(msg_key);
            if let Err(e) = msg_map.insert(DATA, &smsg).await {
                log::warn!("store to db error, {:?}, message: {:?}", e, smsg);
                continue;
            }
            //set ttl
            match msg_map.expire(expiry_interval.as_millis() as TimestampMillis).await {
                Err(e) => {
                    log::warn!("set map ttl to db error, {:?}, message: {:?}", e, smsg);
                }
                Ok(false) => {
                    log::warn!("set map ttl to db fail, message: {:?}", smsg);
                }
                Ok(true) => {}
            }

            log::debug!(
                "expiry_time: {:?} {:?}",
                msg_map.ttl().await.map(|t| t.map(|t| Duration::from_millis(t as u64))),
                rmqtt::broker::types::format_timestamp_millis(expiry_time_at)
            );

            //topic
            topic.push(TopicLevel::Normal(msg_id.to_string()));
            self.topic_tree.write().await.insert(&topic, msg_id);
            self.topic_list.write().await.insert((expiry_time_at, topic));

            count += 1;
        }
        self.messages_received_count_add(count);
        if let Err(e) = self.storage_messages_counter_add(count).await {
            log::warn!("messages_received_counter add error, {:?}", e);
        }
    }

    #[inline]
    async fn _get_v2(
        &self,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
    ) -> Result<Vec<(MsgID, From, Publish)>> {
        let inner = self;
        let mut topic = Topic::from_str(topic_filter).map_err(|e| anyhow!(format!("{:?}", e)))?;
        if !topic.levels().last().map(|l| matches!(l, TopicLevel::MultiWildcard)).unwrap_or_default() {
            topic.push(TopicLevel::SingleWildcard);
        }
        // let now = timestamp_millis();
        let (expireds, matcheds): (Vec<Option<Topic>>, Vec<Option<MsgID>>) = {
            inner
                .topic_tree
                .read()
                .await
                .matches(&topic)
                .into_iter()
                .map(|(_t, msg_id)| (None, Some(msg_id)))
                .unzip()
        };

        let expireds = expireds.into_iter().flatten().collect::<Vec<_>>();
        let matcheds = matcheds.into_iter().flatten().collect::<Vec<_>>();

        log::debug!("_get_v2 expireds: {:?}", expireds);
        log::debug!("_get_v2 matcheds msg_ids: {:?}", matcheds);
        let matcheds = futures::future::join_all(matcheds.into_iter().map(|msg_id| async move {
            let msg_key = msg_id.to_be_bytes(); //self.make_msg_key(msg_id);
            let mut msg_map = self.storage_db.map(msg_key);

            let is_forwarded =
                self._is_forwarded_v2(&mut msg_map, client_id, topic_filter, group).await.unwrap_or_default();
            if !is_forwarded {
                if let Err(e) = inner
                    ._forwarded_set_v2(
                        &msg_map,
                        client_id,
                        group.map(|g| (TopicFilter::from(topic_filter), g.clone())),
                    )
                    .await
                {
                    log::warn!("_get_v2 error, {:?}", e);
                }
            }

            if is_forwarded {
                None
            } else if let Ok(Some(msg)) = inner._get_message_v2(&msg_map).await {
                log::debug!("_get_v2 msg: {:?}", msg);
                if msg.is_expiry() {
                    None
                } else {
                    Some((msg_id, msg.from, msg.publish))
                }
            } else {
                None
            }
        }))
        .await
        .into_iter()
        .flatten()
        .collect();

        if !expireds.is_empty() {
            self.remove_expireds(expireds).await;
        }

        Ok(matcheds)
    }

    #[inline]
    async fn _is_forwarded_v2(
        &self,
        msg_map: &mut StorageMap,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
    ) -> Result<bool> {
        let key = Self::make_forwarded_key_v2(client_id);
        if msg_map.contains_key(key).await? {
            log::debug!("_is_forwarded_v2 contains_key client_id: {:?}", client_id);
            return Ok(true);
        }
        if let Some(group) = group {
            let mut iter =
                msg_map.prefix_iter::<_, Option<(TopicFilter, SharedGroup)>>(FORWARDED_PREFIX).await?;
            while let Some(item) = iter.next().await {
                log::debug!("_is_forwarded_v2 item: {:?}", item);
                match item {
                    Ok((_, Some((tf, g)))) => {
                        if &g == group && tf == topic_filter {
                            return Ok(true);
                        }
                    }
                    Ok((_, None)) => {}
                    Err(e) => {
                        log::warn!("traverse forwardeds error, {:?}", e);
                        return Err(MqttError::from(e));
                    }
                }
            }
        }
        Ok(false)
    }

    #[inline]
    async fn _forwarded_set_v2(
        &self,
        msg_map: &StorageMap,
        client_id: &str,
        opts: Option<(TopicFilter, SharedGroup)>,
    ) -> Result<()> {
        log::debug!(
            "_forwarded_set_v2 client_id: {:?}, msg_map name: {:?}",
            client_id,
            String::from_utf8_lossy(msg_map.name())
        );
        msg_map.insert(Self::make_forwarded_key_v2(client_id), &opts).await?;
        Ok(())
    }

    #[inline]
    async fn _get_message_v2(&self, msg_map: &StorageMap) -> Result<Option<StoredMessage>> {
        Ok(msg_map.get::<_, StoredMessage>(DATA).await?)
    }

    #[inline]
    async fn remove_expireds(&self, expireds: Vec<Topic>) {
        if expireds.len() > 10 {
            let topic_tree = self.topic_tree.clone();
            tokio::spawn(async move {
                for t in expireds {
                    topic_tree.write().await.remove(&t);
                }
            });
        } else {
            let mut topic_tree = self.topic_tree.write().await;
            for t in expireds {
                topic_tree.remove(&t);
            }
        }
    }
}

#[async_trait]
impl MessageManager for &'static StorageMessageManager {
    #[inline]
    async fn set(&self, from: From, p: Publish, expiry_interval: Duration) -> Result<MsgID> {
        let msg_id = self.storage_next_msg_id();
        log::debug!("StorageMessageManager set msg_id: {}, expiry_interval: {:?}", msg_id, expiry_interval);
        match self
            .msg_tx
            .clone()
            .send((from, p, expiry_interval, msg_id))
            .timeout(futures_time::time::Duration::from_millis(1500))
            .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                log::error!("StorageMessageManager set error, {:?}", e);
                return Err(MqttError::from(e.to_string()));
            }
            Err(e) => {
                log::warn!("StorageMessageManager set timeout, {:?}", e);
            }
        }
        self.msg_queue_count.fetch_add(1, Ordering::SeqCst);
        Ok(msg_id)
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
        let matcheds = async move { inner._get_v2(&client_id, &topic_filter, group.as_ref()).await }
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
                return Err(MqttError::from(e.to_string()));
            }
            Err(e) => {
                log::warn!("StorageMessageManager get timeout, {:?}", e);
                vec![]
            }
        };
        if now.elapsed().as_millis() > 500 {
            log::info!(
                "StorageMessageManager::get cost time: {:?}, waiting_count: {:?}",
                now.elapsed(),
                self.exec.waiting_count()
            );
        }
        log::debug!("StorageMessageManager get matcheds: {:?}", matcheds.len());
        Ok(matcheds)
    }

    #[inline]
    async fn set_forwardeds(
        &self,
        msg_id: MsgID,
        sub_client_ids: Vec<(ClientId, Option<(TopicFilter, SharedGroup)>)>,
    ) {
        log::debug!(
            "StorageMessageManager set_forwardeds msg_id: {:?}, sub_client_ids: {:?}",
            msg_id,
            sub_client_ids
        );
        let now = std::time::Instant::now();
        let inner = self.inner.clone();
        //self.make_msg_key(msg_id)
        let msg_map = self.storage_db.map(msg_id.to_be_bytes());
        let _ = async move {
            for (client_id, opts) in sub_client_ids {
                if let Err(e) = inner._forwarded_set_v2(&msg_map, &client_id, opts).await {
                    log::warn!(
                        "set_forwardeds error, msg_id: {}, client_id: {}, opts: {:?}",
                        msg_id,
                        client_id,
                        e
                    );
                }
            }
        }
        .spawn(&self.exec)
        .timeout(futures_time::time::Duration::from_millis(3000))
        .await;
        if now.elapsed().as_millis() > 500 {
            log::info!(
                "StorageMessageManager::set_forwardeds cost time: {:?}, waiting_count: {:?}",
                now.elapsed(),
                self.exec.waiting_count()
            );
        }
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
