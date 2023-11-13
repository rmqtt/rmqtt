use std::convert::From as _;
use std::mem::size_of;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rmqtt::{
    anyhow,
    anyhow::anyhow,
    async_trait::async_trait,
    futures::channel::mpsc,
    log,
    ntex_mqtt::TopicLevel,
    once_cell::sync::OnceCell,
    rust_box::std_ext::RwLock,
    rust_box::task_exec_queue::{Builder, SpawnExt, TaskExecQueue},
    tokio,
};

use rmqtt::futures::{SinkExt, StreamExt};
use rmqtt::tokio::time::sleep;
use rmqtt::{
    broker::retain::RetainTree, broker::MessageManager, timestamp_secs, ClientId, From, PMsgID, PersistedMsg,
    Publish, Result, SharedGroup, Timestamp, Topic, TopicFilter,
};

use crate::store::storage::Storage;
use crate::store::{StorageDB, StorageKV};

type Msg = (From, Publish, Duration, PMsgID);

pub struct StorageMessageManager {
    inner: Arc<StorageMessageManagerInner>,
    exec: TaskExecQueue,
}

impl StorageMessageManager {
    #[inline]
    pub(crate) fn get_or_init(
        storage_db: StorageDB,
        messages_received_kv: StorageKV,
        messages_unexpired_kv: StorageKV,
        messages_forwarded_kv: StorageKV,
    ) -> &'static StorageMessageManager {
        static INSTANCE: OnceCell<StorageMessageManager> = OnceCell::new();
        INSTANCE.get_or_init(|| {
            let queue_max = 300_000;
            let (exec, task_runner) = Builder::default().workers(1000).queue_max(queue_max).build();

            tokio::spawn(async move {
                task_runner.await;
            });

            let exec1 = exec.clone();
            tokio::spawn(async move {
                let max_limit = 1000;
                sleep(Duration::from_secs(30)).await;
                loop {
                    if exec1.active_count() > 1 {
                        sleep(Duration::from_secs(3)).await;
                        continue;
                    }
                    let exec1 = exec1.clone();
                    let now = std::time::Instant::now();
                    let removeds = if let Some(msg_mgr) = INSTANCE.get() {
                        tokio::task::spawn_blocking(move || {
                            match msg_mgr.remove_expired_messages(max_limit, &exec1) {
                                Err(e) => {
                                    log::warn!("remove expired messages error, {:?}", e);
                                    0
                                }
                                Ok(removed) => removed,
                            }
                        })
                        .await
                        .unwrap_or_default()
                    } else {
                        0
                    };
                    log::debug!(
                        "remove_expired_messages, removeds: {} cost time: {:?}",
                        removeds,
                        now.elapsed()
                    );
                    if removeds >= max_limit {
                        continue;
                    }
                    sleep(Duration::from_secs(30)).await; //@TODO config enable
                }
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
                        tokio::task::spawn_blocking(move || {
                            msg_mgr._batch_set(msgs);
                        });
                    }
                    log::error!("Recv failed because receiver is gone");
                }
            });

            let messages_received_count = AtomicIsize::new(messages_received_kv.len() as isize);
            log::info!("messages_received_count: {}", messages_received_count.load(Ordering::SeqCst));
            let messages_received_max =
                storage_db.counter_get("messages_received").unwrap_or_default() as isize;

            let messages_received_max = AtomicIsize::new(messages_received_max);
            let id_generater = AtomicUsize::new(storage_db.counter_get("id_generater").unwrap_or(1));
            log::info!("current msg_id: {}", id_generater.load(Ordering::SeqCst));
            Self {
                inner: Arc::new(StorageMessageManagerInner {
                    storage_db,
                    messages_received_kv,
                    messages_unexpired_kv,
                    messages_forwarded_kv,
                    subs_tree: Default::default(),
                    messages_received_count,
                    messages_received_max,
                    msg_tx,
                    msg_queue_count,
                    id_generater,
                }),
                exec,
            }
        })
    }
}

impl Deref for StorageMessageManager {
    type Target = Arc<StorageMessageManagerInner>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

pub struct StorageMessageManagerInner {
    storage_db: StorageDB,
    messages_received_kv: StorageKV,
    messages_unexpired_kv: StorageKV,
    messages_forwarded_kv: StorageKV, //key: PMsgID + ClientId, val: Option<(TopicFilter, SharedGroup)>

    subs_tree: RwLock<RetainTree<PMsgID>>,

    messages_received_count: AtomicIsize,
    messages_received_max: AtomicIsize,

    msg_tx: mpsc::Sender<Msg>,
    msg_queue_count: Arc<AtomicIsize>,

    id_generater: AtomicUsize,
}

impl StorageMessageManagerInner {
    #[inline]
    fn save_msg_id(&self) -> Result<()> {
        self.storage_db.counter_set("id_generater", self.id_generater.load(Ordering::SeqCst))?;
        Ok(())
    }

    #[inline]
    fn next_msg_id(&self) -> usize {
        self.id_generater.fetch_add(1, Ordering::SeqCst)
    }

    #[inline]
    fn messages_received_count_inc(&self) {
        self.messages_received_count.fetch_add(1, Ordering::SeqCst);
        self.messages_received_max.fetch_add(1, Ordering::SeqCst);
    }

    #[inline]
    fn messages_received_count_dec(&self) {
        self.messages_received_count.fetch_sub(1, Ordering::SeqCst);
    }

    #[inline]
    pub(crate) fn remove_expired_messages(&self, max_limit: usize, exec: &TaskExecQueue) -> Result<usize> {
        let now_inst = std::time::Instant::now();
        let now = timestamp_secs();
        let len = size_of::<Timestamp>();
        let mut removed_topics = Vec::new();
        let mut messages_received_removed_count = 0;

        let mut unexpired_removed_keys = Vec::new();
        let mut forwarded_removed_keys = Vec::new();
        let mut received_removed_keys = Vec::new();
        for (expired_count, item) in self.messages_unexpired_kv.iter_meta().enumerate() {
            if exec.waiting_count() > 1 {
                log::debug!("remove topics exec.waiting_count(): {:?}", exec.waiting_count());
                break;
            }

            let meta = item?;
            let (expiry_time_bytes, msg_id_bytes) = meta.key.as_ref().split_at(len);
            let expiry_time =
                Timestamp::from_be_bytes(expiry_time_bytes.try_into().map_err(anyhow::Error::new)?);

            let is_expiry = expiry_time < now;
            if !is_expiry || expired_count > max_limit {
                break;
            }

            unexpired_removed_keys.push(meta.key.to_vec());
            forwarded_removed_keys.push(msg_id_bytes.to_vec());

            let msg_id = PMsgID::from_be_bytes(msg_id_bytes.try_into().map_err(anyhow::Error::new)?);

            received_removed_keys.push(msg_id_bytes.to_vec());
            if let Some(pmsg) = self.messages_received_kv.get::<_, PersistedMsg>(msg_id_bytes)? {
                let mut topic =
                    Topic::from_str(&pmsg.publish.topic).map_err(|e| anyhow!(format!("{:?}", e)))?;
                topic.push(TopicLevel::Normal(msg_id.to_string()));
                removed_topics.push(topic);
                self.messages_received_count_dec();
                messages_received_removed_count += 1;
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

        self.messages_unexpired_kv.batch_remove(unexpired_removed_keys)?;
        self.messages_forwarded_kv.batch_remove_with_prefix(forwarded_removed_keys)?;
        self.messages_received_kv.batch_remove(received_removed_keys)?;

        if messages_received_removed_count == 0 && self.messages_unexpired_kv.is_empty() {
            for item in self.messages_received_kv.iter_meta() {
                let item = item?;
                self.messages_received_kv.remove(item.key.as_ref())?;
                self.messages_received_count_dec();
            }
            for item in self.messages_forwarded_kv.iter_meta() {
                let item = item?;
                self.messages_forwarded_kv.remove(item.key.as_ref())?;
            }
        }

        log::debug!("remove topics cost time: {:?}", now_inst.elapsed());
        Ok(messages_received_removed_count)
    }

    #[inline]
    fn _forwarded_key(&self, msg_id: PMsgID, client_id: &str) -> Vec<u8> {
        let mut key = msg_id.to_be_bytes().to_vec();
        key.extend_from_slice(client_id.as_bytes());
        key
    }

    #[inline]
    fn _forwarded_set(
        &self,
        msg_id: PMsgID,
        client_id: &str,
        opts: Option<(TopicFilter, SharedGroup)>,
    ) -> Result<()> {
        self.messages_forwarded_kv.insert(self._forwarded_key(msg_id, client_id), &opts)?;
        Ok(())
    }

    #[inline]
    fn _is_forwarded(
        &self,
        msg_id: PMsgID,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
    ) -> Result<bool> {
        let key = self._forwarded_key(msg_id, client_id);
        if self.messages_forwarded_kv.contains_key(key)? {
            return Ok(true);
        }
        if let Some(group) = group {
            for item in self
                .messages_forwarded_kv
                .prefix_iter::<_, Option<(TopicFilter, SharedGroup)>>(msg_id.to_be_bytes())
            {
                if let (_, Some((tf, g))) = item? {
                    if &g == group && tf == topic_filter {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }

    #[inline]
    fn _batch_set(&self, msgs: Vec<(From, Publish, Duration, PMsgID)>) {
        let mut received_key_vals = Vec::new();
        let mut unexpired_key_vals = Vec::new();
        let _ = self.save_msg_id();
        for (from, publish, expiry_interval, msg_id) in msgs {
            let mut topic = match Topic::from_str(&publish.topic) {
                Err(e) => {
                    log::warn!("Topic::from_str error, {:?}", e);
                    continue;
                }
                Ok(topic) => topic,
            };
            let expiry_time = timestamp_secs() + expiry_interval.as_secs() as i64;

            let pmsg = PersistedMsg { msg_id, from, publish, expiry_time };
            topic.push(TopicLevel::Normal(msg_id.to_string()));

            let msg_id_bytes = msg_id.to_be_bytes();

            received_key_vals.push((msg_id_bytes.to_vec(), pmsg));
            self.messages_received_count_inc();
            self.subs_tree.write().insert(&topic, msg_id);

            let mut expiry_time = expiry_time.to_be_bytes().to_vec();
            expiry_time.extend_from_slice(msg_id_bytes.as_slice());
            unexpired_key_vals.push((expiry_time, ()));

            if let Err(e) = self.storage_db.counter_inc("messages_received") {
                log::warn!("messages_received counter_inc error, {:?}", e);
                continue;
            }
        }

        if let Err(e) = self.messages_received_kv.batch_insert::<_, PersistedMsg>(received_key_vals) {
            log::warn!("messages_received batch_insert error, {:?}", e);
        }
        if let Err(e) = self.messages_unexpired_kv.batch_insert::<_, ()>(unexpired_key_vals) {
            log::warn!("messages_unexpired batch_insert error, {:?}", e);
        }
    }

    #[inline]
    fn _set(&self, from: From, publish: Publish, expiry_interval: Duration, msg_id: PMsgID) -> Result<()> {
        let mut topic = Topic::from_str(&publish.topic).map_err(|e| anyhow!(format!("{:?}", e)))?;
        let expiry_time = timestamp_secs() + expiry_interval.as_secs() as i64;

        let pmsg = PersistedMsg { msg_id, from, publish, expiry_time };
        topic.push(TopicLevel::Normal(msg_id.to_string()));

        let msg_id_bytes = msg_id.to_be_bytes();

        self.messages_received_kv.insert(msg_id_bytes.as_slice(), &pmsg)?;
        self.messages_received_count_inc();
        self.subs_tree.write().insert(&topic, msg_id);

        let mut expiry_time = expiry_time.to_be_bytes().to_vec();
        expiry_time.extend_from_slice(msg_id_bytes.as_slice());
        self.messages_unexpired_kv.insert::<_, ()>(expiry_time.as_slice(), &())?;

        self.storage_db.counter_inc("messages_received")?;
        Ok(())
    }

    #[inline]
    fn _get(
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

        let matcheds = matcheds
            .into_iter()
            .filter_map(|msg_id| {
                let is_forwarded =
                    self._is_forwarded(msg_id, client_id, topic_filter, group).unwrap_or_default();
                if !is_forwarded {
                    if let Err(e) = inner._forwarded_set(
                        msg_id,
                        client_id,
                        group.map(|g| (TopicFilter::from(topic_filter), g.clone())),
                    ) {
                        log::warn!("_forwarded_set error, {:?}", e);
                    }
                }

                if is_forwarded {
                    None
                } else if let Ok(Some(pmsg)) =
                    inner.messages_received_kv.get::<_, PersistedMsg>(msg_id.to_be_bytes().as_slice())
                {
                    if pmsg.is_expiry() {
                        None
                    } else {
                        Some((msg_id, pmsg.from, pmsg.publish))
                    }
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        Ok(matcheds)
    }
}

#[async_trait]
impl MessageManager for &'static StorageMessageManager {
    #[inline]
    async fn set(&self, from: From, p: Publish, expiry_interval: Duration) -> Result<PMsgID> {
        let msg_id = self.next_msg_id();
        self.msg_tx.clone().send((from, p, expiry_interval, msg_id)).await.map_err(|e| anyhow!(e))?;
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
        let matcheds = async move {
            tokio::task::spawn_blocking(move || inner._get(&client_id, &topic_filter, group.as_ref())).await
        }
        .spawn(&self.exec)
        .result()
        .await
        .map_err(|e| anyhow!(e.to_string()))???;
        if now.elapsed().as_millis() > 300 {
            log::info!(
                "StorageMessageManager::get cost time: {:?}, waiting_count: {:?}",
                now.elapsed(),
                self.exec.waiting_count()
            );
        }
        Ok(matcheds)
    }

    #[inline]
    async fn set_forwardeds(
        &self,
        msg_id: PMsgID,
        sub_client_ids: Vec<(ClientId, Option<(TopicFilter, SharedGroup)>)>,
    ) {
        let now = std::time::Instant::now();
        let inner = self.inner.clone();
        let _ = async move {
            tokio::task::spawn_blocking(move || {
                for (client_id, opt) in sub_client_ids {
                    if let Err(e) = inner._forwarded_set(msg_id, &client_id, opt) {
                        log::warn!("set_forwardeds error, {:?}", e);
                    }
                }
            });
        }
        .spawn(&self.exec)
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
        self.storage_db.should_merge_on_get()
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
