use std::time::Duration;

use futures::future::FutureExt;
use once_cell::sync::OnceCell;

use rmqtt::{
    broker::{
        default::DefaultShared,
        Entry,
        session::{ClientInfo, Session, SessionOfflineInfo},
        Shared, SubRelations, SubRelationsMap, types::{From, Id, Publish, Reason, Subscribe, SubscribeReturn, To, Tx, Unsubscribe},
    },
    grpc::{Message, MessageReply, MessageType},
    Result, Runtime,
};

use super::{ClusterRouter, GrpcClients, MessageSender};
use super::message::Message as RaftMessage;

pub struct ClusterLockEntry {
    inner: Box<dyn Entry>,
    cluster_shared: &'static ClusterShared,
}

impl ClusterLockEntry {
    #[inline]
    pub fn new(inner: Box<dyn Entry>, cluster_shared: &'static ClusterShared) -> Self {
        Self { inner, cluster_shared }
    }
}

#[async_trait]
impl Entry for ClusterLockEntry {
    #[inline]
    fn try_lock(&self) -> Result<Box<dyn Entry>> {
        Ok(Box::new(ClusterLockEntry::new(self.inner.try_lock()?, self.cluster_shared)))
    }

    #[inline]
    fn id(&self) -> Id {
        self.inner.id()
    }

    #[inline]
    async fn set(&mut self, session: Session, tx: Tx, conn: ClientInfo) -> Result<()> {
        let msg = RaftMessage::Connected { id: session.id.clone() }
            .encode()?;
        let raft_mailbox = self.cluster_shared.router.raft_mailbox().await;
        raft_mailbox.send(msg).await.map_err(anyhow::Error::new)?;
        self.inner.set(session, tx, conn).await
    }

    #[inline]
    async fn remove(&mut self) -> Result<Option<(Session, Tx, ClientInfo)>> {
        self.inner.remove().await
    }

    #[inline]
    async fn kick(&mut self, clear_subscriptions: bool) -> Result<Option<SessionOfflineInfo>> {
        log::debug!(
            "{:?} ClusterLockEntry kick ..., clear_subscriptions: {}",
            self.client().await.map(|c| c.id.clone()),
            clear_subscriptions
        );
        let id = self.id();

        let raft_mailbox = self.cluster_shared.router.raft_mailbox().await;
        let reply = raft_mailbox
            .query(RaftMessage::GetClientNodeId { client_id: &id.client_id }.encode()?)  //query
            .await
            .map_err(anyhow::Error::new)?;

        log::debug!("{:?} kick, GetClientNodeId: reply: {:?}", id, reply);

        let prev_node_id = match bincode::deserialize(reply.as_ref()).map_err(anyhow::Error::new)? {
            Some(prev_node_id) => prev_node_id,
            None => id.node_id,
        };

        if prev_node_id == id.node_id {
            //kicked from local
            self.inner.kick(clear_subscriptions).await
        } else {
            //kicked from other node
            if let Some(client) = self.cluster_shared.grpc_clients.get(&prev_node_id) {
                let mut msg_sender = MessageSender {
                    client: client.value().clone(),
                    msg_type: self.cluster_shared.message_type,
                    msg: Message::Kick(id.clone(), true), //clear_subscriptions
                    max_retries: 3,
                    retry_interval: Duration::from_millis(500),
                };
                match msg_sender.send().await {
                    Ok(reply) => {
                        if let MessageReply::Kick(Some(kicked)) = reply {
                            log::debug!("{:?} kicked: {:?}", id, kicked);
                            Ok(Some(kicked))
                        } else {
                            log::error!(
                                "{:?} Message::Kick from other node, prev_node_id: {:?}, reply: {:?}",
                                id,
                                prev_node_id,
                                reply
                            );
                            Ok(None)
                        }
                    }
                    Err(e) => {
                        log::error!(
                            "{:?} Message::Kick from other node, prev_node_id: {:?}, error: {:?}",
                            id,
                            prev_node_id,
                            e
                        );
                        Ok(None)
                    }
                }
            } else {
                log::error!(
                    "{:?} kick error, grpc_client is not exist, prev_node_id: {:?}",
                    id,
                    prev_node_id,
                );
                Ok(None)
            }
        }
    }

    #[inline]
    async fn is_connected(&self) -> bool {
        self.inner.is_connected().await
    }

    #[inline]
    async fn session(&self) -> Option<Session> {
        self.inner.session().await
    }

    #[inline]
    async fn client(&self) -> Option<ClientInfo> {
        self.inner.client().await
    }

    #[inline]
    fn tx(&self) -> Option<Tx> {
        self.inner.tx()
    }

    #[inline]
    async fn subscribe(&self, subscribe: Subscribe) -> Result<SubscribeReturn> {
        self.inner.subscribe(subscribe).await
    }

    #[inline]
    async fn unsubscribe(&self, unsubscribe: &Unsubscribe) -> Result<()> {
        self.inner.unsubscribe(unsubscribe).await
    }

    #[inline]
    async fn publish(&self, from: From, p: Publish) -> Result<(), (From, Publish, Reason)> {
        self.inner.publish(from, p).await
    }
}

pub struct ClusterShared {
    inner: &'static DefaultShared,
    router: &'static ClusterRouter,
    grpc_clients: GrpcClients,
    pub message_type: MessageType,
}

impl ClusterShared {
    #[inline]
    pub(crate) fn get_or_init(
        router: &'static ClusterRouter,
        grpc_clients: GrpcClients,
        message_type: MessageType,
    ) -> &'static ClusterShared {
        static INSTANCE: OnceCell<ClusterShared> = OnceCell::new();
        INSTANCE.get_or_init(|| Self { inner: DefaultShared::instance(), router, grpc_clients, message_type })
    }

    #[inline]
    pub(crate) fn inner(&self) -> Box<dyn Shared> {
        Box::new(self.inner)
    }
}

#[async_trait]
impl Shared for &'static ClusterShared {
    #[inline]
    fn entry(&self, id: Id) -> Box<dyn Entry> {
        Box::new(ClusterLockEntry::new(self.inner.entry(id), self))
    }

    #[inline]
    fn id(&self, client_id: &str) -> Option<Id> {
        self.router.id(client_id)
    }

    #[inline]
    async fn forwards(&self, from: From, publish: Publish) -> Result<(), Vec<(To, From, Publish, Reason)>> {
        log::debug!("[forwards] from: {:?}, publish: {:?}", from, publish);

        let topic = publish.topic();
        let mut relations_map =
            match Runtime::instance().extends.router().await.matches(publish.topic()).await {
                Ok(relations_map) => relations_map,
                Err(e) => {
                    log::warn!("forwards, from:{:?}, topic:{:?}, error: {:?}", from, topic, e);
                    SubRelationsMap::default()
                }
            };

        let mut errs = Vec::new();

        let this_node_id = Runtime::instance().node.id();
        if let Some(relations) = relations_map.remove(&this_node_id) {
            //forwards to local
            if let Err(e) = self.forwards_to(from.clone(), &publish, relations).await {
                errs.extend(e);
            }
        }
        if !relations_map.is_empty() {
            log::debug!("forwards to other nodes, relations_map:{:?}", relations_map);
            //forwards to other nodes
            let mut fut_senders = Vec::new();
            for (node_id, relations) in relations_map {
                if let Some(client) = self.grpc_clients.get(&node_id) {
                    let from = from.clone();
                    let publish = publish.clone();
                    let message_type = self.message_type;
                    let fut_sender = async move {
                        let mut msg_sender = MessageSender {
                            client: client.value().clone(),
                            msg_type: message_type,
                            msg: Message::ForwardsTo(from, publish, relations),
                            max_retries: 3,
                            retry_interval: Duration::from_millis(500),
                        };
                        (node_id, msg_sender.send().await)
                    };
                    fut_senders.push(fut_sender.boxed());
                } else {
                    log::error!(
                        "forwards error, grpc_client is not exist, node_id: {}, relations: {:?}",
                        node_id,
                        relations
                    );
                }
            }

            tokio::spawn(async move {
                let replys = futures::future::join_all(fut_senders).await;
                for (node_id, reply) in replys {
                    if let Err(e) = reply {
                        log::error!(
                            "forwards Message::ForwardsTo to other node, from: {:?}, to: {:?}, error: {:?}",
                            from,
                            node_id,
                            e
                        );
                    }
                }
            });
        }

        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs)
        }
    }

    #[inline]
    async fn forwards_to(
        &self,
        from: From,
        publish: &Publish,
        relations: SubRelations,
    ) -> Result<(), Vec<(To, From, Publish, Reason)>> {
        self.inner.forwards_to(from, publish, relations).await
    }

    #[inline]
    async fn forwards_and_get_shareds(
        &self,
        from: From,
        publish: Publish,
    ) -> Result<SubRelationsMap, Vec<(To, From, Publish, Reason)>> {
        self.inner.forwards_and_get_shareds(from, publish).await
    }

    #[inline]
    async fn clients(&self) -> usize {
        self.inner.clients().await
    }

    #[inline]
    async fn sessions(&self) -> usize {
        self.inner.sessions().await
    }

    #[inline]
    async fn all_clients(&self) -> usize {
        self.router.all_onlines()
    }

    #[inline]
    async fn all_sessions(&self) -> usize {
        self.router.all_statuses()
    }

    #[inline]
    fn iter(&self) -> Box<dyn Iterator<Item=Box<dyn Entry>> + Sync + Send> {
        self.inner.iter()
    }

    #[inline]
    fn random_session(&self) -> Option<(Session, ClientInfo)> {
        self.inner.random_session()
    }
}
