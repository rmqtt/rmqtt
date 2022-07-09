use std::convert::From as _f;

use once_cell::sync::OnceCell;

use rmqtt::{
    AsStr,
    broker::{
        default::DefaultShared,
        Entry, Shared, SubRelations, SubRelationsMap,
        session::{ClientInfo, Session, SessionOfflineInfo},
        types::{
            ClientId, From, Id, IsOnline, NodeId, Publish, QoS, Reason, SessionStatus, SharedGroup, Subscribe,
            SubscribeReturn, To, TopicFilter, TopicFilterString, Tx, Unsubscribe, IsAdmin
        },
    },
    grpc::{GrpcClients, Message, MessageReply, MessageType}, Result, Runtime,
};

use super::{hook_message_dropped, MessageBroadcaster};

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

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
    async fn try_lock(&self) -> Result<Box<dyn Entry>> {
        Ok(Box::new(ClusterLockEntry::new(self.inner.try_lock().await?, self.cluster_shared)))
    }

    #[inline]
    fn id(&self) -> Id {
        self.inner.id()
    }

    #[inline]
    fn exist(&self) -> bool {
        self.inner.exist()
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
    async fn kick(&mut self, clear_subscriptions: bool, is_admin: IsAdmin) -> Result<Option<SessionOfflineInfo>> {
        log::debug!("{:?} ClusterLockEntry kick 1 ...", self.client().map(|c| c.id.clone()));
        if let Some(kicked) = self.inner.kick(clear_subscriptions, is_admin).await? {
            log::debug!("{:?} broadcast kick reply kicked: {:?}", self.id(), kicked);
            return Ok(Some(kicked));
        }

        match MessageBroadcaster::new(
            self.cluster_shared.grpc_clients.clone(),
            self.cluster_shared.message_type,
            Message::Kick(self.id(), true, is_admin),
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
                    for (tf, (qos, group)) in kicked.subscriptions.iter() {
                        router.add(tf, self.id(), *qos, (*group).clone()).await?;
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
    fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }

    #[inline]
    fn session(&self) -> Option<Session> {
        self.inner.session()
    }

    #[inline]
    fn client(&self) -> Option<ClientInfo> {
        self.inner.client()
    }

    #[inline]
    fn tx(&self) -> Option<Tx> {
        self.inner.tx()
    }

    #[inline]
    async fn subscribe(&self, subscribe: &Subscribe) -> Result<SubscribeReturn> {
        self.inner.subscribe(subscribe).await
    }

    #[inline]
    async fn unsubscribe(&self, unsubscribe: &Unsubscribe) -> Result<bool> {
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
    fn id(&self, client_id: &str) -> Option<Id> {
        self.inner.id(client_id)
    }

    #[inline]
    fn exist(&self, client_id: &str) -> bool {
        self.inner.exist(client_id)
    }

    #[inline]
    async fn forwards(&self, from: From, publish: Publish) -> Result<(), Vec<(To, From, Publish, Reason)>> {
        let this_node_id = Runtime::instance().node.id();
        let topic = publish.topic();
        log::debug!("forwards, from: {:?}, topic: {:?}", from, topic.to_string());

        //Matching subscriptions
        let (relations, shared_relations) =
            match Runtime::instance().extends.router().await.matches(topic).await {
                Ok(mut relations_map) => {
                    let mut relations = SubRelations::new();
                    let mut shared_relations = Vec::new();
                    for (node_id, rels) in relations_map.drain() {
                        for (topic_filter, client_id, qos, group) in rels {
                            if let Some(group) = group {
                                //pub type SharedSubRelations = HashMap<TopicFilterString, Vec<(SharedGroup, NodeId, ClientId, QoS, IsOnline)>>;
                                shared_relations.push((topic_filter, node_id, client_id, qos, group));
                            } else {
                                relations.push((topic_filter, client_id, qos, None));
                            }
                        }
                    }
                    (relations, shared_relations)
                }
                Err(e) => {
                    log::warn!("forwards, from:{:?}, topic:{:?}, error: {:?}", from, topic, e);
                    (Vec::new(), Vec::new())
                }
            };

        log::debug!("relations: {}, shared_relations:{}", relations.len(), shared_relations.len());

        //forwards to local
        let local_res = self.inner.forwards_to(from.clone(), &publish, relations).await;
        log::debug!("forwards, from: {:?}, local_res: {:?}", from, local_res);

        //forwards to remote
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

            type SharedSubGroups = HashMap<
                TopicFilterString, //key is TopicFilter
                HashMap<SharedGroup, Vec<(NodeId, ClientId, QoS, Option<IsOnline>)>>,
            >;
            type SharedRelation = (TopicFilter, NodeId, ClientId, QoS, (SharedGroup, IsOnline));

            let mut shared_sub_groups: SharedSubGroups = HashMap::default();

            let add_one_to_shared_sub_groups =
                |shared_groups: &mut SharedSubGroups, shared_rel: SharedRelation| {
                    let (topic_filter, node_id, client_id, qos, (group, is_online)) = shared_rel;
                    if let Some(groups) = shared_groups.get_mut(topic_filter.as_str()) {
                        groups.entry(group).or_default().push((node_id, client_id, qos, Some(is_online)));
                    } else {
                        let mut groups = HashMap::default();
                        groups.insert(group, vec![(node_id, client_id, qos, Some(is_online))]);
                        shared_groups.insert(topic_filter.to_string(), groups);
                    }
                };

            let add_to_shared_sub_groups =
                |shared_groups: &mut SharedSubGroups, mut shared_rels: Vec<SharedRelation>| {
                    for shared_rel in shared_rels.drain(..) {
                        add_one_to_shared_sub_groups(shared_groups, shared_rel);
                    }
                };

            add_to_shared_sub_groups(&mut shared_sub_groups, shared_relations);

            for reply in replys {
                match reply {
                    Ok(reply) => {
                        if let MessageReply::Forwards(mut o_relations_map) = reply {
                            log::debug!("other noade relations: {:?}", o_relations_map);
                            for (node_id, rels) in o_relations_map.drain() {
                                for (topic_filter, client_id, qos, group) in rels {
                                    if let Some(group) = group {
                                        add_one_to_shared_sub_groups(
                                            &mut shared_sub_groups,
                                            (topic_filter, node_id, client_id, qos, group),
                                        );
                                    }
                                }
                            }
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

            //shared subscription choice
            let mut node_shared_subs: HashMap<NodeId, SubRelations> = HashMap::default();
            for (topic_filter, sub_groups) in shared_sub_groups.iter_mut() {
                for (_group, subs) in sub_groups.iter_mut() {
                    if let Some((idx, _is_online)) =
                    Runtime::instance().extends.shared_subscription().await.choice(subs).await
                    {
                        let (node_id, client_id, qos, _is_online) = subs.remove(idx);
                        node_shared_subs.entry(node_id).or_default().push((
                            TopicFilter::from(topic_filter.clone()),
                            client_id,
                            qos,
                            None,
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
            for (id, (_addr, grpc_client)) in grpc_clients.iter() {
                if let Some(sub_rels) = node_shared_subs.remove(id) {
                    delivers.push(grpc_client.send_message(
                        message_type,
                        Message::ForwardsTo(from.clone(), publish.clone(), sub_rels),
                    ));
                }
            }
            if !delivers.is_empty() {
                let ress = futures::future::join_all(delivers).await;
                for res in ress {
                    if let Err(e) = res {
                        log::error!("deliver shared subscriptions error, {:?}", e);
                    }
                }
            }
        };

        tokio::spawn(broadcast_fut);

        local_res?;
        Ok(())
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
    fn subscriptions(&self) -> usize {
        self.inner.subscriptions()
    }

    #[inline]
    fn subscriptions_shared(&self) -> usize {
        self.inner.subscriptions_shared()
    }

    #[inline]
    fn iter(&self) -> Box<dyn Iterator<Item=Box<dyn Entry>> + Sync + Send> {
        self.inner.iter()
    }

    #[inline]
    fn random_session(&self) -> Option<(Session, ClientInfo)> {
        self.inner.random_session()
    }


    #[inline]
    async fn session_status(&self, client_id: &str) -> Option<SessionStatus> {
        self.inner.session_status(client_id).await
    }

    #[inline]
    fn get_grpc_clients(&self) -> GrpcClients {
        self.grpc_clients.clone()
    }
}
