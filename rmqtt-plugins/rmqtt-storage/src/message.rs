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
    log,
    ntex_mqtt::TopicLevel,
    once_cell::sync::OnceCell,
    rust_box::task_exec_queue::{Builder, SpawnExt, TaskExecQueue},
    tokio,
    tokio::sync::RwLock,
};

use rmqtt::tokio::time::sleep;
use rmqtt::{
    broker::retain::RetainTree, broker::MessageManager, timestamp_secs, ClientId, From, PMsgID, PersistedMsg,
    Publish, Result, SharedGroup, Timestamp, Topic, TopicFilter,
};

use crate::store::storage::Storage;
use crate::store::{StorageDB, StorageKV};

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
            let (exec, task_runner) = Builder::default().workers(1000).queue_max(300_000).build();

            tokio::spawn(async move {
                task_runner.await;
            });

            tokio::spawn(async move {
                let max_limit = 50000;
                sleep(Duration::from_secs(30)).await;

                loop {
                    let now = std::time::Instant::now();
                    let removeds = tokio::task::spawn_blocking(move || {
                        tokio::runtime::Handle::current().block_on(async move {
                            if let Some(msg_mgr) = INSTANCE.get() {
                                match msg_mgr.remove_expired_messages(max_limit).await {
                                    Err(e) => {
                                        log::warn!("remove expired messages error, {:?}", e);
                                        0
                                    }
                                    Ok(removed) => removed,
                                }
                            } else {
                                0
                            }
                        })
                    })
                    .await
                    .unwrap_or_default();
                    log::info!(
                        "remove_expired_messages, removeds: {} cost time: {:?}",
                        removeds,
                        now.elapsed()
                    );
                    if removeds >= max_limit {
                        continue;
                    }
                    sleep(Duration::from_secs(10)).await; //@TODO config enable
                }
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
    pub(crate) async fn remove_expired_messages(&self, max_limit: usize) -> Result<usize> {
        let now_inst = std::time::Instant::now();
        let now = timestamp_secs();
        let len = size_of::<Timestamp>();
        let mut removed_topics = Vec::new();
        let mut messages_received_removed_count = 0;

        for (expired_count, item) in self.messages_unexpired_kv.iter_meta().enumerate() {
            let meta = item?;
            let (expiry_time_bytes, msg_id_bytes) = meta.key.as_ref().split_at(len);
            let expiry_time =
                Timestamp::from_be_bytes(expiry_time_bytes.try_into().map_err(anyhow::Error::new)?);

            let is_expiry = expiry_time < now;
            if !is_expiry || expired_count > max_limit {
                break;
            }

            self.messages_unexpired_kv.remove(meta.key.as_ref())?;
            self.messages_forwarded_kv.remove_with_prefix(msg_id_bytes)?;

            let msg_id = PMsgID::from_be_bytes(msg_id_bytes.try_into().map_err(anyhow::Error::new)?);

            if let Some(pmsg) = self.messages_received_kv.remove_and_fetch::<_, PersistedMsg>(msg_id_bytes)? {
                let mut topic =
                    Topic::from_str(&pmsg.publish.topic).map_err(|e| anyhow!(format!("{:?}", e)))?;
                topic.push(TopicLevel::Normal(msg_id.to_string()));
                removed_topics.push(topic);
                self.messages_received_count_dec();
                messages_received_removed_count += 1;
            }
        }

        log::info!(
            "removed_topics: {}, messages_received_removed_count: {}, cost time: {:?}",
            removed_topics.len(),
            messages_received_removed_count,
            now_inst.elapsed()
        );
        for topic in removed_topics {
            self.subs_tree.write().await.remove(&topic);
        }
        log::info!("remove topics cost time: {:?}", now_inst.elapsed());
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
    async fn _set(
        &self,
        from: From,
        publish: Publish,
        expiry_interval: Duration,
        msg_id: PMsgID,
    ) -> Result<()> {
        let mut topic = Topic::from_str(&publish.topic).map_err(|e| anyhow!(format!("{:?}", e)))?;
        let expiry_time = timestamp_secs() + expiry_interval.as_secs() as i64;

        let pmsg = PersistedMsg { msg_id, from, publish, expiry_time };
        topic.push(TopicLevel::Normal(msg_id.to_string()));

        let msg_id_bytes = msg_id.to_be_bytes();

        self.messages_received_kv.insert(msg_id_bytes.as_slice(), &pmsg)?;
        self.messages_received_count_inc();
        self.subs_tree.write().await.insert(&topic, msg_id);

        let mut expiry_time = expiry_time.to_be_bytes().to_vec();
        expiry_time.extend_from_slice(msg_id_bytes.as_slice());
        self.messages_unexpired_kv.insert::<_, ()>(expiry_time.as_slice(), &())?;

        self.storage_db.counter_inc("messages_received")?;
        Ok(())
    }
}

#[async_trait]
impl MessageManager for &'static StorageMessageManager {
    #[inline]
    async fn set(&self, from: From, p: Publish, expiry_interval: Duration) -> Result<PMsgID> {
        let inner = self.inner.clone();
        let msg_id = inner.next_id()?;
        async move {
            if let Err(e) = inner._set(from, p, expiry_interval, msg_id).await {
                log::warn!("Persistence of the Publish message failed! {:?}", e);
            }
        }
        .spawn(&self.exec)
        .await
        .map_err(|e| anyhow!(e.to_string()))?;
        Ok(msg_id)
    }

    #[inline]
    async fn get(
        &self,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
    ) -> Result<Vec<(PMsgID, rmqtt::From, Publish)>> {
        let inner = &self.inner;
        let mut topic = Topic::from_str(topic_filter).map_err(|e| anyhow!(format!("{:?}", e)))?;
        if !topic.levels().last().map(|l| matches!(l, TopicLevel::MultiWildcard)).unwrap_or_default() {
            topic.push(TopicLevel::SingleWildcard);
        }

        let matcheds = {
            inner.subs_tree.read().await.matches(&topic).iter().map(|(_, msg_id)| *msg_id).collect::<Vec<_>>()
        };

        let matcheds = matcheds
            .into_iter()
            .filter_map(|msg_id| {
                let is_forwarded =
                    self.inner._is_forwarded(msg_id, client_id, topic_filter, group).unwrap_or_default();
                if !is_forwarded {
                    if let Err(e) = self.inner._forwarded_set(
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
                    self.inner.messages_received_kv.get::<_, PersistedMsg>(msg_id.to_be_bytes().as_slice())
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

    #[inline]
    fn set_forwardeds(
        &self,
        msg_id: PMsgID,
        sub_client_ids: Vec<(ClientId, Option<(TopicFilter, SharedGroup)>)>,
    ) {
        for (client_id, opt) in sub_client_ids {
            if let Err(e) = self.inner._forwarded_set(msg_id, &client_id, opt) {
                log::warn!("set_forwardeds error, {:?}", e);
            }
        }
    }

    #[inline]
    async fn count(&self) -> isize {
        self.messages_received_count.load(Ordering::SeqCst)
    }

    #[inline]
    async fn max(&self) -> isize {
        self.messages_received_max.load(Ordering::SeqCst)
        //self.inner.storage_db.counter_get("messages_received").unwrap_or_default() as isize
    }
}
