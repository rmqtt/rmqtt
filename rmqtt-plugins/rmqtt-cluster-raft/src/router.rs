use std::sync::Arc;

use once_cell::sync::OnceCell;
use rmqtt_raft::{Error, Mailbox, Result as RaftResult, Store};
use tokio::sync::RwLock;

use rmqtt::{
    broker::{
        default::DefaultRouter,
        IsOnline,
        Router,
        SubRelationsMap, topic::TopicTree, types::{ClientId, Id, NodeId, QoS, SharedGroup, TopicFilter, TopicName},
    },
    Result,
};

use super::message::Message;

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;
type DashMap<K, V> = dashmap::DashMap<K, V, ahash::RandomState>;

pub struct ClusterRouter {
    inner: &'static DefaultRouter,
    raft_mailbox: Arc<RwLock<Option<Mailbox>>>,
    client_statuses: DashMap<ClientId, (Id, IsOnline)>,
}

impl ClusterRouter {
    #[inline]
    pub(crate) fn get_or_init() -> &'static Self {
        static INSTANCE: OnceCell<ClusterRouter> = OnceCell::new();
        INSTANCE.get_or_init(|| Self {
            inner: DefaultRouter::instance(),
            raft_mailbox: Arc::new(RwLock::new(None)),
            client_statuses: DashMap::default(),
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
        self.client_statuses.get(client_id).map(|entry| {
            let (id, _) = entry.value();
            id.node_id
        })
    }

    #[inline]
    pub(crate) fn id(&self, client_id: &str) -> Option<Id> {
        self.client_statuses.get(client_id).map(|entry| {
            let (id, _) = entry.value();
            id.clone()
        })
    }

    #[inline]
    pub(crate) fn all_onlines(&self) -> usize {
        self.client_statuses
            .iter()
            .filter_map(|entry| {
                let (_id, online) = entry.value();
                if *online {
                    Some(())
                } else {
                    None
                }
            })
            .count()
    }

    #[inline]
    pub(crate) fn all_statuses(&self) -> usize {
        self.client_statuses.len()
    }
}

#[async_trait]
impl Router for &'static ClusterRouter {
    #[inline]
    async fn add(
        &self,
        topic_filter: &str,
        node_id: NodeId,
        client_id: &str,
        qos: QoS,
        shared_group: Option<SharedGroup>,
    ) -> Result<()> {
        log::debug!(
            "[Router.add] topic_filter: {:?}, node_id: {:?}, client_id: {:?}, qos: {:?}, shared_group: {:?}",
            topic_filter,
            node_id,
            client_id,
            qos,
            shared_group
        );

        let msg = Message::Add { topic_filter, node_id, client_id, qos, shared_group }.encode()?;
        let _ = self.raft_mailbox().await.send(msg).await.map_err(anyhow::Error::new)?;

        Ok(())
    }

    #[inline]
    async fn remove(&self, topic_filter: &str, node_id: NodeId, client_id: &str) -> Result<()> {
        log::debug!(
            "[Router.remove] topic_filter: {:?}, node_id: {:?}, client_id: {:?}",
            topic_filter,
            node_id,
            client_id
        );

        let msg = Message::Remove { topic_filter, node_id, client_id }.encode()?;
        let _ = self.raft_mailbox().await.send(msg).await.map_err(anyhow::Error::new)?;

        Ok(())
    }

    #[inline]
    async fn matches(&self, topic: &TopicName) -> Result<SubRelationsMap> {
        self.inner.matches(topic).await
    }

    ///Check online or offline
    async fn is_online(&self, node_id: NodeId, client_id: &str) -> bool {
        log::debug!("[Router.is_online] node_id: {:?}, client_id: {:?}", node_id, client_id);
        self.client_statuses
            .get(client_id)
            .map(|entry| {
                let (_, online) = entry.value();
                *online
            })
            .unwrap_or(false)
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
            Message::Connected { id } => {
                log::debug!("[Router.Connected] id: {:?}", id);
                self.client_statuses.insert(id.client_id.clone(), (id, true));
            }
            Message::Disconnected { id } => {
                log::debug!("[Router.Disconnected] id: {:?}", id,);
                if let Some(mut entry) = self.client_statuses.get_mut(&id.client_id) {
                    let (c_id, online) = entry.value_mut();
                    if *c_id != id {
                        log::warn!("[Router.Disconnected] id not the same, input id: {:?}, current id: {:?}", id, c_id);
                    } else {
                        *online = false;
                    }
                } else {
                    log::warn!("[Router.Disconnected] id: {:?}, Not found", id);
                }
            }
            Message::SessionTerminated { id } => {
                log::debug!("[Router.SessionTerminated] id: {:?}", id,);
                self.client_statuses.remove_if(&id.client_id, |_, (c_id, online)| {
                    if *c_id != id {
                        log::warn!("[Router.SessionTerminated] id not the same, input id: {:?}, current id: {:?}, online: {}", id, c_id, online);
                        false
                    } else {
                        true
                    }
                });
            }
            Message::Add { topic_filter, node_id, client_id, qos, shared_group } => {
                log::debug!(
                    "[Router.add] topic_filter: {:?}, node_id: {:?}, client_id: {:?}, qos: {:?}, shared_group: {:?}",
                    topic_filter,
                    node_id,
                    client_id,
                    qos,
                    shared_group
                );
                self.inner
                    .add(topic_filter, node_id, client_id, qos, shared_group)
                    .await
                    .map_err(|e| Error::Other(Box::new(e)))?;
            }
            Message::Remove { topic_filter, node_id, client_id } => {
                log::debug!(
                    "[Router.remove] topic_filter: {:?}, node_id: {:?}, client_id: {:?}",
                    topic_filter,
                    node_id,
                    client_id,
                );
                self.inner
                    .remove(topic_filter, node_id, client_id)
                    .await
                    .map_err(|e| Error::Other(Box::new(e)))?;
            }
            Message::GetClientNodeId { client_id } => {
                //unimplemented!("Only query operations are supported")
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
        let client_statuses = &self
            .client_statuses
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect::<Vec<_>>();

        let snapshot =
            bincode::serialize(&(self.inner.topics.read().await.as_ref(), relations, client_statuses))
                .map_err(|e| Error::Other(e))?;
        log::debug!("snapshot len: {}", snapshot.len());
        Ok(snapshot)
    }

    async fn restore(&mut self, snapshot: &[u8]) -> RaftResult<()> {
        log::info!("restore, snapshot.len: {}", snapshot.len());

        let (topics, relations, client_statuses): (
            TopicTree<()>,
            Vec<(TopicFilter, HashMap<NodeId, HashMap<ClientId, (QoS, Option<SharedGroup>)>>)>,
            Vec<(ClientId, (Id, IsOnline))>,
        ) = bincode::deserialize(snapshot).map_err(|e| Error::Other(e))?;

        *self.inner.topics.write().await = topics;

        self.inner.relations.clear();
        for (topic_filter, relation) in relations {
            self.inner.relations.insert(topic_filter, relation);
        }

        self.client_statuses.clear();
        for (client_id, content) in client_statuses {
            self.client_statuses.insert(client_id, content);
        }

        Ok(())
    }
}
