//! Cluster-aware shared subscription and session management.
//!
//! Defines [`ClusterLockEntry`] and [`ClusterShared`] which extend the local
//! shared state (sessions/subscriptions) with gRPC-based inter-node
//! forwarding, health checks, and subscription queries.
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use rust_box::task_exec_queue::{SpawnExt, TaskExecQueue};

use rmqtt::context::ServerContext;
use rmqtt::net::MqttError;
use rmqtt::shared::{DefaultShared, Entry, MessageLoadCallback, Shared};
use rmqtt::types::{HealthInfo, MsgID, NodeHealthStatus};
use rmqtt::{
    grpc::{GrpcClient, GrpcClients, Message, MessageBroadcaster, MessageReply, MessageSender, MessageType},
    session::Session,
    types::{
        ClientId, ForwardedCount, ForwardedRecipients, From, Id, IsAdmin, IsOnline, NodeId, NodeName,
        OfflineSession, Publish, Reason, SessionStatus, SharedGroup, SharedGroupType, SubRelations,
        SubRelationsMap, SubsSearchParams, SubsSearchResult, Subscribe, SubscribeReturn,
        SubscriptionIdentifier, SubscriptionOptions, To, TopicFilter, Tx, Unsubscribe,
    },
    Result,
};

use super::message::{BroadcastGrpcMessage, BroadcastGrpcMessageReply};
use super::{hook_message_dropped, kick};

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

/// A locked session entry that broadcasts session operations (kick,
/// online checks, subscription queries) to remote cluster nodes.
pub struct ClusterLockEntry {
    inner: Box<dyn Entry>,
    cluster_shared: ClusterShared,
}

impl ClusterLockEntry {
    /// Creates a new cluster lock entry wrapping an inner [`Entry`] and
    /// a reference to the [`ClusterShared`] for inter-node communication.
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
        // self.cluster_shared.exec
        match kick(
            self.cluster_shared.grpc_clients.clone(),
            self.cluster_shared.message_type,
            Message::Kick(self.id(), clean_start, true, is_admin),
        )
        .spawn(&self.cluster_shared.exec)
        .result()
        .await
        .map_err(|e| anyhow!(e.to_string()))
        .flatten()
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
            Some(self.cluster_shared.rw_timeout),
        )
        .select_ok(|reply: MessageReply| -> Result<bool> {
            if let MessageReply::Online(true) = reply {
                Ok(true)
            } else {
                Err(MqttError::None.into())
            }
        })
        .spawn(&self.cluster_shared.exec)
        .result()
        .await
        .map_err(|e| anyhow!(e.to_string()))
        .flatten()
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
            Some(self.cluster_shared.rw_timeout),
        )
        .select_ok(|reply: MessageReply| -> Result<Option<Vec<SubsSearchResult>>> {
            if let MessageReply::SubscriptionsGet(Some(subs)) = reply {
                Ok(Some(subs))
            } else {
                Err(MqttError::None.into())
            }
        })
        .spawn(&self.cluster_shared.exec)
        .result()
        .await
        .map_err(|e| anyhow!(e.to_string()))
        .flatten()
        .unwrap_or(None)
    }
}

/// Cluster-aware shared subscription state that extends [`DefaultShared`]
/// with gRPC-based forwarding to remote nodes.
#[derive(Clone)]
pub struct ClusterShared {
    inner: DefaultShared,
    scx: ServerContext,
    exec: TaskExecQueue,
    forwards_exec: TaskExecQueue,
    grpc_clients: GrpcClients,
    /// The gRPC message type used for cluster communication.
    pub message_type: MessageType,
    node_names: Arc<HashMap<NodeId, NodeName>>,
    exec_queue_busy_limit: isize,
    exec_queue_workers_busy_limit: isize,
    rw_timeout: Duration,
}

impl ClusterShared {
    /// Creates a new [`ClusterShared`] wrapping a [`DefaultShared`] and
    /// gRPC clients for inter-node communication.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub(crate) fn new(
        scx: ServerContext,
        exec: TaskExecQueue,
        grpc_clients: GrpcClients,
        message_type: MessageType,
        node_names: HashMap<NodeId, NodeName>,
        exec_queue_max: usize,
        exec_queue_workers: usize,
        rw_timeout: Duration,
    ) -> ClusterShared {
        let scx1 = scx.clone();
        let forwards_exec = scx.get_exec("BC_FORWARDS_EXEC");
        Self {
            inner: DefaultShared::new(Some(scx1)),
            scx,
            exec,
            forwards_exec,
            grpc_clients,
            message_type,
            node_names: Arc::new(node_names),
            exec_queue_busy_limit: (exec_queue_max as f64 * 0.7) as isize,
            exec_queue_workers_busy_limit: (exec_queue_workers as f64 * 0.9) as isize,
            rw_timeout,
        }
    }

    #[inline]
    pub(crate) fn inner(&self) -> &DefaultShared {
        &self.inner
    }

    #[inline]
    pub(crate) fn grpc_client(&self, node_id: u64) -> Option<GrpcClient> {
        self.grpc_clients.get(&node_id).map(|(_, c)| c.clone())
    }

    /// Converts [`SubRelations`] to [`ForwardedRecipients`].
    ///
    /// For shared-group subscriptions, each group member is emitted separately:
    /// the actual subscriber carries `Some((topic_filter, shared_group))`,
    /// while other group members carry `None`.
    #[inline]
    pub(crate) fn sub_relations_to_client_ids(relations: &SubRelations) -> ForwardedRecipients {
        relations
            .iter()
            .flat_map(|(tf, cid, _, _, group_shared)| {
                if let Some((group, _, cids)) = group_shared {
                    cids.iter()
                        .map(|g_cid| {
                            if g_cid == cid {
                                (g_cid.clone(), Some((tf.clone(), group.clone())))
                            } else {
                                (g_cid.clone(), None)
                            }
                        })
                        .collect()
                } else {
                    vec![(cid.clone(), None)]
                }
            })
            .collect()
    }

    #[inline]
    async fn forward_to_target_client(
        &self,
        msg_id: Option<MsgID>,
        from: From,
        publish: Publish,
        target_clientid: ClientId,
    ) -> std::result::Result<ForwardedRecipients, (ForwardedRecipients, Vec<(To, From, Publish, Reason)>)>
    {
        let mut opts = SubscriptionOptions::default();
        opts.set_qos(publish.qos);
        let relations = vec![(publish.topic.clone(), target_clientid.clone(), opts, None, None)];

        //forwards to local
        if self.exist(&target_clientid) {
            let recipients = self.inner.forwards_to(from.clone(), &publish, relations, msg_id).await?;
            Ok(recipients)
        } else {
            //forward other nodes
            let grpc_clients = self.grpc_clients.clone();
            let message_type = self.message_type;
            let rw_timeout = self.rw_timeout;
            // let inner = self.inner.clone();
            let scx = self.scx.clone();
            let (forwardeds_tx, forwardeds_rx) = tokio::sync::oneshot::channel();
            let broadcast_fut = async move {
                scx.stats.forwards.inc();
                //forwards to other node and get shared subscription relations
                let replys = MessageBroadcaster::new(
                    grpc_clients.clone(),
                    message_type,
                    Message::Forwards(from.clone(), publish.clone()),
                    Some(rw_timeout),
                )
                .join_all()
                .await;
                scx.stats.forwards.dec();

                let mut all_forwardeds = Vec::new();
                for (_, reply) in replys {
                    match reply {
                        Ok(reply) => {
                            if let MessageReply::Forwards(o_relations_map, o_forwardeds) = reply {
                                log::debug!(
                                    "other noade relations: {o_relations_map:?}, o_forwardeds: {o_forwardeds:?}"
                                    );
                                all_forwardeds.extend(o_forwardeds);
                            }
                        }
                        Err(e) => {
                            log::error!(
                                "forwards Message::Forwards to other node, from: {from:?}, error: {e:?}"
                            );
                        }
                    }
                }

                let _ = forwardeds_tx.send(all_forwardeds);
            };

            if let Err(e) = self.forwards_exec.spawn(broadcast_fut).await {
                log::warn!("{:?}", e.to_string());
            }

            Ok(forwardeds_rx.await.unwrap_or_default())
        }
    }

    #[inline]
    async fn forward_to_subscriptions(
        &self,
        msg_id: Option<MsgID>,
        from: From,
        publish: Publish,
    ) -> std::result::Result<ForwardedRecipients, (ForwardedRecipients, Vec<(To, From, Publish, Reason)>)>
    {
        let this_node_id = self.scx.node.id();
        let topic = &publish.topic;
        log::debug!("forwards, from: {:?}, topic: {:?}", from, topic.to_string());
        //Matching subscriptions
        let (relations, shared_relations, forwardeds) = match self
            .scx
            .extends
            .router()
            .await
            .matches(from.id.clone(), topic)
            .await
        {
            Ok(mut relations_map) => {
                //let subs_size: SubscriptionSize = relations_map.values().map(|subs| subs.len()).sum();
                let forwardeds = self.inner()._collect_subscription_client_ids(&relations_map);
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
                (relations, shared_relations, forwardeds)
            }
            Err(e) => {
                log::warn!("forwards, from:{from:?}, topic:{topic:?}, error: {e:?}");
                (Vec::new(), Vec::new(), Vec::new())
            }
        };

        log::debug!("relations: {}, shared_relations:{}", relations.len(), shared_relations.len());

        //forwards to local
        self.inner.forwards_to(from.clone(), &publish, relations, msg_id).await?;

        //forwards to remote
        let grpc_clients = self.grpc_clients.clone();
        let message_type = self.message_type;
        let inner = self.inner.clone();
        let scx = self.scx.clone();
        let rw_timeout = self.rw_timeout;
        let (forwardeds_tx, forwardeds_rx) = tokio::sync::oneshot::channel();
        let broadcast_fut = async move {
            scx.stats.forwards.inc();
            //forwards to other node and get shared subscription relations
            let replys = MessageBroadcaster::new(
                grpc_clients.clone(),
                message_type,
                Message::Forwards(from.clone(), publish.clone()),
                Some(rw_timeout),
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
            let mut all_forwardeds = Vec::new();
            for (_, reply) in replys {
                match reply {
                    Ok(reply) => {
                        if let MessageReply::Forwards(mut o_relations_map, o_forwardeds) = reply {
                            log::debug!(
                                "other noade relations: {o_relations_map:?}, o_forwardeds: {o_forwardeds:?}"
                            );
                            all_forwardeds.extend(o_forwardeds);

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
                        log::error!("forwards Message::Forwards to other node, from: {from:?}, error: {e:?}");
                    }
                }
            }

            let _ = forwardeds_tx.send(all_forwardeds);

            //shared subscription choice
            let mut node_shared_subs: HashMap<NodeId, SubRelations> = HashMap::default();
            for (topic_filter, sub_groups) in shared_sub_groups.iter_mut() {
                for (group, subs) in sub_groups.iter_mut() {
                    if let Some((idx, _is_online)) = scx
                        .extends
                        .shared_subscription()
                        .await
                        .choice(&scx, group, &from.id, &publish.topic, subs)
                        .await
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
            log::debug!("node_shared_subs: {node_shared_subs:?}");

            //send to this node
            if let Some(sub_rels) = node_shared_subs.remove(&this_node_id) {
                if let Err((_, droppeds)) = inner.forwards_to(from.clone(), &publish, sub_rels, None).await {
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
                            Message::ForwardsTo(from.clone(), publish.clone(), sub_rels, None),
                            Some(rw_timeout),
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
                        log::error!("deliver shared subscriptions error, {e:?}");
                    }
                }
                scx.stats.forwards.dec();
            }
        };

        if let Err(e) = self.forwards_exec.spawn(broadcast_fut).await {
            log::warn!("{:?}", e.to_string());
        }

        let forwardeds =
            self.inner()._merge_subscription_client_ids(forwardeds, forwardeds_rx.await.unwrap_or_default());

        Ok(forwardeds)
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
        msg_id: Option<MsgID>,
        from: From,
        publish: Publish,
    ) -> std::result::Result<ForwardedCount, (ForwardedCount, Vec<(To, From, Publish, Reason)>)> {
        let (subscriber_count, forwardeds) = if let Some(target_clientid) = publish.target_clientid.clone() {
            match self.forward_to_target_client(msg_id, from, publish, target_clientid).await {
                Ok(forwardeds) => (1, forwardeds),
                Err((forwardeds, errs)) => return Err((forwardeds.len(), errs)),
            }
        } else {
            match self.forward_to_subscriptions(msg_id, from, publish).await {
                Ok(forwardeds) => (forwardeds.len(), forwardeds),
                Err((forwardeds, errs)) => return Err((forwardeds.len(), errs)),
            }
        };
        // Record subscriber IDs after successful forwarding.
        if let Some(msg_id) = msg_id {
            if !forwardeds.is_empty() {
                if let Err(e) = self.scx.extends.message_mgr().await.mark_forwarded(msg_id, forwardeds).await
                {
                    log::warn!("forwards: mark_forwarded error, msg_id: {:?}, {e:?}", msg_id);
                }
            }
        }
        Ok(subscriber_count)
    }

    #[inline]
    async fn forwards_and_get_shareds(
        &self,
        from: From,
        publish: Publish,
    ) -> std::result::Result<
        (SubRelationsMap, ForwardedRecipients),
        (ForwardedRecipients, Vec<(To, From, Publish, Reason)>),
    > {
        self.inner.forwards_and_get_shareds(from, publish).await
    }

    #[inline]
    async fn forwards_to(
        &self,
        from: From,
        publish: &Publish,
        relations: SubRelations,
        msg_id: Option<MsgID>,
    ) -> std::result::Result<ForwardedRecipients, (ForwardedRecipients, Vec<(To, From, Publish, Reason)>)>
    {
        let from_node_id = from.node_id;
        let is_this_node = from_node_id == self.scx.node.id();
        let subscribers = if !is_this_node {
            msg_id.map(|msg_id| (msg_id, Self::sub_relations_to_client_ids(&relations)))
        } else {
            None
        };
        let res = self.inner.forwards_to(from, publish, relations, msg_id).await;
        if res.is_ok() {
            if let Some((msg_id, subscribers)) = subscribers {
                // fire-and-forget send ForwardsToAck ack
                if let Some(client) = self.grpc_client(from_node_id) {
                    let ack_msg = Message::ForwardsToAck(msg_id, self.scx.node.id(), subscribers);
                    let sender =
                        MessageSender::new(client, self.message_type, ack_msg, Some(self.rw_timeout));
                    if let Err(e) = sender.notify().spawn(&self.forwards_exec).await {
                        log::warn!(
                            "Failed to send ForwardsToAck to node {}, msg_id={:?}, error: {:?}",
                            from_node_id,
                            msg_id,
                            e.to_string()
                        );
                    }
                }
            }
        }
        res
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
            Some(self.rw_timeout),
        )
        .select_ok(|reply: MessageReply| -> Result<SessionStatus> {
            if let MessageReply::SessionStatus(Some(status)) = reply {
                Ok(status)
            } else {
                Err(MqttError::None.into())
            }
        })
        .spawn(&self.exec)
        .result()
        .await
        .map(|ss| ss.ok())
        .ok()
        .flatten()
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
                    Some(self.rw_timeout),
                )
                .send()
                .await;
                match reply {
                    Ok(MessageReply::SubscriptionsSearch(subs)) => {
                        replys.extend(subs);
                    }
                    Err(e) => {
                        log::warn!("query_subscriptions, error: {e:?}");
                    }
                    _ => {
                        log::error!("unreachable!(), reply: {reply:?}");
                    }
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
        let message_mgr = self.scx.extends.message_mgr().await;
        if message_mgr.merge_on_read() {
            let mut msgs = message_mgr.get(client_id, topic_filter, group).await?;
            if !self.grpc_clients.is_empty() {
                let replys = MessageBroadcaster::new(
                    self.grpc_clients.clone(),
                    self.message_type,
                    Message::MessageGet(
                        ClientId::from(client_id),
                        TopicFilter::from(topic_filter),
                        group.cloned(),
                    ),
                    Some(self.rw_timeout),
                )
                .join_all()
                .await;
                for (_, reply) in replys {
                    let reply = reply?;
                    match reply {
                        MessageReply::Error(e) => return Err(anyhow!(e)),
                        MessageReply::MessageGet(res) => {
                            msgs.extend(res);
                        }
                        _ => {
                            log::error!("unreachable!(), reply: {reply:?}");
                            return Err(anyhow!("unreachable!()"));
                        }
                    }
                }
            }
            Ok(msgs)
        } else {
            message_mgr.get(client_id, topic_filter, group).await
        }
    }

    #[inline]
    async fn message_load_with(
        &self,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
        cb: Arc<dyn MessageLoadCallback>,
    ) -> Result<()> {
        let message_mgr = self.scx.extends.message_mgr().await;
        if message_mgr.merge_on_read() {
            let msgs = message_mgr.get(client_id, topic_filter, group).await?;
            cb.on_messages(msgs).await?;

            if !self.grpc_clients.is_empty() {
                let cb2 = cb.clone();
                MessageBroadcaster::new(
                    self.grpc_clients.clone(),
                    self.message_type,
                    Message::MessageGet(
                        ClientId::from(client_id),
                        TopicFilter::from(topic_filter),
                        group.cloned(),
                    ),
                    Some(self.rw_timeout),
                )
                .join_all_with_async(move |reply| {
                    let msgs = match reply {
                        Ok(MessageReply::MessageGet(msgs)) => msgs,
                        Ok(other) => {
                            log::warn!("unexpected reply: {other:?}");
                            vec![]
                        }
                        Err(e) => {
                            log::warn!("Message::MessageGet failed, {e:?}");
                            vec![]
                        }
                    };
                    let cb2 = cb2.clone();
                    async move { cb2.on_messages(msgs).await }
                })
                .await;
            }
            Ok(())
        } else {
            let msgs = message_mgr.get(client_id, topic_filter, group).await?;
            cb.on_messages(msgs).await
        }
    }

    async fn retain_load_with(
        &self,
        topic_filter: &TopicFilter,
        cb: Arc<dyn rmqtt::shared::RetainLoadCallback>,
    ) -> Result<Vec<(NodeId, MsgID)>> {
        let retain_mgr = self.scx.extends.retain().await;
        if retain_mgr.merge_on_read() {
            let retains = retain_mgr.get(topic_filter).await?;
            let mut excludeds = cb.on_retains(retains).await?;

            if !self.grpc_clients.is_empty() {
                let cb2 = cb.clone();
                let replys = MessageBroadcaster::new(
                    self.grpc_clients.clone(),
                    self.message_type,
                    Message::GetRetains(topic_filter.clone()),
                    Some(self.rw_timeout),
                )
                .join_all_with_async(move |reply| {
                    let retains = match reply {
                        Ok(MessageReply::GetRetains(retains)) => retains,
                        Ok(other) => {
                            log::warn!("unexpected reply: {other:?}");
                            vec![]
                        }
                        Err(e) => {
                            log::warn!("Message::GetRetains failed, {e:?}");
                            vec![]
                        }
                    };
                    let cb2 = cb2.clone();
                    async move { cb2.on_retains(retains).await }
                })
                .await;

                for (node_id, result) in replys {
                    match result {
                        Ok(mut ids) => excludeds.append(&mut ids),
                        Err(e) => log::warn!("Message::GetRetains failed, node_id: {node_id}, {e:?}"),
                    }
                }
            }

            Ok(excludeds)
        } else {
            let retains = retain_mgr.get(topic_filter).await?;
            cb.on_retains(retains).await
        }
    }

    #[inline]
    async fn message_mark_forwarded(
        &self,
        from_node_id: NodeId,
        msg_id: MsgID,
        recipients: ForwardedRecipients,
    ) -> Result<()> {
        if from_node_id == self.scx.node.id() {
            self.scx.extends.message_mgr().await.mark_forwarded(msg_id, recipients).await
        } else {
            if let Some(client) = self.grpc_client(from_node_id) {
                let ack_msg = Message::ForwardsToAck(msg_id, self.scx.node.id(), recipients);
                let sender = MessageSender::new(client, self.message_type, ack_msg, Some(self.rw_timeout));
                if let Err(e) = sender.notify().spawn(&self.forwards_exec).await {
                    log::warn!(
                        "Failed to send ForwardsToAck to node {}, msg_id={:?}, error: {:?}",
                        from_node_id,
                        msg_id,
                        e.to_string()
                    );
                }
            }
            Ok(())
        }
    }

    #[inline]
    async fn subscriptions_count(&self) -> usize {
        self.inner.subscriptions_count().await
    }

    #[inline]
    fn get_grpc_clients(&self) -> GrpcClients {
        self.grpc_clients.clone()
    }

    #[inline]
    async fn check_health(&self) -> Result<HealthInfo> {
        let running = true;
        let descr = "Ok";
        let mut nodes_health_infos = Vec::new();

        nodes_health_infos.push(self.health_status().await?);

        let data = BroadcastGrpcMessage::GetNodeHealthStatus.encode()?;
        let replys = MessageBroadcaster::new(
            self.grpc_clients.clone(),
            self.message_type,
            Message::Data(data),
            Some(Duration::from_secs(5)),
        )
        .join_all()
        .await;

        for (node_id, reply) in replys {
            match reply {
                Ok(reply) => match reply {
                    MessageReply::Data(data) => {
                        let BroadcastGrpcMessageReply::GetNodeHealthStatus(o_status) =
                            BroadcastGrpcMessageReply::decode(&data)?;
                        nodes_health_infos.push(o_status);
                    }
                    MessageReply::Error(err) => {
                        log::error!("Get GrpcMessage::GetNodeHealthStatus from other node, error: {err}");
                        nodes_health_infos.push(NodeHealthStatus {
                            node_id,
                            running: false,
                            leader_id: None,
                            descr: Some(err),
                        });
                    }
                    _ => {
                        log::error!("unreachable!(), reply: {reply:?}")
                    }
                },
                Err(e) => {
                    log::error!("Get GrpcMessage::GetNodeHealthStatus from other node, error: {e:?}");
                    nodes_health_infos.push(NodeHealthStatus {
                        node_id,
                        running: false,
                        leader_id: None,
                        descr: Some(e.to_string()),
                    });
                }
            }
        }

        Ok(HealthInfo { running, descr: Some(descr.into()), nodes: nodes_health_infos })
    }

    #[inline]
    async fn health_status(&self) -> Result<NodeHealthStatus> {
        let node_id = self.scx.node.id();
        Ok(NodeHealthStatus { node_id, running: true, leader_id: None, descr: None })
    }

    #[inline]
    fn operation_is_busy(&self) -> bool {
        self.exec.active_count() > self.exec_queue_workers_busy_limit
            || self.exec.waiting_count() > self.exec_queue_busy_limit
    }

    #[inline]
    fn node_name(&self, id: NodeId) -> String {
        self.node_names.get(&id).cloned().unwrap_or_default()
    }
}
