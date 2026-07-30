//! In-memory (RAM) message storage implementation.
//!
//! Provides [`RamMessageManager`] and [`RamMessageManagerInner`] for
//! storing, retrieving, and expiring offline messages in memory.
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::convert::From as _f;
use std::mem::size_of;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};
use futures_time::{self, future::FutureExt};
use get_size2::GetSize;
use tokio::{sync::RwLock, time::sleep};

use rmqtt::{
    message::MessageManager,
    retain::RetainTree,
    topic::{Level, Topic},
    types::{
        topic_size, ClientId, ForwardedRecipients, From, MsgID, Publish, SharedGroup, StoredMessage,
        TimestampMillis, TopicFilter,
    },
    utils::{timestamp_millis, Bytesize},
    Result,
};

use crate::config::RamConfig;

enum Msg {
    Store {
        from: From,
        publish: Publish,
        expiry_interval: Duration,
        msg_id: MsgID,
        recipients: Option<ForwardedRecipients>,
    },
    MarkForwarded {
        msg_id: MsgID,
        recipients: ForwardedRecipients,
    },
}

/// An in-memory message manager that implements [`MessageManager`].
#[derive(Clone)]
pub struct RamMessageManager {
    inner: Arc<RamMessageManagerInner>,
}

impl RamMessageManager {
    #[inline]
    pub(crate) async fn new(
        cfg: RamConfig,
        cleanup_count: usize,
        timeout: Duration,
    ) -> Result<RamMessageManager> {
        let (msg_tx, msg_rx) = mpsc::channel::<Msg>(cfg.queue_max);
        let msg_queue_count = Arc::new(AtomicIsize::new(0));

        let this = Self {
            inner: Arc::new(RamMessageManagerInner {
                cfg,
                messages: Default::default(),
                messages_encode: Default::default(),
                topic_tree: Default::default(),
                forwardeds: Default::default(),
                expiries: Default::default(),
                id_gen: AtomicUsize::new(1),
                messages_bytes_size: Default::default(),
                msg_tx,
                msg_queue_count,
                timeout,
            }),
        };

        Ok(this.serve(cleanup_count, msg_rx))
    }

    fn serve(self, max_limit: usize, mut msg_rx: mpsc::Receiver<Msg>) -> Self {
        let msg_mgr = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    msg = msg_rx.next() => {
                        match msg {
                            Some(msg) => {
                                let _ = msg_mgr._process(msg).await;
                                msg_mgr.msg_queue_count.fetch_sub(1, Ordering::Relaxed);
                            }
                            None => {
                                log::error!("Recv failed because receiver is gone");
                                break;
                            }
                        }
                    }
                }
            }
        });

        let msg_mgr = self.clone();
        tokio::spawn(async move {
            sleep(Duration::from_secs(30)).await;
            let mut total_removeds = 0;
            loop {
                let now = std::time::Instant::now();
                let msg_mgr = msg_mgr.clone();
                let removeds = tokio::task::spawn_blocking(move || {
                    tokio::runtime::Handle::current().block_on(async move {
                        match msg_mgr.remove_expired_messages(max_limit).await {
                            Err(e) => {
                                log::warn!("remove expired messages error, {e}");
                                0
                            }
                            Ok(removed) => removed,
                        }
                    })
                })
                .await
                .unwrap_or_default();

                total_removeds += removeds;
                if removeds >= max_limit {
                    continue;
                }
                if total_removeds > 0 {
                    log::debug!(
                        "remove_expired_messages, total removeds: {} cost time: {:?}",
                        total_removeds,
                        now.elapsed()
                    );
                }
                total_removeds = 0;
                sleep(Duration::from_secs(3)).await; //@TODO config enable
            }
        });
        self
    }
}

impl Deref for RamMessageManager {
    type Target = Arc<RamMessageManagerInner>;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

pub struct RamMessageManagerInner {
    cfg: RamConfig,
    pub(crate) messages: scc::HashMap<MsgID, (StoredMessage, usize)>,
    pub(crate) messages_encode: scc::HashMap<MsgID, (Vec<u8>, usize)>,
    pub(crate) topic_tree: RwLock<RetainTree<MsgID>>,
    pub(crate) forwardeds: scc::HashMap<MsgID, BTreeMap<ClientId, Option<(TopicFilter, SharedGroup)>>>,
    pub(crate) expiries: RwLock<BinaryHeap<(Reverse<TimestampMillis>, MsgID)>>,
    pub(crate) id_gen: AtomicUsize,
    messages_bytes_size: AtomicIsize,
    msg_tx: mpsc::Sender<Msg>,
    msg_queue_count: Arc<AtomicIsize>,
    timeout: Duration,
}

impl RamMessageManager {
    #[inline]
    fn messages_bytes_size_add(&self, n: isize) {
        self.messages_bytes_size.fetch_add(n, Ordering::SeqCst);
    }

    #[inline]
    fn messages_bytes_size_sub(&self, n: isize) {
        self.messages_bytes_size.fetch_sub(n, Ordering::SeqCst);
    }

    #[inline]
    pub(crate) fn messages_bytes_size_get(&self) -> usize {
        self.messages_bytes_size.load(Ordering::SeqCst) as usize
    }

    #[inline]
    pub(crate) fn msg_queue_count_get(&self) -> isize {
        self.msg_queue_count.load(Ordering::SeqCst)
    }

    #[inline]
    async fn remove_expired_messages(&self, max_limit: usize) -> Result<usize> {
        let now = timestamp_millis();
        let inner = self.inner.as_ref();

        let removed_msg_ids = {
            let mut expiries = inner.expiries.write().await;
            let mut removeds = Vec::new();
            while let Some((expiry_time_at, _)) = expiries.peek() {
                if removeds.len() > max_limit {
                    break;
                }

                if expiry_time_at.0 > now {
                    let messages_count = self.messages_count();
                    let messages_bytes = self.messages_bytes_size_get();
                    if messages_count > self.cfg.cache_max_count {
                        log::debug!(
                            "The number of messages exceeds the maximum limit, the number of messages is: {}, the limit is: {}",
                            messages_count, self.cfg.cache_max_count
                        );
                    } else if messages_bytes > self.cfg.cache_capacity.as_usize() {
                        log::debug!(
                            "The total number of bytes in the message exceeds the limit, total message bytes are: {:?}, the limit is: {:?}",
                            Bytesize::from(messages_bytes), self.cfg.cache_capacity
                        );
                    } else {
                        break;
                    }
                }

                if let Some((_, msg_id)) = expiries.pop() {
                    removeds.push(msg_id);
                }
            }
            removeds
        };

        for msg_id in removed_msg_ids.iter() {
            if let Ok(Some(msg)) = self.messages_remove(msg_id).await {
                let mut topic =
                    Topic::from_str(&msg.publish.topic).map_err(|e| anyhow!(format!("{:?}", e)))?;
                topic.push(Level::Normal(msg_id.to_string()));
                inner.topic_tree.write().await.remove(&topic);
                inner.forwardeds.remove(msg_id);
            }
        }
        Ok(removed_msg_ids.len())
    }

    #[inline]
    pub(crate) async fn forwardeds_count(&self) -> usize {
        let mut c = 0;
        self.forwardeds
            .scan_async(|_, v| {
                c += v.len();
            })
            .await;
        c
    }

    #[inline]
    fn messages_count(&self) -> usize {
        if self.cfg.encode {
            self.messages_encode.len()
        } else {
            self.messages.len()
        }
    }

    #[inline]
    async fn messages_remove(&self, msg_id: &MsgID) -> Result<Option<StoredMessage>> {
        if self.cfg.encode {
            if let Some((_, (msg, msg_size))) = self.messages_encode.remove_async(msg_id).await {
                self.messages_bytes_size_sub(msg_size as isize);
                Ok(Some(StoredMessage::decode(&msg).map_err(|e| anyhow!(e))?))
            } else {
                Ok(None)
            }
        } else if let Some((_, (msg, msg_size))) = self.messages.remove_async(msg_id).await {
            self.messages_bytes_size_sub(msg_size as isize);
            Ok(Some(msg))
        } else {
            Ok(None)
        }
    }

    #[allow(dead_code)]
    #[inline]
    fn messages_get(&self, msg_id: &MsgID) -> Result<Option<StoredMessage>> {
        if self.cfg.encode {
            if let Some(msg) = self.messages_encode.get(msg_id) {
                Ok(Some(StoredMessage::decode(&msg.get().0).map_err(|e| anyhow!(e))?))
            } else {
                Ok(None)
            }
        } else if let Some(msg) = self.messages.get(msg_id) {
            Ok(Some(msg.get().0.clone()))
        } else {
            Ok(None)
        }
    }

    #[inline]
    async fn messages_get_async(&self, msg_id: &MsgID) -> Result<Option<StoredMessage>> {
        if self.cfg.encode {
            if let Some(msg) = self.messages_encode.get_async(msg_id).await {
                Ok(Some(StoredMessage::decode(&msg.get().0).map_err(|e| anyhow!(e))?))
            } else {
                Ok(None)
            }
        } else if let Some(msg) = self.messages.get_async(msg_id).await {
            Ok(Some(msg.get().0.clone()))
        } else {
            Ok(None)
        }
    }

    #[inline]
    async fn messages_exist(&self, msg_id: &MsgID) -> bool {
        if self.cfg.encode {
            self.messages_encode.contains_async(msg_id).await
        } else {
            self.messages.contains_async(msg_id).await
        }
    }

    #[inline]
    async fn set_forwardeds(&self, msg_id: MsgID, forwardeds: ForwardedRecipients) {
        let mut clientids = self.inner.forwardeds.entry_async(msg_id).await.or_default();
        for (client_id, opts) in forwardeds {
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
        forwardeds: Option<ForwardedRecipients>,
    ) -> Result<()> {
        let mut topic = Topic::from_str(&publish.topic).map_err(|e| anyhow!(format!("{:?}", e)))?;
        topic.push(Level::Normal(msg_id.to_string()));
        let expiry_time_at = timestamp_millis() + expiry_interval.as_millis() as i64;
        let inner = &self.inner;
        let msg = StoredMessage { msg_id, from, publish, expiry_time_at };

        let msg_len =
            topic_size(&topic) + size_of::<MsgID>() * 2 + size_of::<(Reverse<TimestampMillis>, MsgID)>();

        if self.cfg.encode {
            let msg = msg.encode()?;
            let msg_len = msg.len() + msg_len;

            // Acquire messages_encode shard lock and hold it while writing
            // topic_tree and expiries, preventing remove_expired_messages
            // from interleaving between the three maps.
            let entry = inner.messages_encode.entry_async(msg_id).await.insert_entry((msg, msg_len));
            self.messages_bytes_size_add(msg_len as isize);
            inner.topic_tree.write().await.insert(&topic, msg_id);
            inner.expiries.write().await.push((Reverse(expiry_time_at), msg_id));
            if let Some(forwardeds) = forwardeds {
                self.set_forwardeds(msg_id, forwardeds).await;
            }
            drop(entry);
        } else {
            let msg_len = msg.get_size() + msg_len;
            let entry = inner.messages.entry_async(msg_id).await.insert_entry((msg, msg_len));
            self.messages_bytes_size_add(msg_len as isize);
            inner.topic_tree.write().await.insert(&topic, msg_id);
            inner.expiries.write().await.push((Reverse(expiry_time_at), msg_id));
            if let Some(forwardeds) = forwardeds {
                self.set_forwardeds(msg_id, forwardeds).await;
            }
            drop(entry);
        };

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
        if !topic.levels().last().map(|l| matches!(l, Level::MultiWildcard)).unwrap_or_default() {
            topic.push(Level::SingleWildcard);
        }

        let now = std::time::Instant::now();
        let msg_ids = {
            inner
                .topic_tree
                .read()
                .await
                .matches(&topic)
                .iter()
                .map(|(_, msg_id)| *msg_id)
                .collect::<Vec<_>>()
        };
        let elaps = now.elapsed();
        if elaps.as_millis() > 100 {
            log::warn!(
                "slow matches operation, client_id: {}, topic_filter: {}, group: {:?}, elapsed: {:?}",
                client_id,
                topic_filter,
                group,
                elaps
            );
        }

        let count = msg_ids.len();
        let mut matcheds = Vec::with_capacity(count);

        for (i, msg_id) in msg_ids.into_iter().enumerate() {
            if (i & 1023) == 1023 {
                tokio::task::yield_now().await;
            }
            let now = std::time::Instant::now();
            if let Ok(Some(msg)) = self.messages_get_async(&msg_id).await {
                // Use get() instead of entry().or_default() to avoid
                // creating an empty BTreeMap orphan in forwardeds.
                let is_forwarded = self.inner.forwardeds.get_async(&msg_id).await.is_some_and(|cids| {
                    if cids.contains_key(client_id) {
                        true
                    } else if let Some(group) = group {
                        //Check if subscription is shared
                        cids.iter().any(|(_, tf_g)| {
                            tf_g.as_ref().is_some_and(|(tf, g)| g == group && tf == topic_filter)
                        })
                    } else {
                        false
                    }
                });

                log::debug!("is_forwarded: {is_forwarded}, is_expiry: {}", msg.is_expiry());
                if is_forwarded
                    || msg.is_expiry()
                    || msg.publish.target_clientid.as_ref().is_some_and(|t_cid| t_cid != client_id)
                {
                    continue;
                }
                // Record is deferred to the caller after
                // confirming successful delivery.
                matcheds.push((msg_id, msg.from, msg.publish));
            }
            let elaps = now.elapsed();
            if elaps.as_millis() > 100 {
                log::warn!(
                "slow messages_get_async operation, client_id: {}, topic_filter: {}, group: {:?}, elapsed: {:?}",
                client_id,
                topic_filter,
                group,
                elaps
            );
            }
        }
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
         forwardeds.len(): {}, messages_bytes_size:{}, id_gen: {}, waittings: {}, active_count: {}, rate: {:?}",
            vals_size,
            nodes_size,
            inner.messages.len(),
            inner.expiries.read().await.len(),
            inner.forwardeds.len(),
            self.messages_bytes_size_get(),
            inner.id_gen.load(Ordering::Relaxed),
            0,
            0,
            0,
        )
    }

    #[inline]
    async fn _process(&self, msg: Msg) -> Result<()> {
        match msg {
            Msg::Store { from, publish, expiry_interval, msg_id, recipients } => {
                if let Err(e) = self._set(from, publish, expiry_interval, msg_id, recipients).await {
                    log::warn!("Store of the Publish message failed! {e}");
                }
            }
            Msg::MarkForwarded { msg_id, recipients } => {
                if self.messages_exist(&msg_id).await {
                    self.set_forwardeds(msg_id, recipients).await;
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl MessageManager for RamMessageManager {
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
        recipients: Option<ForwardedRecipients>,
    ) -> Result<()> {
        let timeout = self.inner.timeout;
        let mut tx = self.inner.msg_tx.clone();

        // Increment BEFORE send so fetch_sub can never race ahead.
        self.msg_queue_count.fetch_add(1, Ordering::Relaxed);

        let send_fut = tx.send(Msg::Store { from, publish: p, expiry_interval, msg_id, recipients });
        let res = if timeout > Duration::ZERO {
            send_fut
                .timeout(futures_time::time::Duration::from(timeout))
                .await
                .map_err(|_e| anyhow!("store timeout"))
        } else {
            Ok(send_fut.await)
        };
        match res {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => {
                self.msg_queue_count.fetch_sub(1, Ordering::Relaxed);
                log::warn!("RamMessageManager store error, {e}");
                Err(anyhow!(e))
            }
            Err(e) => {
                self.msg_queue_count.fetch_sub(1, Ordering::Relaxed);
                log::warn!("RamMessageManager store timeout, {e}");
                Err(e)
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
        let res = self._get(client_id, topic_filter, group).await;
        let elaps = now.elapsed();
        if elaps.as_secs() > 1 {
            log::warn!(
                "slow message fetch operation, client_id: {}, topic_filter: {}, group: {:?}, elapsed: {:?}",
                client_id,
                topic_filter,
                group,
                elaps
            );
        }
        res
    }

    #[inline]
    async fn mark_forwarded(&self, msg_id: MsgID, forwardeds: ForwardedRecipients) -> Result<()> {
        // Send through msg_tx for batched processing; the batch processor
        // will verify message existence before writing forwardeds.
        // The channel ensures ordering so that any preceding Store for
        // the same msg_id is already committed.
        let timeout = self.inner.timeout;
        let mut tx = self.inner.msg_tx.clone();

        // Increment BEFORE send so fetch_sub can never race ahead.
        self.msg_queue_count.fetch_add(1, Ordering::Relaxed);

        let send_fut = tx.send(Msg::MarkForwarded { msg_id, recipients: forwardeds });
        let res = if timeout > Duration::ZERO {
            send_fut.timeout(futures_time::time::Duration::from(timeout)).await.map_err(|e| anyhow!(e))
        } else {
            Ok(send_fut.await)
        };
        match res {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => {
                self.msg_queue_count.fetch_sub(1, Ordering::Relaxed);
                log::warn!("RamMessageManager mark_forwarded error, {e}");
                Err(anyhow!(e))
            }
            Err(e) => {
                self.msg_queue_count.fetch_sub(1, Ordering::Relaxed);
                log::warn!("RamMessageManager mark_forwarded timeout, {e}");
                Err(e)
            }
        }
    }

    #[inline]
    fn merge_on_read(&self) -> bool {
        true
    }

    #[inline]
    async fn count(&self) -> isize {
        if self.cfg.encode {
            self.inner.messages_encode.len() as isize
        } else {
            self.inner.messages.len() as isize
        }
    }

    #[inline]
    async fn max(&self) -> isize {
        self.messages_count() as isize
    }

    #[inline]
    fn enable(&self) -> bool {
        true
    }
}

#[test]
fn test_message_manager() {
    use rmqtt::codec::v5::PublishProperties;
    use rmqtt::types::{CodecPublish, From, Id, QoS, TopicName};

    let runner = async move {
        let cfg = RamConfig::default();
        let msg_mgr = Box::leak(Box::new(
            RamMessageManager::new(cfg, usize::MAX, Duration::from_millis(5000)).await.unwrap(),
        )) as &'static RamMessageManager;
        sleep(Duration::from_millis(10)).await;
        let f = From::from_custom(Id::from(1, ClientId::from("test-001")));
        let p = CodecPublish {
            dup: false,
            retain: false,
            qos: QoS::try_from(1).unwrap(),
            topic: TopicName::from(""),
            packet_id: Some(std::num::NonZeroU16::try_from(1).unwrap()),
            payload: bytes::Bytes::from("test ..."),
            properties: Some(PublishProperties::default()),
        };

        let mut p = <CodecPublish as Into<Publish>>::into(p).create_time(timestamp_millis());

        let now = std::time::Instant::now();
        for i in 0..5 {
            p.topic = TopicName::from("/xx/yy/zz");
            let msg_id = msg_mgr.next_msg_id();
            msg_mgr
                .store(msg_id, f.clone(), p.clone(), Duration::from_secs(i + 2), Some(Vec::new()))
                .await
                .unwrap();
        }

        for i in 0..5 {
            p.topic = TopicName::from("/xx/yy/cc");
            let msg_id = msg_mgr.next_msg_id();
            msg_mgr
                .store(msg_id, f.clone(), p.clone(), Duration::from_secs(i + 2), Some(Vec::new()))
                .await
                .unwrap();
        }

        for i in 0..5 {
            p.topic = TopicName::from("/xx/yy/");
            let msg_id = msg_mgr.next_msg_id();
            msg_mgr
                .store(msg_id, f.clone(), p.clone(), Duration::from_secs(i + 2), Some(Vec::new()))
                .await
                .unwrap();
        }

        for i in 0..5 {
            p.topic = TopicName::from("/xx/yy/ee/ff");
            let msg_id = msg_mgr.next_msg_id();
            msg_mgr
                .store(msg_id, f.clone(), p.clone(), Duration::from_secs(i + 2), Some(Vec::new()))
                .await
                .unwrap();
        }

        for i in 0..5 {
            p.topic = TopicName::from("/foo/yy/ee");
            let msg_id = msg_mgr.next_msg_id();
            msg_mgr
                .store(msg_id, f.clone(), p.clone(), Duration::from_secs(i + 2), Some(Vec::new()))
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

#[test]
fn test_dedup_same_client() {
    use rmqtt::codec::v5::PublishProperties;
    use rmqtt::types::{CodecPublish, From, Id, QoS, TopicName};

    let runner = async move {
        let cfg = RamConfig::default();
        let msg_mgr = Box::leak(Box::new(
            RamMessageManager::new(cfg, usize::MAX, Duration::from_millis(5000)).await.unwrap(),
        )) as &'static RamMessageManager;
        sleep(Duration::from_millis(10)).await;
        let f = From::from_custom(Id::from(1, ClientId::from("pub-001")));
        let p = CodecPublish {
            dup: false,
            retain: false,
            qos: QoS::try_from(1).unwrap(),
            topic: TopicName::from("/sensor/temp"),
            packet_id: Some(std::num::NonZeroU16::try_from(1).unwrap()),
            payload: bytes::Bytes::from("25.3"),
            properties: Some(PublishProperties::default()),
        };
        let p = <CodecPublish as Into<Publish>>::into(p).create_time(timestamp_millis());

        // Store one message
        let msg_id = msg_mgr.next_msg_id();
        msg_mgr.store(msg_id, f.clone(), p.clone(), Duration::from_secs(30), Some(Vec::new())).await.unwrap();
        sleep(Duration::from_millis(10)).await;

        let tf = TopicFilter::from("/sensor/temp");

        // First get should return 1 message
        let msgs = msg_mgr.get("c-001", &tf, None).await.unwrap();
        assert_eq!(msgs.len(), 1, "first get should return 1 message");

        // Record c-001 via mark_forwarded (simulating the effect of
        // forwards() recording successful delivery).
        msg_mgr.mark_forwarded(msg_id, vec![(ClientId::from("c-001"), None)]).await.unwrap();
        sleep(Duration::from_millis(10)).await;

        // Second get (same client, same topic) should return 0 (dedup)
        let msgs = msg_mgr.get("c-001", &tf, None).await.unwrap();
        assert_eq!(msgs.len(), 0, "second get should return 0 (dedup)");

        // Different client should still get the message
        let msgs = msg_mgr.get("c-002", &tf, None).await.unwrap();
        assert_eq!(msgs.len(), 1, "different client should get the message");
    };

    tokio::runtime::Runtime::new().unwrap().block_on(runner);
}

#[test]
fn test_mark_forwarded_basic() {
    use rmqtt::codec::v5::PublishProperties;
    use rmqtt::types::{CodecPublish, From, Id, QoS, TopicName};

    let runner = async move {
        let cfg = RamConfig::default();
        let msg_mgr = Box::leak(Box::new(
            RamMessageManager::new(cfg, usize::MAX, Duration::from_millis(5000)).await.unwrap(),
        )) as &'static RamMessageManager;
        sleep(Duration::from_millis(10)).await;
        let f = From::from_custom(Id::from(1, ClientId::from("pub-001")));
        let p = CodecPublish {
            dup: false,
            retain: false,
            qos: QoS::try_from(1).unwrap(),
            topic: TopicName::from("/sensor/temp"),
            packet_id: Some(std::num::NonZeroU16::try_from(1).unwrap()),
            payload: bytes::Bytes::from("25.3"),
            properties: Some(PublishProperties::default()),
        };
        let p = <CodecPublish as Into<Publish>>::into(p).create_time(timestamp_millis());

        // Store message with empty subscriber list
        let msg_id = msg_mgr.next_msg_id();
        msg_mgr.store(msg_id, f.clone(), p.clone(), Duration::from_secs(30), Some(Vec::new())).await.unwrap();
        sleep(Duration::from_millis(10)).await;

        // Initially no client has been forwarded
        let msgs = msg_mgr.get("c-001", &TopicFilter::from("/sensor/temp"), None).await.unwrap();
        assert_eq!(msgs.len(), 1);

        // Add c-001 to forwarded list
        msg_mgr.mark_forwarded(msg_id, vec![(ClientId::from("c-001"), None)]).await.unwrap();
        sleep(Duration::from_millis(20)).await;

        // Now c-001 should be deduplicated
        let msgs = msg_mgr.get("c-001", &TopicFilter::from("/sensor/temp"), None).await.unwrap();
        assert_eq!(msgs.len(), 0, "c-001 should be deduplicated after mark_forwarded");
    };

    tokio::runtime::Runtime::new().unwrap().block_on(runner);
}

#[test]
fn test_mark_forwarded_non_existent() {
    let runner = async move {
        let cfg = RamConfig::default();
        let msg_mgr = Box::leak(Box::new(
            RamMessageManager::new(cfg, usize::MAX, Duration::from_millis(5000)).await.unwrap(),
        )) as &'static RamMessageManager;
        sleep(Duration::from_millis(10)).await;

        // Calling mark_forwarded with a msg_id that was never stored
        // should not panic, crash, or leak memory
        let result = msg_mgr.mark_forwarded(99999, vec![(ClientId::from("ghost-client"), None)]).await;
        assert!(result.is_ok(), "non-existent msg_id should return Ok(())");

        // forwardeds should not contain the ghost entry
        let count = msg_mgr.forwardeds_count().await;
        assert_eq!(count, 0, "forwardeds should be empty after non-existent add");
    };

    tokio::runtime::Runtime::new().unwrap().block_on(runner);
}

#[test]
fn test_shared_subscription_dedup() {
    use rmqtt::codec::v5::PublishProperties;
    use rmqtt::types::{CodecPublish, From, Id, QoS, TopicName};

    let runner = async move {
        let cfg = RamConfig::default();
        let msg_mgr = Box::leak(Box::new(
            RamMessageManager::new(cfg, usize::MAX, Duration::from_millis(5000)).await.unwrap(),
        )) as &'static RamMessageManager;
        sleep(Duration::from_millis(10)).await;
        let f = From::from_custom(Id::from(1, ClientId::from("pub-001")));
        let p = CodecPublish {
            dup: false,
            retain: false,
            qos: QoS::try_from(1).unwrap(),
            topic: TopicName::from("/sensor/temp"),
            packet_id: Some(std::num::NonZeroU16::try_from(1).unwrap()),
            payload: bytes::Bytes::from("25.3"),
            properties: Some(PublishProperties::default()),
        };
        let p = <CodecPublish as Into<Publish>>::into(p).create_time(timestamp_millis());

        let msg_id = msg_mgr.next_msg_id();
        msg_mgr.store(msg_id, f.clone(), p.clone(), Duration::from_secs(30), Some(Vec::new())).await.unwrap();
        sleep(Duration::from_millis(10)).await;

        let group = SharedGroup::from("group1");
        let tf = TopicFilter::from("/sensor/temp");

        // First client in shared group gets the message
        let msgs = msg_mgr.get("c-001", &tf, Some(&group)).await.unwrap();
        assert_eq!(msgs.len(), 1, "first client in shared group gets message");

        // Record c-001 with shared group info, simulating forwards() success.
        msg_mgr
            .mark_forwarded(
                msg_id,
                vec![(ClientId::from("c-001"), Some((TopicFilter::from("/sensor/temp"), group.clone())))],
            )
            .await
            .unwrap();
        sleep(Duration::from_millis(20)).await;

        // Second client in same shared group should NOT get it (dedup by shared group)
        let msgs = msg_mgr.get("c-002", &tf, Some(&group)).await.unwrap();
        assert_eq!(msgs.len(), 0, "second client in same group is deduplicated");

        // But a different client without shared group should still get it
        let msgs = msg_mgr.get("c-003", &tf, None).await.unwrap();
        assert_eq!(msgs.len(), 1, "non-shared client still gets the message");
    };

    tokio::runtime::Runtime::new().unwrap().block_on(runner);
}

#[test]
fn test_message_expiry() {
    use rmqtt::codec::v5::PublishProperties;
    use rmqtt::types::{CodecPublish, From, Id, QoS, TopicName};

    let runner = async move {
        let cfg = RamConfig::default();
        let msg_mgr = Box::leak(Box::new(
            RamMessageManager::new(cfg, usize::MAX, Duration::from_millis(5000)).await.unwrap(),
        )) as &'static RamMessageManager;
        sleep(Duration::from_millis(10)).await;
        let f = From::from_custom(Id::from(1, ClientId::from("pub-001")));
        let p = CodecPublish {
            dup: false,
            retain: false,
            qos: QoS::try_from(1).unwrap(),
            topic: TopicName::from("/sensor/temp"),
            packet_id: Some(std::num::NonZeroU16::try_from(1).unwrap()),
            payload: bytes::Bytes::from("25.3"),
            properties: Some(PublishProperties::default()),
        };
        let p = <CodecPublish as Into<Publish>>::into(p).create_time(timestamp_millis());

        // Store a message with very short expiry (1 second)
        let msg_id = msg_mgr.next_msg_id();
        msg_mgr.store(msg_id, f.clone(), p.clone(), Duration::from_secs(1), Some(Vec::new())).await.unwrap();
        sleep(Duration::from_millis(10)).await;

        // Should be available before expiry
        let msgs = msg_mgr.get("c-001", &TopicFilter::from("/sensor/temp"), None).await.unwrap();
        assert_eq!(msgs.len(), 1, "message available before expiry");

        // Wait for expiry
        sleep(Duration::from_secs(2)).await;

        // Should be gone after expiry
        let msgs = msg_mgr.get("c-002", &TopicFilter::from("/sensor/temp"), None).await.unwrap();
        assert_eq!(msgs.len(), 0, "message expired");
    };

    tokio::runtime::Runtime::new().unwrap().block_on(runner);
}

#[test]
fn test_wildcard_matching() {
    use rmqtt::codec::v5::PublishProperties;
    use rmqtt::types::{CodecPublish, From, Id, QoS, TopicName};

    let runner = async move {
        let cfg = RamConfig::default();
        let msg_mgr = Box::leak(Box::new(
            RamMessageManager::new(cfg, usize::MAX, Duration::from_millis(5000)).await.unwrap(),
        )) as &'static RamMessageManager;
        sleep(Duration::from_millis(10)).await;
        let f = From::from_custom(Id::from(1, ClientId::from("pub-001")));
        let mut p = CodecPublish {
            dup: false,
            retain: false,
            qos: QoS::try_from(1).unwrap(),
            topic: TopicName::from(""),
            packet_id: Some(std::num::NonZeroU16::try_from(1).unwrap()),
            payload: bytes::Bytes::from("data"),
            properties: Some(PublishProperties::default()),
        };

        // Store messages on different topics
        let topics = vec!["/a/b/c", "/a/b/d", "/a/x/y", "/x/y/z"];
        for topic in &topics {
            p.topic = TopicName::from(*topic);
            let msg_id = msg_mgr.next_msg_id();
            msg_mgr
                .store(
                    msg_id,
                    f.clone(),
                    <CodecPublish as Into<Publish>>::into(p.clone()).create_time(timestamp_millis()),
                    Duration::from_secs(30),
                    Some(Vec::new()),
                )
                .await
                .unwrap();
        }
        sleep(Duration::from_millis(20)).await;

        // Multi-level wildcard /a/b/# should match /a/b/c and /a/b/d
        let msgs = msg_mgr.get("c-001", &TopicFilter::from("/a/b/#"), None).await.unwrap();
        assert_eq!(msgs.len(), 2, "/a/b/# matches 2 topics");

        // Single-level wildcard /a/+/c should match /a/b/c
        let msgs = msg_mgr.get("c-002", &TopicFilter::from("/a/+/c"), None).await.unwrap();
        assert_eq!(msgs.len(), 1, "/a/+/c matches 1 topic");

        // Exact match
        let msgs = msg_mgr.get("c-003", &TopicFilter::from("/a/b/c"), None).await.unwrap();
        assert_eq!(msgs.len(), 1, "exact match");
    };

    tokio::runtime::Runtime::new().unwrap().block_on(runner);
}

#[test]
fn test_methods() {
    let runner = async move {
        let cfg = RamConfig::default();
        let msg_mgr = Box::leak(Box::new(
            RamMessageManager::new(cfg, usize::MAX, Duration::from_millis(5000)).await.unwrap(),
        )) as &'static RamMessageManager;
        sleep(Duration::from_millis(10)).await;

        // Initial state
        assert!(msg_mgr.enable(), "enable() should return true");
        assert!(msg_mgr.merge_on_read(), "merge_on_read() should return true");
        assert_eq!(msg_mgr.count().await, 0, "count should be 0 initially");
        assert!(msg_mgr.max().await >= 0, "max() should be >= 0");

        // next_msg_id generates unique incrementing IDs
        let id1 = msg_mgr.next_msg_id();
        let id2 = msg_mgr.next_msg_id();
        let id3 = msg_mgr.next_msg_id();
        assert!(id1 < id2, "msg_ids should be incrementing");
        assert!(id2 < id3, "msg_ids should be incrementing");
    };

    tokio::runtime::Runtime::new().unwrap().block_on(runner);
}
