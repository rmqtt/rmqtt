use once_cell::sync::OnceCell;
use std::convert::From as _f;
type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

use rmqtt::{
    broker::{
        default::DefaultShared,
        session::{ClientInfo, Session, SessionOfflineInfo},
        types::{
            ClientId, From, Id, NodeId, Publish, QoS, Reason, SharedGroup, Subscribe, SubscribeReturn, To,
            TopicFilter, TopicFilterString, Tx, Unsubscribe,
        },
        Entry, IsOnline, Shared, SharedSubRelations, SubRelations,
    },
    grpc::{Message, MessageReply, MessageType},
    Result, Runtime,
};

use super::{hook_message_dropped, GrpcClients, MessageBroadcaster};

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
        self.inner.set(session, tx, conn).await
    }

    #[inline]
    async fn remove(&mut self) -> Result<Option<(Session, Tx, ClientInfo)>> {
        self.inner.remove().await
    }

    #[inline]
    async fn kick(&mut self, clear_subscriptions: bool) -> Result<Option<SessionOfflineInfo>> {
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
                    for (tf, (qos, shared_group)) in kicked.subscriptions.iter() {
                        router.add(tf, node_id, client_id, *qos, shared_group.clone()).await?;
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
    async fn forwards(&self, from: From, publish: Publish) -> Result<(), Vec<(To, From, Publish, Reason)>> {
        let this_node_id = Runtime::instance().node.id();
        let topic = publish.topic();
        log::debug!("forwards, from: {:?}, topic: {:?}", from, topic.to_string());
        let (relations, shared_relations, _other_relations) =
            match Runtime::instance().extends.router().await.matches(topic).await {
                Ok((relations, shared_relations, _other_relations)) => {
                    (relations, shared_relations, _other_relations)
                }
                Err(e) => {
                    log::warn!("forwards, from:{:?}, topic:{:?}, error: {:?}", from, topic, e);
                    (Vec::new(), HashMap::default(), HashMap::default())
                }
            };

        let local_res = self.inner.forwards_to(from.clone(), &publish, relations).await;
        log::debug!("forwards, from: {:?}, local_res: {:?}", from, local_res);

        let grpc_clients = self.grpc_clients.clone();
        let message_type = self.message_type;
        let inner = self.inner;
        let broadcast_fut = async move {
            //forwards to other node and get shared subscription relations
            let replys = MessageBroadcaster::new(
                grpc_clients.clone(),
                message_type,
                Message::Forwards(from.clone(), publish.clone()),
            )
            .join_all()
            .await;

            let mut shared_sub_groups: HashMap<
                TopicFilterString, //key is TopicFilter
                HashMap<SharedGroup, Vec<(NodeId, ClientId, QoS, Option<IsOnline>)>>,
            > = HashMap::default();
            let mut add_to_shared_sub_groups = |mut shared_subs: SharedSubRelations| {
                for (topic_filter, subs) in shared_subs.drain() {
                    let groups = shared_sub_groups.entry(topic_filter).or_default();
                    for (group, node_id, client_id, qos, is_online) in subs {
                        groups.entry(group).or_default().push((node_id, client_id, qos, Some(is_online)));
                    }
                }
            };

            add_to_shared_sub_groups(shared_relations);

            for reply in replys {
                match reply {
                    Ok(reply) => {
                        if let MessageReply::Forwards(o_shared_subs) = reply {
                            log::debug!("o_shared_subs: {:?}", o_shared_subs);
                            add_to_shared_sub_groups(o_shared_subs);
                        }
                    }
                    Err(e) => {
                        log::error!(
                            "forwards Message::Forwards to other node, from: {:?}, error: {:?}",
                            from,
                            e
                        );
                    }
                }
            }

            //shared_sub_groups
            log::debug!("shared_sub_groups: {:?}", shared_sub_groups);
            let mut node_shared_subs: HashMap<NodeId, SubRelations> = HashMap::default();
            for (topic_filter, sub_groups) in shared_sub_groups.iter_mut() {
                for (_group, subs) in sub_groups.iter_mut() {
                    if let Some((idx, _is_online)) =
                        Runtime::instance().extends.shared_subscription().await.choice(subs).await
                    {
                        let (node_id, client_id, qos, _) = subs.remove(idx);
                        node_shared_subs.entry(node_id).or_default().push((
                            TopicFilter::from(topic_filter.clone()),
                            client_id,
                            qos,
                        ));
                    }
                }
            }
            log::debug!("node_shared_subs: {:?}", node_shared_subs);

            //send to this node
            if let Some(sub_rels) = node_shared_subs.remove(&this_node_id) {
                if let Err(droppeds) = inner.forwards_to(from.clone(), &publish, sub_rels).await {
                    hook_message_dropped(droppeds).await;
                }
            }

            //send to other node
            let mut delivers = Vec::new();
            for (node_addr, grpc_client) in grpc_clients.iter() {
                if let Some(sub_rels) = node_shared_subs.remove(&node_addr.id) {
                    delivers.push(grpc_client.send_message(
                        message_type,
                        Message::ForwardsTo(from.clone(), publish.clone(), sub_rels),
                    ));
                }
            }
            if !delivers.is_empty() {
                //tokio::spawn(async move {
                let ress = futures::future::join_all(delivers).await;
                for res in ress {
                    if let Err(e) = res {
                        log::error!("deliver shared subscriptions error, {:?}", e);
                    }
                }
                //});
            }
        };

        tokio::spawn(broadcast_fut);

        local_res?;
        Ok(())
    }

    async fn forwards_and_get_shareds(
        &self,
        from: From,
        publish: Publish,
    ) -> Result<SharedSubRelations, Vec<(To, From, Publish, Reason)>> {
        self.inner.forwards_and_get_shareds(from, publish).await
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
    async fn clients(&self) -> usize {
        self.inner.clients().await
    }

    #[inline]
    async fn sessions(&self) -> usize {
        self.inner.sessions().await
    }

    #[inline]
    async fn all_clients(&self) -> usize {
        let mut clients = self.inner.all_clients().await;

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
    async fn all_sessions(&self) -> usize {
        let mut sessions = self.inner.all_sessions().await;

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
