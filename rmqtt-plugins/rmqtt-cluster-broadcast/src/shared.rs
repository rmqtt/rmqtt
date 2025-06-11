use std::convert::From as _f;
use std::time::Duration;

use async_trait::async_trait;

use rmqtt::context::ServerContext;
use rmqtt::net::MqttError;
use rmqtt::shared::{DefaultShared, Shared};
use rmqtt::types::MsgID;
use rmqtt::{
    grpc::{GrpcClients, Message, MessageBroadcaster, MessageReply, MessageSender, MessageType},
    session::Session,
    shared::Entry,
    types::{
        ClientId, From, Id, IsAdmin, IsOnline, NodeId, OfflineSession, Publish, Reason, SessionStatus,
        SharedGroup, SharedGroupType, SubRelations, SubRelationsMap, SubsSearchParams, SubsSearchResult,
        Subscribe, SubscribeReturn, SubscriptionClientIds, SubscriptionIdentifier, SubscriptionOptions, To,
        TopicFilter, Tx, Unsubscribe,
    },
    Result,
};

use super::{hook_message_dropped, kick};

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

pub struct ClusterLockEntry {
    inner: Box<dyn Entry>,
    cluster_shared: ClusterShared,
}

impl ClusterLockEntry {
    #[inline]
    pub fn new(inner: Box<dyn Entry>, cluster_shared: ClusterShared) -> Self {
        Self { inner, cluster_shared }
    }
}

#[async_trait]
impl Entry for ClusterLockEntry {
    #[inline]
    async fn try_lock(&self) -> Result<Box<dyn Entry>> {
        Ok(Box::new(ClusterLockEntry::new(self.inner.try_lock().await?, self.cluster_shared.clone())))
    }

    #[inline]
    fn id(&self) -> Id {
        self.inner.id()
    }

    #[inline]
    fn id_same(&self) -> Option<bool> {
        self.inner.id_same()
    }

    #[inline]
    fn exist(&self) -> bool {
        self.inner.exist()
    }

    #[inline]
    async fn set(&mut self, session: Session, tx: Tx) -> Result<()> {
        self.inner.set(session, tx).await
    }

    #[inline]
    async fn remove(&mut self) -> Result<Option<(Session, Tx)>> {
        self.inner.remove().await
    }

    #[inline]
    async fn remove_with(&mut self, id: &Id) -> Result<Option<(Session, Tx)>> {
        self.inner.remove_with(id).await
    }

    #[inline]
    async fn kick(
        &mut self,
        clean_start: bool,
        clear_subscriptions: bool,
        is_admin: IsAdmin,
    ) -> Result<OfflineSession> {
        log::debug!("{:?} ClusterLockEntry kick 1 ...", self.session().map(|s| s.id.clone()));
        if let OfflineSession::Exist(kicked) =
            self.inner.kick(clean_start, clear_subscriptions, is_admin).await?
        {
            log::debug!("{:?} broadcast kick reply kicked: {:?}", self.id(), kicked);
            return Ok(OfflineSession::Exist(kicked));
        }

        match kick(
            self.cluster_shared.grpc_clients.clone(),
            self.cluster_shared.message_type,
            Message::Kick(self.id(), clean_start, true, is_admin),
        )
        .await
        {
            Ok(kicked) => {
                log::debug!("{:?} broadcast kick reply kicked: {:?}", self.id(), kicked);
                Ok(kicked)
            }
            Err(e) => {
                log::debug!("{:?}, broadcast Message::Kick reply: {:?}", self.id(), e);
                Ok(OfflineSession::NotExist)
            }
        }
    }

    #[inline]
    async fn online(&self) -> bool {
        if self.inner.online().await {
            return true;
        }

        MessageBroadcaster::new(
            self.cluster_shared.grpc_clients.clone(),
            self.cluster_shared.message_type,
            Message::Online(self.id().client_id.clone()),
            Some(Duration::from_secs(10)),
        )
        .select_ok(|reply: MessageReply| -> Result<bool> {
            if let MessageReply::Online(true) = reply {
                Ok(true)
            } else {
                Err(MqttError::None.into())
            }
        })
        .await
        .unwrap_or(false)
    }

    #[inline]
    async fn is_connected(&self) -> bool {
        self.inner.is_connected().await
    }

    #[inline]
    fn session(&self) -> Option<Session> {
        self.inner.session()
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
    async fn publish(&self, from: From, p: Publish) -> std::result::Result<(), (From, Publish, Reason)> {
        self.inner.publish(from, p).await
    }

    #[inline]
    async fn subscriptions(&self) -> Option<Vec<SubsSearchResult>> {
        if let Some(subs) = self.inner.subscriptions().await {
            return Some(subs);
        }
        MessageBroadcaster::new(
            self.cluster_shared.grpc_clients.clone(),
            self.cluster_shared.message_type,
            Message::SubscriptionsGet(self.id().client_id.clone()),
            Some(Duration::from_secs(10)),
        )
        .select_ok(|reply: MessageReply| -> Result<Option<Vec<SubsSearchResult>>> {
            if let MessageReply::SubscriptionsGet(Some(subs)) = reply {
                Ok(Some(subs))
            } else {
                Err(MqttError::None.into())
            }
        })
        .await
        .unwrap_or(None)
    }
}

#[derive(Clone)]
pub struct ClusterShared {
    inner: DefaultShared,
    scx: ServerContext,
    grpc_clients: GrpcClients,
    pub message_type: MessageType,
}

impl ClusterShared {
    #[inline]
    pub(crate) fn new(
        scx: ServerContext,
        grpc_clients: GrpcClients,
        message_type: MessageType,
    ) -> ClusterShared {
        let scx1 = scx.clone();
        Self { inner: DefaultShared::new(Some(scx1)), scx, grpc_clients, message_type }
    }

    #[inline]
    pub(crate) fn inner(&self) -> &DefaultShared {
        &self.inner
    }
}

#[async_trait]
impl Shared for ClusterShared {
    #[inline]
    fn entry(&self, id: Id) -> Box<dyn Entry> {
        Box::new(ClusterLockEntry::new(self.inner.entry(id), self.clone()))
    }

    #[inline]
    fn exist(&self, client_id: &str) -> bool {
        self.inner.exist(client_id)
    }

    #[inline]
    async fn forwards(
        &self,
        from: From,
        publish: Publish,
    ) -> std::result::Result<SubscriptionClientIds, Vec<(To, From, Publish, Reason)>> {
        let this_node_id = self.scx.node.id();
        let topic = &publish.topic;
        log::debug!("forwards, from: {:?}, topic: {:?}", from, topic.to_string());

        //Matching subscriptions
        let (relations, shared_relations, sub_client_ids) = match self
            .scx
            .extends
            .router()
            .await
            .matches(from.id.clone(), topic)
            .await
        {
            Ok(mut relations_map) => {
                //let subs_size: SubscriptionSize = relations_map.values().map(|subs| subs.len()).sum();
                let sub_client_ids = self.inner()._collect_subscription_client_ids(&relations_map);
                let mut relations = SubRelations::new();
                let mut shared_relations = Vec::new();
                for (node_id, rels) in relations_map.drain() {
                    for (topic_filter, client_id, opts, sub_ids, group) in rels {
                        if let Some(group) = group {
                            //pub type SharedSubRelations = HashMap<TopicFilterString, Vec<(SharedGroup, NodeId, ClientId, QoS, IsOnline)>>;
                            shared_relations.push((topic_filter, node_id, client_id, opts, sub_ids, group));
                        } else {
                            relations.push((topic_filter, client_id, opts, sub_ids, None));
                        }
                    }
                }
                (relations, shared_relations, sub_client_ids)
            }
            Err(e) => {
                log::warn!("forwards, from:{:?}, topic:{:?}, error: {:?}", from, topic, e);
                (Vec::new(), Vec::new(), None)
            }
        };

        log::debug!("relations: {}, shared_relations:{}", relations.len(), shared_relations.len());

        //forwards to local
        let local_res = self.inner.forwards_to(from.clone(), &publish, relations).await;
        log::debug!("forwards, from: {:?}, local_res: {:?}", from, local_res);

        //forwards to remote
        let grpc_clients = self.grpc_clients.clone();
        let message_type = self.message_type;
        let inner = self.inner.clone();
        let scx = self.scx.clone();
        let (sub_client_ids_tx, sub_client_ids_rx) = tokio::sync::oneshot::channel();
        let broadcast_fut = async move {
            scx.stats.forwards.inc();
            //forwards to other node and get shared subscription relations
            let replys = MessageBroadcaster::new(
                grpc_clients.clone(),
                message_type,
                Message::Forwards(from.clone(), publish.clone()),
                Some(Duration::from_secs(10)),
            )
            .join_all()
            .await;
            scx.stats.forwards.dec();
            type SharedSubGroups = HashMap<
                TopicFilter, //key is TopicFilter
                HashMap<
                    SharedGroup,
                    Vec<(
                        NodeId,
                        ClientId,
                        SubscriptionOptions,
                        Option<Vec<SubscriptionIdentifier>>,
                        Option<IsOnline>,
                    )>,
                >,
            >;
            type SharedRelation = (
                TopicFilter,
                NodeId,
                ClientId,
                SubscriptionOptions,
                Option<Vec<SubscriptionIdentifier>>,
                SharedGroupType,
            );

            #[allow(clippy::mutable_key_type)]
            let mut shared_sub_groups: SharedSubGroups = HashMap::default();

            let add_one_to_shared_sub_groups =
                |shared_groups: &mut SharedSubGroups, shared_rel: SharedRelation| {
                    let (topic_filter, node_id, client_id, opts, sub_ids, (group, is_online, _client_ids)) =
                        shared_rel;
                    if let Some(groups) = shared_groups.get_mut(&topic_filter) {
                        groups.entry(group).or_default().push((
                            node_id,
                            client_id,
                            opts,
                            sub_ids,
                            Some(is_online),
                        ));
                    } else {
                        #[allow(clippy::mutable_key_type)]
                        let mut groups = HashMap::default();
                        groups.insert(group, vec![(node_id, client_id, opts, sub_ids, Some(is_online))]);
                        shared_groups.insert(topic_filter, groups);
                    }
                };

            let add_to_shared_sub_groups =
                |shared_groups: &mut SharedSubGroups, mut shared_rels: Vec<SharedRelation>| {
                    for shared_rel in shared_rels.drain(..) {
                        add_one_to_shared_sub_groups(shared_groups, shared_rel);
                    }
                };

            add_to_shared_sub_groups(&mut shared_sub_groups, shared_relations);
            let mut all_sub_client_ids = Vec::new();
            for (_, reply) in replys {
                match reply {
                    Ok(reply) => {
                        if let MessageReply::Forwards(mut o_relations_map, o_sub_client_ids) = reply {
                            log::debug!(
                                "other noade relations: {:?}, o_sub_client_ids: {:?}",
                                o_relations_map,
                                o_sub_client_ids
                            );

                            if let Some(o_sub_client_ids) = o_sub_client_ids {
                                all_sub_client_ids.extend(o_sub_client_ids);
                            }

                            for (node_id, rels) in o_relations_map.drain() {
                                for (topic_filter, client_id, opts, sub_ids, group) in rels {
                                    if let Some(group) = group {
                                        add_one_to_shared_sub_groups(
                                            &mut shared_sub_groups,
                                            (topic_filter, node_id, client_id, opts, sub_ids, group),
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

            let _ = sub_client_ids_tx.send(all_sub_client_ids);

            //shared subscription choice
            let mut node_shared_subs: HashMap<NodeId, SubRelations> = HashMap::default();
            for (topic_filter, sub_groups) in shared_sub_groups.iter_mut() {
                for (_group, subs) in sub_groups.iter_mut() {
                    if let Some((idx, _is_online)) =
                        scx.extends.shared_subscription().await.choice(&scx, subs).await
                    {
                        let (node_id, client_id, opts, sub_ids, _is_online) = subs.remove(idx);
                        node_shared_subs.entry(node_id).or_default().push((
                            topic_filter.clone(),
                            client_id,
                            opts,
                            sub_ids,
                            None,
                        ));
                    }
                }
            }
            log::debug!("node_shared_subs: {:?}", node_shared_subs);

            //send to this node
            if let Some(sub_rels) = node_shared_subs.remove(&this_node_id) {
                if let Err(droppeds) = inner.forwards_to(from.clone(), &publish, sub_rels).await {
                    hook_message_dropped(&scx, droppeds).await;
                }
            }

            //send to other node
            let mut delivers = Vec::new();
            for (id, (_addr, grpc_client)) in grpc_clients.iter() {
                if let Some(sub_rels) = node_shared_subs.remove(id) {
                    delivers.push(
                        MessageSender::new(
                            grpc_client.clone(),
                            message_type,
                            Message::ForwardsTo(from.clone(), publish.clone(), sub_rels),
                            Some(Duration::from_secs(60)),
                        )
                        .send(),
                    );
                }
            }
            if !delivers.is_empty() {
                scx.stats.forwards.inc();
                let ress = futures::future::join_all(delivers).await;
                for res in ress {
                    if let Err(e) = res {
                        log::error!("deliver shared subscriptions error, {:?}", e);
                    }
                }
                scx.stats.forwards.dec();
            }
        };

        tokio::spawn(broadcast_fut);

        let sub_client_ids =
            self.inner()._merge_subscription_client_ids(sub_client_ids, sub_client_ids_rx.await.ok());

        local_res?;
        Ok(sub_client_ids)
    }

    #[inline]
    async fn forwards_and_get_shareds(
        &self,
        from: From,
        publish: Publish,
    ) -> std::result::Result<(SubRelationsMap, SubscriptionClientIds), Vec<(To, From, Publish, Reason)>> {
        self.inner.forwards_and_get_shareds(from, publish).await
    }

    #[inline]
    async fn forwards_to(
        &self,
        from: From,
        publish: &Publish,
        relations: SubRelations,
    ) -> std::result::Result<(), Vec<(To, From, Publish, Reason)>> {
        self.inner.forwards_to(from, publish, relations).await
    }

    #[inline]
    fn iter(&self) -> Box<dyn Iterator<Item = Box<dyn Entry>> + Sync + Send + '_> {
        self.inner.iter()
    }

    #[inline]
    fn random_session(&self) -> Option<Session> {
        self.inner.random_session()
    }

    #[inline]
    async fn session_status(&self, client_id: &str) -> Option<SessionStatus> {
        if let Some(status) = self.inner.session_status(client_id).await {
            return Some(status);
        }
        MessageBroadcaster::new(
            self.grpc_clients.clone(),
            self.message_type,
            Message::SessionStatus(ClientId::from(client_id)),
            Some(Duration::from_secs(10)),
        )
        .select_ok(|reply: MessageReply| -> Result<SessionStatus> {
            if let MessageReply::SessionStatus(Some(status)) = reply {
                Ok(status)
            } else {
                Err(MqttError::None.into())
            }
        })
        .await
        .ok()
    }

    #[inline]
    async fn client_states_count(&self) -> usize {
        self.inner.client_states_count().await
    }

    #[inline]
    fn sessions_count(&self) -> usize {
        self.inner.sessions_count()
    }

    #[inline]
    async fn query_subscriptions(&self, q: &SubsSearchParams) -> Vec<SubsSearchResult> {
        let limit = q._limit;
        let mut replys = self.inner.query_subscriptions(q).await;

        let grpc_clients = self.get_grpc_clients();
        for c in grpc_clients.iter().map(|(_, (_, c))| c.clone()) {
            if replys.len() < limit {
                let mut q = q.clone();
                q._limit = limit - replys.len();
                let reply = MessageSender::new(
                    c,
                    self.message_type,
                    Message::SubscriptionsSearch(q),
                    Some(Duration::from_secs(10)),
                )
                .send()
                .await;
                match reply {
                    Ok(MessageReply::SubscriptionsSearch(subs)) => {
                        replys.extend(subs);
                    }
                    Err(e) => {
                        log::warn!("query_subscriptions, error: {:?}", e);
                    }
                    _ => unreachable!(),
                };
            } else {
                break;
            }
        }

        replys
    }

    #[inline]
    async fn message_load(
        &self,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
    ) -> Result<Vec<(MsgID, From, Publish)>> {
        self.inner.message_load(client_id, topic_filter, group).await
    }

    #[inline]
    async fn subscriptions_count(&self) -> usize {
        self.inner.subscriptions_count().await
    }

    #[inline]
    fn get_grpc_clients(&self) -> GrpcClients {
        self.grpc_clients.clone()
    }
}
