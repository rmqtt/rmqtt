use std::borrow::Cow;
use std::io::{Read, Write};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use rust_box::task_exec_queue::SpawnExt;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use rmqtt::context::ServerContext;
use rmqtt::{
    router::{DefaultRouter, Router},
    types::{
        AllRelationsMap, ClientId, Id, IsOnline, NodeId, Route, SubRelationsMap, SubscriptionOptions,
        TimestampMillis, Topic, TopicFilter, TopicName,
    },
    utils::timestamp_millis,
    utils::Counter,
    Result, SubsSearchParams, SubsSearchResult,
};
use rmqtt_raft::{Error, Mailbox, Result as RaftResult, Store};

use crate::task_exec_queue;

use super::config::{retry, Compression, BACKOFF_STRATEGY};
use super::message::{Message, MessageReply};

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;
type DashMap<K, V> = dashmap::DashMap<K, V, ahash::RandomState>;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub(crate) struct ClientStatus {
    pub id: Id,
    pub online: IsOnline,
    pub handshaking: bool,
    pub handshak_duration: TimestampMillis,
}

impl ClientStatus {
    fn new(id: Id, online: IsOnline, handshaking: bool) -> Self {
        Self { id, online, handshaking, handshak_duration: timestamp_millis() }
    }

    pub fn handshaking(&self, try_lock_timeout: Duration) -> bool {
        self.handshaking
            && (timestamp_millis()
                < (self.handshak_duration + try_lock_timeout.as_millis() as TimestampMillis))
    }
}

#[derive(Clone)]
pub(crate) struct ClusterRouter {
    inner: DefaultRouter,
    raft_mailbox: Arc<RwLock<Option<Mailbox>>>,
    client_states: Arc<DashMap<ClientId, ClientStatus>>,
    pub try_lock_timeout: Duration,
    compression: Option<Compression>,
}

impl ClusterRouter {
    #[inline]
    pub(crate) fn new(
        scx: ServerContext,
        try_lock_timeout: Duration,
        compression: Option<Compression>,
    ) -> Self {
        Self {
            inner: DefaultRouter::new(Some(scx)),
            raft_mailbox: Arc::new(RwLock::new(None)),
            client_states: Arc::new(DashMap::default()),
            try_lock_timeout,
            compression,
        }
    }

    #[inline]
    pub(crate) fn _inner(&self) -> &DefaultRouter {
        &self.inner
    }

    #[inline]
    pub(crate) async fn set_raft_mailbox(&self, raft_mailbox: Mailbox) {
        self.raft_mailbox.write().await.replace(raft_mailbox);
    }

    #[inline]
    pub(crate) async fn raft_mailbox(&self) -> Mailbox {
        if let Some(mailbox) = self.raft_mailbox.read().await.as_ref() {
            mailbox.clone()
        } else {
            unreachable!()
        }
    }

    #[inline]
    pub(crate) fn _client_node_id(&self, client_id: &str) -> Option<NodeId> {
        self.client_states.get(client_id).map(|entry| entry.id.node_id)
    }

    #[inline]
    pub(crate) fn id(&self, client_id: &str) -> Option<Id> {
        self.client_states.get(client_id).map(|entry| entry.id.clone())
    }

    #[inline]
    pub(crate) fn states_count(&self) -> usize {
        self.client_states.len()
    }

    #[inline]
    pub(crate) fn status(&self, client_id: &str) -> Option<ClientStatus> {
        self.client_states.get(client_id).map(|entry| entry.value().clone())
    }

    #[inline]
    pub(crate) fn _handshakings(&self) -> usize {
        self.client_states.iter().filter_map(|entry| if entry.handshaking { Some(()) } else { None }).count()
    }
}

#[async_trait]
impl Router for ClusterRouter {
    #[inline]
    async fn add(&self, topic_filter: &str, id: Id, opts: SubscriptionOptions) -> Result<()> {
        log::debug!("[Router.add] topic_filter: {:?}, id: {:?}, opts: {:?}", topic_filter, id, opts);

        let msg = Message::Add { topic_filter, id, opts }.encode()?;
        let mailbox = self.raft_mailbox().await;
        let _ = async move { mailbox.send_proposal(msg).await.map_err(anyhow::Error::new) }
            .spawn(task_exec_queue())
            .result()
            .await
            .map_err(|_| anyhow!("Router::add(..), task execution failure"))??;
        Ok(())
    }

    #[inline]
    async fn remove(&self, topic_filter: &str, id: Id) -> Result<bool> {
        log::debug!("[Router.remove] topic_filter: {:?}, id: {:?}", topic_filter, id);
        let msg = Message::Remove { topic_filter, id: id.clone() }.encode()?;
        let raft_mailbox = self.raft_mailbox().await;
        tokio::spawn(async move {
            if let Err(e) = retry(BACKOFF_STRATEGY.clone(), || async {
                let msg = msg.clone();
                let mailbox = raft_mailbox.clone();
                let res = async move { mailbox.send_proposal(msg).await }
                    .spawn(task_exec_queue())
                    .result()
                    .await
                    .map_err(|_| anyhow!("Router::remove(..), task execution failure"))?
                    .map_err(|e| anyhow!(e))?;
                Ok(res)
            })
            .await
            {
                log::warn!("[Router.remove] Failed to send Message::Remove, id: {:?}, {:?}", id, e);
            }
        });
        Ok(true)
    }

    #[inline]
    async fn matches(&self, id: Id, topic: &TopicName) -> Result<SubRelationsMap> {
        self.inner.matches(id, topic).await
    }

    ///Check online or offline
    async fn is_online(&self, node_id: NodeId, client_id: &str) -> bool {
        log::debug!("[Router.is_online] node_id: {:?}, client_id: {:?}", node_id, client_id);
        self.client_states.get(client_id).map(|entry| entry.online).unwrap_or(false)
    }

    #[inline]
    async fn gets(&self, limit: usize) -> Vec<Route> {
        self.inner.gets(limit).await
    }

    #[inline]
    async fn get(&self, topic: &str) -> Result<Vec<Route>> {
        self.inner.get(topic).await
    }

    #[inline]
    async fn query_subscriptions(&self, q: &SubsSearchParams) -> Vec<SubsSearchResult> {
        self.inner.query_subscriptions(q).await
    }

    #[inline]
    async fn topics_tree(&self) -> usize {
        self.inner.topics_tree().await
    }

    #[inline]
    fn topics(&self) -> Counter {
        self.inner.topics()
    }

    #[inline]
    fn routes(&self) -> Counter {
        self.inner.routes()
    }

    #[inline]
    fn merge_topics(&self, topics_map: &HashMap<NodeId, Counter>) -> Counter {
        self.inner.merge_topics(topics_map)
    }

    #[inline]
    fn merge_routes(&self, routes_map: &HashMap<NodeId, Counter>) -> Counter {
        self.inner.merge_routes(routes_map)
    }

    #[inline]
    async fn list_topics(&self, top: usize) -> Vec<String> {
        self.inner.list_topics(top).await
    }

    #[inline]
    async fn list_relations(&self, top: usize) -> Vec<serde_json::Value> {
        self.inner.list_relations(top).await
    }

    #[inline]
    fn relations(&self) -> &AllRelationsMap {
        &self.inner.relations
    }
}

#[async_trait]
impl Store for ClusterRouter {
    async fn apply(&mut self, message: &[u8]) -> RaftResult<Vec<u8>> {
        log::debug!("apply, message.len: {:?}", message.len());
        let message: Message = bincode::deserialize(message).map_err(|e| Error::Other(e))?;
        match message {
            Message::HandshakeTryLock { id } => {
                log::debug!("[Router.HandshakeTryLock] id: {:?}", id);
                let mut try_lock_ok = false;
                let mut prev_id = None;
                self.client_states
                    .entry(id.client_id.clone())
                    .and_modify(|status| {
                        prev_id = Some(status.id.clone());
                        if !status.handshaking(self.try_lock_timeout) {
                            *status = ClientStatus::new(id.clone(), false, true);
                            try_lock_ok = true;
                        }
                    })
                    .or_insert_with(|| {
                        try_lock_ok = true;
                        ClientStatus::new(id.clone(), false, true)
                    });
                log::debug!(
                    "[Router.HandshakeTryLock] id: {:?}, try_lock_ok: {}, prev_id: {:?}",
                    id,
                    try_lock_ok,
                    prev_id
                );
                return if try_lock_ok {
                    Ok(MessageReply::HandshakeTryLock(prev_id).encode().map_err(|_e| Error::Unknown)?)
                } else {
                    Ok(MessageReply::Error("Handshake try lock failed".into())
                        .encode()
                        .map_err(|_e| Error::Unknown)?)
                };
            }
            Message::Connected { id } => {
                log::debug!("[Router.Connected] id: {:?}", id);
                let mut reply = None;
                self.client_states.entry(id.client_id.clone()).and_modify(|status| {
                    if status.id != id && id.create_time < status.id.create_time {
                        log::info!("[Router.Connected] id not the same, input id: {:?}, current status: {:?}", id, status);
                        reply = Some(MessageReply::Error("id not the same".into()));
                    } else {
                        if id.create_time > status.id.create_time {
                            log::info!("[Router.Connected] id.create_time > status.id.create_time, input id: {:?}, current status: {:?}", id, status);
                        }
                        status.id = id.clone();
                        status.online = true;
                        status.handshaking = false;
                    }
                }).or_insert_with(|| {
                    log::debug!("[Router.Connected] id: {:?}, Not found", id);
                    ClientStatus::new(id, true, false)
                });
                if let Some(reply) = reply {
                    return reply.encode().map_err(|_e| Error::Unknown);
                }
            }
            Message::Disconnected { id } => {
                log::debug!("[Router.Disconnected] id: {:?}", id,);
                if let Some(mut entry) = self.client_states.get_mut(&id.client_id) {
                    let status = entry.value_mut();
                    if status.id != id {
                        log::debug!(
                            "[Router.Disconnected] id not the same, input id: {:?}, current status: {:?}",
                            id,
                            status
                        );
                    } else {
                        status.online = false;
                    }
                } else {
                    log::info!("[Router.Disconnected] id: {:?}, Not found", id);
                }
            }
            Message::SessionTerminated { id } => {
                log::debug!("[Router.SessionTerminated] id: {:?}", id,);
                self.client_states.remove_if(&id.client_id, |_, status| {
                    if status.id != id {
                        log::debug!("[Router.SessionTerminated] id not the same, input id: {:?}, current status: {:?}", id, status);
                        false
                    } else {
                        true
                    }
                });
            }
            Message::Add { topic_filter, id, opts } => {
                log::debug!("[Router.add] topic_filter: {:?}, id: {:?}, opts: {:?}", topic_filter, id, opts);
                self.inner.add(topic_filter, id, opts).await?;
            }
            Message::Remove { topic_filter, id } => {
                log::debug!("[Router.remove] topic_filter: {:?}, id: {:?}", topic_filter, id,);
                self.inner.remove(topic_filter, id).await?;
            }
            Message::GetClientNodeId { client_id } => {
                let node_id = self._client_node_id(client_id);
                let data = bincode::serialize(&node_id).map_err(|e| Error::Other(e))?;
                return Ok(data);
            }
            Message::Ping => return MessageReply::Ping.encode().map_err(|_e| Error::Unknown),
        }

        Ok(Vec::new())
    }

    async fn query(&self, query: &[u8]) -> RaftResult<Vec<u8>> {
        log::debug!("query, message.len: {:?}", query.len());
        let query: Message = bincode::deserialize(query).map_err(|e| Error::Other(e))?;
        match query {
            Message::GetClientNodeId { client_id } => {
                let node_id = self._client_node_id(client_id);
                let data = bincode::serialize(&node_id).map_err(|e| Error::Other(e))?;
                return Ok(data);
            }
            _ => {
                log::error!("unimplemented, query: {:?}", query)
            }
        }
        Ok(Vec::new())
    }

    async fn snapshot(&self) -> RaftResult<Vec<u8>> {
        log::debug!("create snapshot ...");
        let now = std::time::Instant::now();
        let relations = &self
            .inner
            .relations
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect::<Vec<_>>();
        let client_states = &self
            .client_states
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect::<Vec<_>>();

        let topics_count = self.inner.topics_count.as_ref();
        let relations_count = self.inner.relations_count.as_ref();

        let snapshot = bincode::serialize(&(relations, client_states, topics_count, relations_count))
            .map_err(|e| Error::Other(e))?;
        log::info!(
            "create snapshot, len: {},  topics_count: {:?}, relations_count: {:?}, cost time: {:?}",
            snapshot.len(),
            topics_count,
            relations_count,
            now.elapsed()
        );

        let now = std::time::Instant::now();
        let compressed = match self.compression {
            Some(Compression::Zstd) => zstd::encode_all(&*snapshot, 1)?,
            Some(Compression::Lz4) => {
                use lz4_flex::block::compress_prepend_size;
                compress_prepend_size(&snapshot)
            }
            Some(Compression::Zlib) => {
                use flate2::write::ZlibEncoder;
                let mut e = ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
                e.write_all(&snapshot)?;
                e.finish()?
            }
            Some(Compression::Snappy) => {
                use snap::write;
                let mut wtr = write::FrameEncoder::new(vec![]);
                wtr.write_all(&snapshot)?;
                wtr.into_inner().map_err(|e| Error::Anyhow(e.into()))?
            }
            None => snapshot,
        };

        if self.compression.is_some() {
            log::info!(
                "create snapshot, compressed({:?}) len: {}, cost time: {:?}",
                self.compression,
                compressed.len(),
                now.elapsed()
            );
        }

        Ok(compressed)
    }

    async fn restore(&mut self, snapshot: &[u8]) -> RaftResult<()> {
        log::info!("restore, snapshot.len: {}", snapshot.len());
        let now = std::time::Instant::now();

        let uncompressed = match self.compression {
            Some(Compression::Zstd) => Cow::Owned(zstd::decode_all(snapshot)?),
            Some(Compression::Lz4) => {
                use lz4_flex::block::decompress_size_prepended;
                let buf = decompress_size_prepended(snapshot).map_err(|e| Error::Other(Box::new(e)))?;
                Cow::Owned(buf)
            }
            Some(Compression::Zlib) => {
                use flate2::bufread::ZlibDecoder;
                let mut z = ZlibDecoder::new(snapshot);
                let mut buf = Vec::new();
                z.read_to_end(&mut buf)?;
                Cow::Owned(buf)
            }
            Some(Compression::Snappy) => {
                let mut buf = Vec::new();
                snap::read::FrameDecoder::new(snapshot).read_to_end(&mut buf)?;
                Cow::Owned(buf)
            }
            None => Cow::Borrowed(snapshot),
        };

        if self.compression.is_some() {
            log::info!(
                "restore, uncompressed({:?}) len: {}, cost time: {:?}",
                self.compression,
                uncompressed.len(),
                now.elapsed()
            );
        }

        let now = std::time::Instant::now();
        let (relations, client_states, topics_count, relations_count): (
            Vec<(TopicFilter, HashMap<ClientId, (Id, SubscriptionOptions)>)>,
            Vec<(ClientId, ClientStatus)>,
            Counter,
            Counter,
        ) = bincode::deserialize(uncompressed.as_ref()).map_err(|e| Error::Other(e))?;

        self.inner.topics_count.set(&topics_count);

        let mut topics = self.inner.topics.write().await;
        self.inner.relations.clear();
        for (topic_filter, relation) in relations {
            let topic = Topic::from_str(&topic_filter).map_err(|e| Error::Msg(format!("{:?}", e)))?;
            self.inner.relations.insert(topic_filter, relation);
            topics.insert(&topic, ());
        }
        self.inner.relations_count.set(&relations_count);

        self.client_states.clear();
        for (client_id, content) in client_states {
            self.client_states.insert(client_id, content);
        }

        log::info!(
            "restore, topics_count: {:?}, relations_count: {:?}, cost time: {:?}",
            topics_count,
            relations_count,
            now.elapsed()
        );
        Ok(())
    }
}
