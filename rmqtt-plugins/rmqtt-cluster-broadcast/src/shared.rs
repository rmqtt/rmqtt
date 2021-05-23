use once_cell::sync::OnceCell;

use rmqtt::{
    broker::{
        default::DefaultShared,
        session::{ClientInfo, Session, SessionOfflineInfo},
        types::{From, Id, Publish, Reason, Subscribe, SubscribeAck, To, Tx, Unsubscribe, UnsubscribeAck},
        Entry, Shared,
    },
    grpc::{Message, MessageReply, MessageType},
    Runtime,
};

use super::{GrpcClients, MessageBroadcaster};

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
    fn try_lock(&self) -> anyhow::Result<Box<dyn Entry>> {
        Ok(Box::new(ClusterLockEntry::new(self.inner.try_lock()?, self.cluster_shared)))
    }

    #[inline]
    fn id(&self) -> Id {
        self.inner.id()
    }

    #[inline]
    async fn set(&mut self, session: Session, tx: Tx, conn: ClientInfo) -> anyhow::Result<()> {
        self.inner.set(session, tx, conn).await
    }

    #[inline]
    async fn remove(&mut self) -> anyhow::Result<Option<(Session, Tx, ClientInfo)>> {
        self.inner.remove().await
    }

    #[inline]
    async fn kick(&mut self, clear_subscriptions: bool) -> anyhow::Result<Option<SessionOfflineInfo>> {
        log::debug!("{:?} ClusterLockEntry kick 1 ...", self.client().await.map(|c| c.id.clone()));
        if let Some(kicked) = self.inner.kick(clear_subscriptions).await? {
            log::debug!("{:?} broadcast kick reply kicked: {:?}", self.id(), kicked);
            return Ok(Some(kicked));
        }

        match MessageBroadcaster::new(
            self.cluster_shared.grpc_clients.clone(),
            self.cluster_shared.message_type,
            Message::Kick(self.id()),
        )
        .kick()
        .await
        {
            Ok(kicked) => {
                log::debug!("{:?} broadcast kick reply kicked: {:?}", self.id(), kicked);
                log::debug!(
                    "{:?} clear_subscriptions: {}, {}, {}",
                    self.id(),
                    clear_subscriptions,
                    kicked.subscriptions.is_empty(),
                    !clear_subscriptions && !kicked.subscriptions.is_empty()
                );
                if !clear_subscriptions && !kicked.subscriptions.is_empty() {
                    let router = Runtime::instance().extends.router().await;
                    let node_id = Runtime::instance().node.id();
                    let client_id = &self.id().client_id;
                    for (tf, qos) in kicked.subscriptions.iter() {
                        router.add(tf, node_id, client_id, *qos).await?;
                    }
                }
                Ok(Some(kicked))
            }
            Err(e) => {
                log::debug!("{:?}, broadcast Message::Kick reply: {:?}", self.id(), e);
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
    async fn subscribe(&self, subscribe: Subscribe) -> anyhow::Result<SubscribeAck> {
        self.inner.subscribe(subscribe).await
    }

    #[inline]
    async fn unsubscribe(&self, unsubscribe: &Unsubscribe) -> anyhow::Result<UnsubscribeAck> {
        self.inner.unsubscribe(unsubscribe).await
    }

    #[inline]
    async fn publish(&self, from: From, p: Publish) -> anyhow::Result<(), (From, Publish, Reason)> {
        self.inner.publish(from, p).await
    }
}

pub struct ClusterShared {
    inner: &'static DefaultShared,
    grpc_clients: GrpcClients,
    pub message_type: MessageType,
}

impl ClusterShared {
    #[inline]
    pub(crate) fn get_or_init(
        grpc_clients: GrpcClients,
        message_type: MessageType,
    ) -> &'static ClusterShared {
        static INSTANCE: OnceCell<ClusterShared> = OnceCell::new();
        INSTANCE.get_or_init(|| Self { inner: DefaultShared::instance(), grpc_clients, message_type })
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
    async fn forwards(
        &self,
        from: From,
        publish: Publish,
    ) -> anyhow::Result<(), Vec<(To, From, Publish, Reason)>> {
        self.inner.forwards(from, publish).await
    }

    #[inline]
    async fn clients(&self) -> usize {
        let mut clients = self.inner.clients().await;

        let replys =
            MessageBroadcaster::new(self.grpc_clients.clone(), self.message_type, Message::NumberOfClients)
                .join_all()
                .await;

        for reply in replys {
            match reply {
                Ok(reply) => {
                    if let MessageReply::NumberOfClients(n) = reply {
                        clients += n
                    }
                }
                Err(e) => {
                    log::error!("Get Message::NumberOfClients from other node, error: {:?}", e);
                }
            }
        }

        clients
    }

    #[inline]
    async fn sessions(&self) -> usize {
        let mut sessions = self.inner.sessions().await;

        let replys =
            MessageBroadcaster::new(self.grpc_clients.clone(), self.message_type, Message::NumberOfSessions)
                .join_all()
                .await;

        for reply in replys {
            match reply {
                Ok(reply) => {
                    if let MessageReply::NumberOfSessions(n) = reply {
                        sessions += n
                    }
                }
                Err(e) => {
                    log::error!("Get Message::NumberOfSessions from other node, error: {:?}", e);
                }
            }
        }

        sessions
    }

    #[inline]
    fn iter(&self) -> Box<dyn Iterator<Item = Box<dyn Entry>> + Sync + Send> {
        self.inner.iter()
    }

    #[inline]
    fn random_session(&self) -> Option<(Session, ClientInfo)> {
        self.inner.random_session()
    }
}
