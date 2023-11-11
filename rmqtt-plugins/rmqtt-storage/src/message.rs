use std::convert::From as _;
use std::mem::size_of;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rmqtt::{
    anyhow,
    anyhow::anyhow,
    async_trait::async_trait,
    futures::channel::mpsc,
    futures::channel::oneshot,
    log,
    ntex_mqtt::TopicLevel,
    once_cell::sync::OnceCell,
    rust_box::std_ext::RwLock,
    rust_box::task_exec_queue::{Builder, SpawnExt, TaskExecQueue},
    tokio, MqttError,
};

use rmqtt::futures::{SinkExt, StreamExt};
use rmqtt::tokio::time::sleep;
use rmqtt::{
    broker::retain::RetainTree, broker::MessageManager, timestamp_secs, ClientId, From, PMsgID, PersistedMsg,
    Publish, Result, SharedGroup, Timestamp, Topic, TopicFilter,
};

use crate::store::storage::Storage;
use crate::store::{StorageDB, StorageKV};

type Msg = (From, Publish, Duration, oneshot::Sender<Result<PMsgID>>);

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
            //let queue_max_two_thirds = (queue_max / 3 * 2) as isize;
            let (exec, task_runner) = Builder::default().workers(200).queue_max(queue_max).build();

            tokio::spawn(async move {
                task_runner.await;
            });

            let exec1 = exec.clone();
            tokio::task::spawn_blocking(move || {
                tokio::runtime::Handle::current().block_on(async move {
                    let max_limit = 1000;
                    sleep(Duration::from_secs(30)).await;
                    loop {
                        if exec1.active_count() > 1 {
                            sleep(Duration::from_secs(5)).await;
                            continue;
                        }

                        let now = std::time::Instant::now();
                        let removeds = if let Some(msg_mgr) = INSTANCE.get() {
                            match msg_mgr.remove_expired_messages(max_limit, &exec1) {
                                Err(e) => {
                                    log::warn!("remove expired messages error, {:?}", e);
                                    0
                                }
                                Ok(removed) => removed,
                            }
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
                        sleep(Duration::from_secs(60)).await; //@TODO config enable
                    }
                })
            });

            let (msg_tx, mut msg_rx) = mpsc::channel::<Msg>(300_000);
            tokio::task::spawn_blocking(move || {
                tokio::runtime::Handle::current().block_on(async move {
                    loop {
                        if INSTANCE.get().is_some() {
                            break;
                        }
                        sleep(Duration::from_millis(10)).await;
                    }
                    if let Some(msg_mgr) = INSTANCE.get() {
                        let mut merger_msgs = Vec::new();
                        while let Some((from, publish, expiry_interval, res_tx)) = msg_rx.next().await {
                            let msg_id = match msg_mgr.next_id() {
                                Err(e) => {
                                    let _ = res_tx.send(Err(e));
                                    continue;
                                }
                                Ok(msg_id) => msg_id,
                            };
                            merger_msgs.push((from, publish, expiry_interval, msg_id, res_tx));
                            while merger_msgs.len() < 1000 {
                                match tokio::time::timeout(Duration::from_millis(0), msg_rx.next()).await {
                                    Ok(Some((from, publish, expiry_interval, res_tx))) => {
                                        let msg_id = match msg_mgr.next_id() {
                                            Err(e) => {
                                                let _ = res_tx.send(Err(e));
                                                continue;
                                            }
                                            Ok(msg_id) => msg_id,
                                        };
                                        merger_msgs.push((from, publish, expiry_interval, msg_id, res_tx));
                                    }
                                    _ => break,
                                }
                            }
                            log::debug!("merger_msgs.len: {}", merger_msgs.len());
                            //merge and send
                            let msgs = std::mem::take(&mut merger_msgs);
                            msg_mgr._batch_set(msgs);
                        }
                    }
                })
            });

            let messages_received_count = AtomicIsize::new(messages_received_kv.len() as isize);
            let messages_received_max =
                storage_db.counter_get("messages_received").unwrap_or_default() as isize;
            let messages_received_max = AtomicIsize::new(messages_received_max);
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
}

impl StorageMessageManagerInner {
    #[inline]
    fn next_id(&self) -> Result<usize> {
        Ok(self.storage_db.generate_id().map(|id| id as usize)?)
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

            //self.messages_unexpired_kv.remove(meta.key.as_ref())?;
            unexpired_removed_keys.push(meta.key.to_vec());
            //self.messages_forwarded_kv.remove_with_prefix(msg_id_bytes)?;
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
    fn _batch_set(&self, msgs: Vec<(From, Publish, Duration, PMsgID, oneshot::Sender<Result<PMsgID>>)>) {
        let mut received_key_vals = Vec::new();
        let mut unexpired_key_vals = Vec::new();
        let mut msg_id_txs = Vec::new();
        for (from, publish, expiry_interval, msg_id, res_tx) in msgs {
            let mut topic = match Topic::from_str(&publish.topic) {
                Err(e) => {
                    //.map_err(|e| anyhow!(format!("{:?}", e)))
                    let _ = res_tx.send(Err(MqttError::from(e)));
                    continue;
                }
                Ok(topic) => topic,
            };
            let expiry_time = timestamp_secs() + expiry_interval.as_secs() as i64;

            let pmsg = PersistedMsg { msg_id, from, publish, expiry_time };
            topic.push(TopicLevel::Normal(msg_id.to_string()));

            let msg_id_bytes = msg_id.to_be_bytes();

            //self.messages_received_kv.insert(msg_id_bytes.as_slice(), &pmsg)?;
            received_key_vals.push((msg_id_bytes.to_vec(), pmsg));
            self.messages_received_count_inc();
            self.subs_tree.write().insert(&topic, msg_id);

            let mut expiry_time = expiry_time.to_be_bytes().to_vec();
            expiry_time.extend_from_slice(msg_id_bytes.as_slice());
            //self.messages_unexpired_kv.insert::<_, ()>(expiry_time.as_slice(), &())?;
            unexpired_key_vals.push((expiry_time, ()));

            if let Err(e) = self.storage_db.counter_inc("messages_received") {
                let _ = res_tx.send(Err(MqttError::from(e)));
                continue;
            }
            msg_id_txs.push((msg_id, res_tx));
        }

        if let Err(e) = self.messages_received_kv.batch_insert::<_, PersistedMsg>(received_key_vals) {
            for (_, tx) in msg_id_txs {
                let _ = tx.send(Err(MqttError::from(e.to_string())));
            }
            return;
        }
        if let Err(e) = self.messages_unexpired_kv.batch_insert::<_, ()>(unexpired_key_vals) {
            for (_, tx) in msg_id_txs {
                let _ = tx.send(Err(MqttError::from(e.to_string())));
            }
            return;
        }
        for (msg_id, tx) in msg_id_txs {
            let _ = tx.send(Ok(msg_id));
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
    ) -> Result<Vec<(PMsgID, rmqtt::From, Publish)>> {
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
        let now = std::time::Instant::now();
        let (tx, rx) = oneshot::channel();
        self.msg_tx.clone().send((from, p, expiry_interval, tx)).await.map_err(|e| anyhow!(e))?;
        let msg_id = rx.await.map_err(|e| anyhow!(e))??;
        let now_elapsed = now.elapsed();
        if now_elapsed.as_millis() > 250 {
            log::info!("StorageMessageManager::set cost time: {:?}", now_elapsed);
        }
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
        if now.elapsed().as_millis() > 100 {
            log::info!("StorageMessageManager::get cost time: {:?}", now.elapsed());
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
            })
            .await
        }
        .spawn(&self.exec)
        .await;
        if now.elapsed().as_millis() > 100 {
            log::info!("StorageMessageManager::set_forwardeds cost time: {:?}", now.elapsed());
        }
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
