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
    rust_box::std_ext::RwLock,
    rust_box::task_exec_queue::{Builder, SpawnExt, TaskExecQueue},
    tokio,
    tokio::task::spawn_blocking,
    tokio::time::sleep,
};

use rmqtt::{
    broker::retain::RetainTree, broker::MessageManager, timestamp_secs, ClientId, From, MqttError, PMsgID,
    PersistedMsg, Publish, Result, SharedGroup, Timestamp, Topic, TopicFilter,
};

use rmqtt::tokio::runtime::Handle;
use rmqtt_storage::{DefaultStorageDB, List, Map, StorageList, StorageMap};

use crate::config::PluginConfig;

type Msg = (From, Publish, Duration, PMsgID);

const MSG_STORE_RECEIVEDS: &str = "received_msgs";
const MSG_STORE_UNEXPIREDS: &str = "unexpired_msgs";
const MSG_STORE_FORWARDEDS: &str = "forwarded_msgs";

static INSTANCE: OnceCell<StorageMessageManager> = OnceCell::new();

#[inline]
pub(crate) async fn get_or_init(
    cfg: Arc<PluginConfig>,
    storage_db: DefaultStorageDB,
    should_merge_on_get: bool,
) -> Result<&'static StorageMessageManager> {
    if let Some(msg_mgr) = INSTANCE.get() {
        return Ok(msg_mgr);
    }
    let msg_mgr = StorageMessageManager::new(cfg, storage_db, should_merge_on_get).await?;
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
        cfg: Arc<PluginConfig>,
        storage_db: DefaultStorageDB,
        should_merge_on_get: bool,
    ) -> Result<StorageMessageManager> {
        let messages_unexpired_list = storage_db.list(MSG_STORE_UNEXPIREDS);
        let mut messages_received_map = storage_db.map(MSG_STORE_RECEIVEDS);
        let messages_forwarded_map = storage_db.map(MSG_STORE_FORWARDEDS);
        let id_generater = StorageMessageManagerInner::storage_new_msg_id_generater(&storage_db).await?;
        log::info!("current msg_id: {}", id_generater.load(Ordering::SeqCst));
        let (messages_received_count, messages_received_max) =
            StorageMessageManagerInner::storage_new_messages_counter(&storage_db, &mut messages_received_map)
                .await?;
        log::info!("messages_received_count: {}", messages_received_count.load(Ordering::SeqCst));
        log::info!("messages_received_max: {}", messages_received_max.load(Ordering::SeqCst));
        let (exec, msg_tx, msg_queue_count) = Self::serve(cfg)?;
        Ok(Self {
            inner: Arc::new(StorageMessageManagerInner {
                storage_db,
                messages_received_map,
                messages_unexpired_list,
                messages_forwarded_map,
                subs_tree: Default::default(),
                messages_received_count,
                messages_received_max,
                msg_tx,
                msg_queue_count,
                id_generater,
                should_merge_on_get,
            }),
            exec,
        })
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
                    tokio::spawn(msg_mgr._batch_set(msgs));
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
                    match spawn_blocking(move || {
                        Handle::current().block_on(async move {
                            match msg_mgr.remove_expired_messages(max_limit).await {
                                Err(e) => {
                                    log::warn!("remove expired messages error, {:?}", e);
                                    0
                                }
                                Ok(removeds) => removeds,
                            }
                        })
                    })
                    .await
                    {
                        Ok(removeds) => removeds,
                        Err(e) => {
                            log::warn!("remove expired messages error, {:?}", e);
                            0
                        }
                    }
                } else {
                    0
                };
                log::info!("remove_expired_messages, removeds: {} cost time: {:?}", removeds, now.elapsed());
                if removeds >= max_limit {
                    continue;
                }
                sleep(Duration::from_secs(20)).await; //@TODO config enable
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
    pub(crate) messages_received_map: StorageMap,
    pub(crate) messages_unexpired_list: StorageList,
    pub(crate) messages_forwarded_map: StorageMap, //key: PMsgID + ClientId, val: Option<(TopicFilter, SharedGroup)>

    subs_tree: RwLock<RetainTree<PMsgID>>,

    messages_received_count: AtomicIsize,
    messages_received_max: AtomicIsize,

    msg_tx: mpsc::Sender<Msg>,
    msg_queue_count: Arc<AtomicIsize>,

    id_generater: AtomicUsize,
    should_merge_on_get: bool,
}

impl StorageMessageManagerInner {
    #[inline]
    async fn _batch_set(&self, msgs: Vec<(From, Publish, Duration, PMsgID)>) {
        let mut received_key_vals = Vec::new();
        let mut unexpired_key_vals = Vec::new();
        if let Err(e) = self.storage_save_msg_id().await {
            log::warn!("save message id error, {:?}", e);
            return;
        }

        for (from, publish, expiry_interval, msg_id) in msgs {
            let mut topic = match Topic::from_str(&publish.topic) {
                Err(e) => {
                    log::warn!("Topic::from_str error, {:?}", e);
                    continue;
                }
                Ok(topic) => topic,
            };
            let expiry_time_at = timestamp_secs() + expiry_interval.as_secs() as i64;

            let pmsg = PersistedMsg { msg_id, from, publish, expiry_time_at };
            topic.push(TopicLevel::Normal(msg_id.to_string()));

            //topic
            self.subs_tree.write().insert(&topic, msg_id);

            //received messages
            let msg_id_bytes = msg_id.to_be_bytes();
            received_key_vals.push((msg_id_bytes.to_vec(), pmsg));

            //unexpired key vals
            unexpired_key_vals.push((msg_id, expiry_time_at));
        }

        self.messages_received_count_add(received_key_vals.len() as isize);
        if let Err(e) = self.storage_messages_counter_add(received_key_vals.len() as isize).await {
            log::warn!("messages_received_counter add error, {:?}", e);
        }
        if let Err(e) = self.messages_received_map.batch_insert::<PersistedMsg>(received_key_vals).await {
            log::warn!("messages_received batch_insert error, {:?}", e);
        }
        if let Err(e) = self.messages_unexpired_list.pushs(unexpired_key_vals).await {
            log::warn!("messages_unexpired pushs error, {:?}", e);
        }
    }

    #[inline]
    pub(crate) async fn remove_expired_messages(&self, max_limit: usize) -> Result<usize> {
        let now_inst = std::time::Instant::now();
        let now = timestamp_secs();
        let mut removed_topics = Vec::new();
        let mut messages_received_removed_count = 0;
        let mut forwarded_removed_keys = Vec::new();
        let mut received_removed_keys = Vec::new();

        let mut expired_count = 0;
        log::debug!("remove_expired_messages max_limit: {}", max_limit);
        while let Some((msg_id, _)) = self
            .messages_unexpired_list
            .pop_f::<_, (PMsgID, Timestamp)>(move |(_, expiry_time_at)| *expiry_time_at <= now)
            .await?
        {
            log::debug!(
                "remove_expired_messages is_expiry, msg_id: {}, expired_count: {}",
                msg_id,
                expired_count
            );

            forwarded_removed_keys.push(msg_id);
            received_removed_keys.push(msg_id.to_be_bytes().to_vec());
            if let Some(pmsg) = self._get_message(msg_id).await? {
                let mut topic =
                    Topic::from_str(&pmsg.publish.topic).map_err(|e| anyhow!(format!("{:?}", e)))?;
                topic.push(TopicLevel::Normal(msg_id.to_string()));
                removed_topics.push(topic);
                self.messages_received_count_dec();
                messages_received_removed_count += 1;
            }

            expired_count += 1;
            if expired_count > max_limit {
                break;
            }
        }

        log::debug!(
            "removed_topics: {}, messages_received_removed_count: {}, cost time: {:?}",
            removed_topics.len(),
            messages_received_removed_count,
            now_inst.elapsed()
        );

        for topic in removed_topics {
            self.subs_tree.write().remove(&topic);
        }

        if let Err(e) = self.messages_forwarded_batch_removes(forwarded_removed_keys).await {
            log::warn!("messages_forwarded_batch_removes error, {:?}", e);
            return Err(e);
        }

        if let Err(e) = self.messages_received_map.batch_remove(received_removed_keys).await {
            log::warn!("messages_received_map.batch_remove error, {:?}", e);
            return Err(MqttError::from(e));
        }

        log::debug!(
            "remove_expired_messages messages_received_removed_count: {}",
            messages_received_removed_count
        );
        if messages_received_removed_count == 0
            && self.messages_unexpired_list.is_empty().await.unwrap_or_default()
        {
            let mut messages_received_map = self.messages_received_map.clone();
            let mut iter = messages_received_map.key_iter().await?;
            let mut removes = Vec::new();
            while let Some(key) = iter.next().await {
                let key = key?;
                removes.push(key);
            }
            drop(iter);
            let removes_count = removes.len() as isize;
            if let Err(e) = messages_received_map.batch_remove(removes).await {
                log::warn!("messages_received_map.batch_remove error, {:?}", e);
                return Err(MqttError::from(e));
            }

            self.messages_received_count_sub(removes_count);

            let mut messages_forwarded_map = self.messages_forwarded_map.clone();
            let mut iter = messages_forwarded_map.key_iter().await?;
            let mut removes = Vec::new();
            while let Some(key) = iter.next().await {
                let key = key?;
                removes.push(key);
            }
            drop(iter);
            if let Err(e) = messages_forwarded_map.batch_remove(removes).await {
                log::warn!("messages_forwarded_map.batch_remove error, {:?}", e);
                return Err(MqttError::from(e));
            }
        }

        Ok(messages_received_removed_count)
    }

    #[inline]
    async fn messages_forwarded_batch_removes(&self, msg_ids: Vec<PMsgID>) -> Result<()> {
        let mut batch = Vec::new();
        let mut messages_forwarded_map = self.messages_forwarded_map.clone();
        for msg_id in msg_ids {
            let mut iter = messages_forwarded_map
                .prefix_iter::<_, Option<(TopicFilter, SharedGroup)>>(msg_id.to_be_bytes())
                .await?;
            while let Some(item) = iter.next().await {
                match item {
                    Ok((key, _)) => {
                        log::debug!(
                            "messages_forwarded_batch_removes, key: {:?}",
                            String::from_utf8_lossy(key.as_slice())
                        );
                        batch.push(key);
                    }
                    Err(e) => {
                        log::error!("traverse forwardeds error, {:?}", e);
                        continue;
                    }
                }
            }
        }
        log::debug!("messages_forwarded_batch_removes batch len: {}", batch.len());
        messages_forwarded_map.batch_remove(batch).await?;
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
    async fn storage_new_messages_counter(
        storage_db: &DefaultStorageDB,
        messages_received_map: &mut StorageMap,
    ) -> Result<(AtomicIsize, AtomicIsize)> {
        let now = std::time::Instant::now();
        let mut count = 0;
        let mut iter = messages_received_map.key_iter().await?;
        while let Some(_) = iter.next().await {
            count += 1;
        }
        let max = storage_db.counter_get("messages_received_max").await?.unwrap_or_default();
        log::info!(
            "messages_received_count: {}, messages_received_max: {}, cost time: {:?}",
            count,
            max,
            now.elapsed()
        );
        Ok((AtomicIsize::new(count), AtomicIsize::new(max)))
    }

    #[inline]
    fn messages_received_count_add(&self, len: isize) {
        self.messages_received_count.fetch_add(len, Ordering::SeqCst);
        self.messages_received_max.fetch_add(len, Ordering::SeqCst);
    }

    #[inline]
    fn messages_received_count_dec(&self) {
        self.messages_received_count.fetch_sub(1, Ordering::SeqCst);
    }

    #[inline]
    fn messages_received_count_sub(&self, c: isize) {
        self.messages_received_count.fetch_sub(c, Ordering::SeqCst);
    }

    #[inline]
    fn make_forwarded_key(msg_id: PMsgID, client_id: &str) -> Vec<u8> {
        [msg_id.to_be_bytes().as_slice(), client_id.as_bytes()].concat()
    }

    #[inline]
    async fn _forwarded_set(
        &self,
        msg_id: PMsgID,
        client_id: &str,
        opts: Option<(TopicFilter, SharedGroup)>,
    ) -> Result<()> {
        self.messages_forwarded_map.insert(Self::make_forwarded_key(msg_id, client_id), &opts).await?;
        Ok(())
    }

    #[inline]
    async fn _is_forwarded(
        &self,
        msg_id: PMsgID,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
    ) -> Result<bool> {
        let key = Self::make_forwarded_key(msg_id, client_id);
        if self.messages_forwarded_map.contains_key(key).await? {
            return Ok(true);
        }
        let mut messages_forwarded_map = self.messages_forwarded_map.clone();
        if let Some(group) = group {
            let mut iter = messages_forwarded_map
                .prefix_iter::<_, Option<(TopicFilter, SharedGroup)>>(msg_id.to_be_bytes())
                .await?;
            while let Some(item) = iter.next().await {
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
    async fn _get_message(&self, msg_id: PMsgID) -> Result<Option<PersistedMsg>> {
        Ok(self.messages_received_map.get::<_, PersistedMsg>(msg_id.to_be_bytes().as_slice()).await?)
    }

    #[inline]
    async fn _get(
        &self,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
    ) -> Result<Vec<(PMsgID, From, Publish)>> {
        let inner = self;
        let mut topic = Topic::from_str(topic_filter).map_err(|e| anyhow!(format!("{:?}", e)))?;
        if !topic.levels().last().map(|l| matches!(l, TopicLevel::MultiWildcard)).unwrap_or_default() {
            topic.push(TopicLevel::SingleWildcard);
        }

        let matcheds =
            { inner.subs_tree.read().matches(&topic).iter().map(|(_, msg_id)| *msg_id).collect::<Vec<_>>() };

        let matcheds = futures::future::join_all(matcheds.into_iter().map(|msg_id| async move {
            let is_forwarded =
                self._is_forwarded(msg_id, client_id, topic_filter, group).await.unwrap_or_default();
            if !is_forwarded {
                if let Err(e) = inner
                    ._forwarded_set(
                        msg_id,
                        client_id,
                        group.map(|g| (TopicFilter::from(topic_filter), g.clone())),
                    )
                    .await
                {
                    log::warn!("_get error, {:?}", e);
                }
            }

            if is_forwarded {
                None
            } else if let Ok(Some(pmsg)) = inner._get_message(msg_id).await {
                if pmsg.is_expiry() {
                    None
                } else {
                    Some((msg_id, pmsg.from, pmsg.publish))
                }
            } else {
                None
            }
        }))
        .await
        .into_iter()
        .flatten()
        .collect();
        Ok(matcheds)
    }
}

#[async_trait]
impl MessageManager for &'static StorageMessageManager {
    #[inline]
    async fn set(&self, from: From, p: Publish, expiry_interval: Duration) -> Result<PMsgID> {
        let msg_id = self.storage_next_msg_id();
        log::debug!("StorageMessageManager set msg_id: {}", msg_id);
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
    ) -> Result<Vec<(PMsgID, From, Publish)>> {
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
                return Err(MqttError::from(e.to_string()));
            }
            Err(e) => {
                log::warn!("StorageMessageManager get timeout, {:?}", e);
                vec![]
            }
        };
        if now.elapsed().as_millis() > 300 {
            log::info!(
                "StorageMessageManager::get cost time: {:?}, waiting_count: {:?}",
                now.elapsed(),
                self.exec.waiting_count()
            );
        }
        log::debug!("StorageMessageManager get matcheds: {:?}", matcheds);
        Ok(matcheds)
    }

    #[inline]
    async fn set_forwardeds(
        &self,
        msg_id: PMsgID,
        sub_client_ids: Vec<(ClientId, Option<(TopicFilter, SharedGroup)>)>,
    ) {
        log::debug!(
            "StorageMessageManager set_forwardeds msg_id: {:?}, sub_client_ids: {:?}",
            msg_id,
            sub_client_ids
        );
        let now = std::time::Instant::now();
        let inner = self.inner.clone();
        let _ = async move {
            for (client_id, opts) in sub_client_ids {
                if let Err(e) = inner._forwarded_set(msg_id, &client_id, opts).await {
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
        if now.elapsed().as_millis() > 200 {
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
        self.messages_received_count.load(Ordering::SeqCst)
    }

    #[inline]
    async fn max(&self) -> isize {
        self.messages_received_max.load(Ordering::SeqCst)
    }
}
