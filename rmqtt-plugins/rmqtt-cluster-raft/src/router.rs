use std::sync::Arc;
use std::time::Duration;

use once_cell::sync::OnceCell;
use rmqtt_raft::{Error, Mailbox, Result as RaftResult, Store};
use tokio::sync::RwLock;

use rmqtt::rust_box::task_exec_queue::SpawnExt;
use rmqtt::stats::Counter;
use rmqtt::{
    ahash, anyhow, async_trait::async_trait, bincode, chrono, dashmap, log, once_cell, serde_json, tokio,
    MqttError,
};
use rmqtt::{
    broker::{
        default::DefaultRouter,
        topic::TopicTree,
        types::{
            ClientId, Id, IsOnline, NodeId, QoS, Route, SharedGroup, TimestampMillis, TopicFilter, TopicName,
        },
        Router, SubRelationsMap,
    },
    Result,
};

use crate::task_exec_queue;

use super::config::{retry, BACKOFF_STRATEGY};
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
        Self { id, online, handshaking, handshak_duration: chrono::Local::now().timestamp_millis() }
    }

    pub fn handshaking(&self, try_lock_timeout: Duration) -> bool {
        self.handshaking
            && (chrono::Local::now().timestamp_millis()
                < (self.handshak_duration + try_lock_timeout.as_millis() as TimestampMillis))
    }
}

pub(crate) struct ClusterRouter {
    inner: &'static DefaultRouter,
    raft_mailbox: Arc<RwLock<Option<Mailbox>>>,
    client_states: DashMap<ClientId, ClientStatus>,
    pub try_lock_timeout: Duration,
}

impl ClusterRouter {
    #[inline]
    pub(crate) fn get_or_init(try_lock_timeout: Duration) -> &'static Self {
        static INSTANCE: OnceCell<ClusterRouter> = OnceCell::new();
        INSTANCE.get_or_init(|| Self {
            inner: DefaultRouter::instance(),
            raft_mailbox: Arc::new(RwLock::new(None)),
            client_states: DashMap::default(),
            try_lock_timeout,
        })
    }

    #[inline]
    pub(crate) fn _inner(&self) -> Box<dyn Router> {
        Box::new(self.inner)
    }

    #[inline]
    pub(crate) async fn set_raft_mailbox(&self, raft_mailbox: Mailbox) {
        self.raft_mailbox.write().await.replace(raft_mailbox);
    }

    #[inline]
    pub(crate) async fn raft_mailbox(&self) -> Mailbox {
        self.raft_mailbox.read().await.as_ref().unwrap().clone()
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
    pub(crate) fn remove_client_status(&self, client_id: &str) {
        self.client_states.remove(client_id);
    }

    #[inline]
    pub(crate) fn _handshakings(&self) -> usize {
        self.client_states.iter().filter_map(|entry| if entry.handshaking { Some(()) } else { None }).count()
    }
}

#[async_trait]
impl Router for &'static ClusterRouter {
    #[inline]
    async fn add(
        &self,
        topic_filter: &str,
        id: Id,
        qos: QoS,
        shared_group: Option<SharedGroup>,
    ) -> Result<()> {
        log::debug!(
            "[Router.add] topic_filter: {:?}, id: {:?}, qos: {:?}, shared_group: {:?}",
            topic_filter,
            id,
            qos,
            shared_group
        );

        let msg = Message::Add { topic_filter, id, qos, shared_group }.encode()?;
        let mailbox = self.raft_mailbox().await;
        let _ = async move { mailbox.send(msg).await.map_err(anyhow::Error::new) }
            .spawn(task_exec_queue())
            .result()
            .await
            .map_err(|_| MqttError::from("Router::add(..), task execution failure"))??;
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
                let res = async move { mailbox.send(msg).await }
                    .spawn(task_exec_queue())
                    .result()
                    .await
                    .map_err(|_| MqttError::from("Router::remove(..), task execution failure"))?
                    .map_err(|e| MqttError::from(e.to_string()))?;
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
    async fn matches(&self, topic: &TopicName) -> Result<SubRelationsMap> {
        self.inner.matches(topic).await
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
}

#[async_trait]
impl Store for &'static ClusterRouter {
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
                    let mut status = entry.value_mut();
                    if status.id != id {
                        log::info!(
                            "[Router.Disconnected] id not the same, input id: {:?}, current status: {:?}",
                            id,
                            status
                        );
                    } else {
                        status.online = false;
                    }
                } else {
                    log::warn!("[Router.Disconnected] id: {:?}, Not found", id);
                }
            }
            Message::SessionTerminated { id } => {
                log::debug!("[Router.SessionTerminated] id: {:?}", id,);
                self.client_states.remove_if(&id.client_id, |_, status| {
                    if status.id != id {
                        log::info!("[Router.SessionTerminated] id not the same, input id: {:?}, current status: {:?}", id, status);
                        false
                    } else {
                        true
                    }
                });
            }
            Message::Add { topic_filter, id, qos, shared_group } => {
                log::debug!(
                    "[Router.add] topic_filter: {:?}, id: {:?}, qos: {:?}, shared_group: {:?}",
                    topic_filter,
                    id,
                    qos,
                    shared_group
                );
                self.inner
                    .add(topic_filter, id, qos, shared_group)
                    .await
                    .map_err(|e| Error::Other(Box::new(e)))?;
            }
            Message::Remove { topic_filter, id } => {
                log::debug!("[Router.remove] topic_filter: {:?}, id: {:?}", topic_filter, id,);
                self.inner.remove(topic_filter, id).await.map_err(|e| Error::Other(Box::new(e)))?;
            }
            Message::GetClientNodeId { client_id } => {
                let node_id = self._client_node_id(client_id);
                let data = bincode::serialize(&node_id).map_err(|e| Error::Other(e))?;
                return Ok(data);
            }
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

        let topics_count = &self.inner.topics_count;
        let relations_count = &self.inner.relations_count;

        let snapshot = bincode::serialize(&(
            self.inner.topics.read().await.as_ref(),
            relations,
            client_states,
            topics_count,
            relations_count,
        ))
        .map_err(|e| Error::Other(e))?;
        log::info!("create snapshot, len: {}", snapshot.len());
        Ok(snapshot)
    }

    async fn restore(&mut self, snapshot: &[u8]) -> RaftResult<()> {
        log::info!("restore, snapshot.len: {}", snapshot.len());

        let (topics, relations, client_states, topics_count, relations_count): (
            TopicTree<()>,
            Vec<(TopicFilter, HashMap<ClientId, (Id, QoS, Option<SharedGroup>)>)>,
            Vec<(ClientId, ClientStatus)>,
            Counter,
            Counter,
        ) = bincode::deserialize(snapshot).map_err(|e| Error::Other(e))?;

        *self.inner.topics.write().await = topics;
        self.inner.topics_count.set(&topics_count);

        self.inner.relations.clear();
        for (topic_filter, relation) in relations {
            self.inner.relations.insert(topic_filter, relation);
        }
        self.inner.relations_count.set(&relations_count);

        self.client_states.clear();
        for (client_id, content) in client_states {
            self.client_states.insert(client_id, content);
        }

        Ok(())
    }
}
