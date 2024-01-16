use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::convert::From as _f;
use std::iter::Iterator;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use ntex_mqtt::TopicLevel;
use once_cell::sync::OnceCell;
use rust_box::task_exec_queue::{Builder, SpawnExt, TaskExecQueue};
use tokio::sync::RwLock;
use tokio::time::sleep;

use rmqtt::{
    anyhow::anyhow, async_trait, log, ntex_mqtt, once_cell, rust_box, scc, timestamp_millis, tokio, Publish,
};

use crate::config::RamConfig;
use rmqtt::{
    broker::retain::RetainTree, broker::topic::Topic, broker::MessageManager, ClientId, From, MsgID, Result,
    SharedGroup, StoredMessage, TimestampMillis, TopicFilter,
};

static INSTANCE: OnceCell<RamMessageManager> = OnceCell::new();

#[inline]
pub(crate) async fn get_or_init(cfg: RamConfig, cleanup_count: usize) -> Result<&'static RamMessageManager> {
    if let Some(msg_mgr) = INSTANCE.get() {
        return Ok(msg_mgr);
    }
    let msg_mgr = RamMessageManager::new(cfg, cleanup_count).await?;
    INSTANCE.set(msg_mgr).map_err(|_| anyhow!("init error!"))?;
    if let Some(msg_mgr) = INSTANCE.get() {
        Ok(msg_mgr)
    } else {
        unreachable!()
    }
}

pub struct RamMessageManager {
    inner: Arc<RamMessageManagerInner>,
    pub(crate) exec: TaskExecQueue,
}

impl RamMessageManager {
    #[inline]
    async fn new(cfg: RamConfig, cleanup_count: usize) -> Result<RamMessageManager> {
        let exec = Self::serve(cfg, cleanup_count)?;
        Ok(Self { inner: Arc::new(RamMessageManagerInner::default()), exec })
    }

    fn serve(cfg: RamConfig, max_limit: usize) -> Result<TaskExecQueue> {
        let (exec, task_runner) = Builder::default().workers(1000).queue_max(300_000).build();

        tokio::spawn(async move {
            task_runner.await;
        });

        tokio::spawn(async move {
            sleep(Duration::from_secs(30)).await;
            loop {
                let now = std::time::Instant::now();
                let removeds = if let Some(msg_mgr) = INSTANCE.get() {
                    tokio::task::spawn_blocking(move || {
                        tokio::runtime::Handle::current().block_on(async move {
                            match msg_mgr.remove_expired_messages(max_limit).await {
                                Err(e) => {
                                    log::warn!("remove expired messages error, {:?}", e);
                                    0
                                }
                                Ok(removed) => removed,
                            }
                        })
                    })
                    .await
                    .unwrap_or_default()
                } else {
                    0
                };
                if removeds >= max_limit {
                    continue;
                }
                if removeds > 0 {
                    log::info!(
                        "remove_expired_messages, removeds: {} cost time: {:?}",
                        removeds,
                        now.elapsed()
                    );
                }
                sleep(Duration::from_secs(30)).await; //@TODO config enable
            }
        });
        Ok(exec)
    }
}

impl Deref for RamMessageManager {
    type Target = Arc<RamMessageManagerInner>;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

#[derive(Default)]
pub struct RamMessageManagerInner {
    pub(crate) messages: scc::HashMap<MsgID, StoredMessage>,
    pub(crate) topic_tree: RwLock<RetainTree<MsgID>>,
    pub(crate) forwardeds: scc::HashMap<MsgID, BTreeMap<ClientId, Option<(TopicFilter, SharedGroup)>>>,
    pub(crate) expiries: RwLock<BinaryHeap<(Reverse<TimestampMillis>, MsgID)>>,
    pub(crate) id_gen: AtomicUsize,
}

impl RamMessageManager {
    #[inline]
    async fn remove_expired_messages(&self, max_limit: usize) -> Result<usize> {
        let now = timestamp_millis();
        let inner = self.inner.as_ref();

        let removed_msg_ids = {
            let mut expiries = inner.expiries.write().await;
            let mut removeds = Vec::new();
            while let Some((expiry_time_at, _)) = expiries.peek() {
                if expiry_time_at.0 > now || removeds.len() > max_limit {
                    break;
                }
                if let Some((_, msg_id)) = expiries.pop() {
                    removeds.push(msg_id);
                }
            }
            removeds
        };

        for msg_id in removed_msg_ids.iter() {
            if let Some((_, msg)) = inner.messages.remove_async(&msg_id).await {
                let mut topic =
                    Topic::from_str(&msg.publish.topic).map_err(|e| anyhow!(format!("{:?}", e)))?;
                topic.push(TopicLevel::Normal(msg_id.to_string()));
                inner.topic_tree.write().await.remove(&topic);
                inner.forwardeds.remove(&msg_id);
            }
        }
        Ok(removed_msg_ids.len())
    }

    #[inline]
    async fn remove_expired_messages_old(&self, max_limit: usize) -> Result<usize> {
        let now = timestamp_millis();
        let inner = self.inner.as_ref();
        let mut expiries = inner.expiries.write().await;
        let mut topic_tree = inner.topic_tree.write().await;
        let mut expired_count = 0;
        while let Some((expiry_time, _)) = expiries.peek() {
            let expiry_time = expiry_time.0;
            if expiry_time > now || expired_count > max_limit {
                break;
            }
            expired_count += 1;
            if let Some((_, msg_id)) = expiries.pop() {
                if let Some((_, msg)) = inner.messages.remove_async(&msg_id).await {
                    let mut topic =
                        Topic::from_str(&msg.publish.topic).map_err(|e| anyhow!(format!("{:?}", e)))?;
                    topic.push(TopicLevel::Normal(msg_id.to_string()));
                    let _ = topic_tree.remove(&topic);
                    inner.forwardeds.remove(&msg_id);
                }
            }
        }
        Ok(expired_count)
    }

    #[inline]
    fn set_forwardeds(
        &self,
        msg_id: MsgID,
        sub_client_ids: Vec<(ClientId, Option<(TopicFilter, SharedGroup)>)>,
    ) {
        let mut clientids = self.inner.forwardeds.entry(msg_id).or_default();
        for (client_id, opts) in sub_client_ids {
            clientids.get_mut().insert(ClientId::from(client_id), opts);
        }
    }

    #[inline]
    async fn _set(
        &self,
        from: From,
        publish: Publish,
        expiry_interval: Duration,
        msg_id: MsgID,
        sub_client_ids: Option<Vec<(ClientId, Option<(TopicFilter, SharedGroup)>)>>,
    ) -> Result<()> {
        let mut topic = Topic::from_str(&publish.topic).map_err(|e| anyhow!(format!("{:?}", e)))?;
        let expiry_time_at = timestamp_millis() + expiry_interval.as_millis() as i64;
        let inner = &self.inner;
        let msg = StoredMessage { msg_id, from, publish, expiry_time_at };
        topic.push(TopicLevel::Normal(msg_id.to_string()));
        inner.messages.insert_async(msg_id, msg).await.map_err(|_| anyhow!("messages insert error"))?;
        inner.topic_tree.write().await.insert(&topic, msg_id);
        inner.expiries.write().await.push((Reverse(expiry_time_at), msg_id));
        if let Some(sub_client_ids) = sub_client_ids {
            self.set_forwardeds(msg_id, sub_client_ids);
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
        let inner = &self.inner;
        let mut topic = Topic::from_str(topic_filter).map_err(|e| anyhow!(format!("{:?}", e)))?;
        if !topic.levels().last().map(|l| matches!(l, TopicLevel::MultiWildcard)).unwrap_or_default() {
            topic.push(TopicLevel::SingleWildcard);
        }

        let matcheds = {
            inner
                .topic_tree
                .read()
                .await
                .matches(&topic)
                .iter()
                .map(|(_, msg_id)| *msg_id)
                .collect::<Vec<_>>()
        };

        let matcheds = matcheds
            .into_iter()
            .filter_map(|msg_id| {
                if let Some(msg) = inner.messages.get(&msg_id) {
                    let mut clientids = self.inner.forwardeds.entry(msg_id).or_default();
                    let is_forwarded = if clientids.get().contains_key(client_id) {
                        true
                    } else if let Some(group) = group {
                        //Check if subscription is shared
                        clientids.get().iter().any(|(_, tf_g)| {
                            if let Some((tf, g)) = tf_g.as_ref() {
                                g == group && tf == topic_filter
                            } else {
                                false
                            }
                        })
                    } else {
                        false
                    };

                    log::debug!("is_forwarded: {}", is_forwarded);
                    if is_forwarded {
                        None
                    } else {
                        let msg = msg.get();
                        if msg.is_expiry() {
                            None
                        } else {
                            clientids.get_mut().insert(
                                ClientId::from(client_id),
                                group.map(|g| (TopicFilter::from(topic_filter), g.clone())),
                            );
                            Some((msg_id, msg.from.clone(), msg.publish.clone()))
                        }
                    }
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        Ok(matcheds)
    }

    #[allow(dead_code)]
    async fn sprint_status(&self) -> String {
        let inner = self.inner.as_ref();
        let (vals_size, nodes_size) = {
            let topic_tree = inner.topic_tree.read().await;
            (topic_tree.values_size(), topic_tree.nodes_size())
        };
        format!(
            "vals_size: {}, nodes_size: {}, messages.len(): {}, expiries.len(): {}, \
         forwardeds.len(): {}, id_gen: {}, waittings: {}, active_count: {}, rate: {:?}",
            vals_size,
            nodes_size,
            inner.messages.len(),
            inner.expiries.read().await.len(),
            inner.forwardeds.len(),
            inner.id_gen.load(Ordering::Relaxed),
            self.exec.waiting_count(),
            self.exec.active_count(),
            self.exec.rate().await
        )
    }
}

#[async_trait]
impl MessageManager for &'static RamMessageManager {
    #[inline]
    fn next_msg_id(&self) -> MsgID {
        self.inner.id_gen.fetch_add(1, Ordering::SeqCst)
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
        let this: &'static RamMessageManager = self;
        async move {
            if let Err(e) = this._set(from, p, expiry_interval, msg_id, sub_client_ids).await {
                log::warn!("Store of the Publish message failed! {:?}", e);
            }
        }
        .spawn(&self.exec)
        .await
        .map_err(|e| anyhow!(e.to_string()))?;
        Ok(())
    }

    #[inline]
    async fn get(
        &self,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
    ) -> Result<Vec<(MsgID, From, Publish)>> {
        self._get(client_id, topic_filter, group).await
    }

    #[inline]
    fn should_merge_on_get(&self) -> bool {
        true
    }

    #[inline]
    async fn count(&self) -> isize {
        self.inner.messages.len() as isize
    }

    #[inline]
    async fn max(&self) -> isize {
        self.exec.completed_count().await
    }

    #[inline]
    fn enable(&self) -> bool {
        true
    }
}

#[test]
fn test_message_manager() {
    use rmqtt::{bytes, chrono, From, Id, PublishProperties, QoS, TopicName};

    let runner = async move {
        let cfg = RamConfig::default();
        let msg_mgr = RamMessageManager::instance(cfg);
        sleep(Duration::from_millis(10)).await;
        let f = From::from_custom(Id::from(1, ClientId::from("test-001")));
        let mut p = Publish {
            dup: false,
            retain: false,
            qos: QoS::try_from(1).unwrap(),
            topic: TopicName::from(""),
            packet_id: Some(std::num::NonZeroU16::try_from(1).unwrap()),
            payload: bytes::Bytes::from("test ..."),
            properties: PublishProperties::default(),
            create_time: timestamp_millis(),
        };

        assert_eq!(timestamp_millis(), chrono::Local::now().timestamp_millis());

        let now = std::time::Instant::now();
        for i in 0..5 {
            p.topic = TopicName::from("/xx/yy/zz");
            let msg_id = msg_mgr.next_msg_id();
            msg_mgr
                .store(msg_id, f.clone(), p.clone(), Duration::from_secs(i + 2), Vec::new())
                .await
                .unwrap();
        }

        for i in 0..5 {
            p.topic = TopicName::from("/xx/yy/cc");
            let msg_id = msg_mgr.next_msg_id();
            msg_mgr
                .store(msg_id, f.clone(), p.clone(), Duration::from_secs(i + 2), Vec::new())
                .await
                .unwrap();
        }

        for i in 0..5 {
            p.topic = TopicName::from("/xx/yy/");
            let msg_id = msg_mgr.next_msg_id();
            msg_mgr
                .store(msg_id, f.clone(), p.clone(), Duration::from_secs(i + 2), Vec::new())
                .await
                .unwrap();
        }

        for i in 0..5 {
            p.topic = TopicName::from("/xx/yy/ee/ff");
            let msg_id = msg_mgr.next_msg_id();
            msg_mgr
                .store(msg_id, f.clone(), p.clone(), Duration::from_secs(i + 2), Vec::new())
                .await
                .unwrap();
        }

        for i in 0..5 {
            p.topic = TopicName::from("/foo/yy/ee");
            let msg_id = msg_mgr.next_msg_id();
            msg_mgr
                .store(msg_id, f.clone(), p.clone(), Duration::from_secs(i + 2), Vec::new())
                .await
                .unwrap();
        }

        println!("cost time: {:?}", now.elapsed());
        sleep(Duration::from_millis(10)).await;
        println!("{}", msg_mgr.sprint_status().await);

        let tf = TopicFilter::from("/xx/yy/#");
        let msgs = msg_mgr.get("c-id-001", &tf, None).await.unwrap();
        println!("===>>> msgs len: {}", msgs.len());
        assert_eq!(msgs.len(), 20);
        //for (f, p) in msgs {
        //    println!("> from: {:?}, publish: {:?}", f, p);
        //}

        let tf = TopicFilter::from("/xx/yy/cc");
        let msgs = msg_mgr.get("c-id-002", &tf, None).await.unwrap();
        println!("===>>> msgs len: {}", msgs.len());
        assert_eq!(msgs.len(), 5);

        let tf = TopicFilter::from("/foo/yy/ee");
        let msgs = msg_mgr.get("", &tf, None).await.unwrap();
        assert_eq!(msgs.len(), 5);
        println!("msgs len: {}", msgs.len());
        //for (f, p) in msgs {
        //    println!("from: {:?}, publish: {:?}", f, p);
        //}

        sleep(Duration::from_millis(1000 * 5)).await;
        println!("{}", msg_mgr.sprint_status().await);
    };

    tokio::runtime::Runtime::new().unwrap().block_on(runner);
}
