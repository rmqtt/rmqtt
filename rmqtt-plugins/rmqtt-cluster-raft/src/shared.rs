use std::sync::Arc;
use std::time::Duration;

use ahash::HashSet;
use anyhow::{anyhow, Error};
use async_trait::async_trait;
use futures::future::FutureExt;
use rust_box::task_exec_queue::SpawnExt;

use crate::HashMap;
use rmqtt::context::ServerContext;
use rmqtt::{
    grpc::{GrpcClient, GrpcClients, Message, MessageBroadcaster, MessageReply, MessageSender, MessageType},
    router::Router,
    session::Session,
    shared::{DefaultShared, Entry, Shared},
    types::{
        From, Id, IsAdmin, NodeId, NodeName, OfflineSession, Publish, Reason, SessionStatus, SubRelations,
        SubRelationsMap, SubsSearchParams, SubsSearchResult, Subscribe, SubscribeReturn,
        SubscriptionClientIds, To, Tx, Unsubscribe,
    },
    HealthInfo, MsgID, NodeHealthStatus, Result, SharedGroup,
};

use super::message::{
    get_client_node_id, Message as RaftMessage, MessageReply as RaftMessageReply, RaftGrpcMessage,
    RaftGrpcMessageReply,
};
use super::{task_exec_queue, ClusterRouter};

pub struct ClusterLockEntry {
    scx: ServerContext,
    inner: Box<dyn Entry>,
    cluster_shared: ClusterShared,
    prev_node_id: Option<NodeId>,
}

impl ClusterLockEntry {
    #[inline]
    pub fn new(
        scx: ServerContext,
        inner: Box<dyn Entry>,
        cluster_shared: ClusterShared,
        prev_node_id: Option<NodeId>,
    ) -> Self {
        Self { scx, inner, cluster_shared, prev_node_id }
    }
}

#[async_trait]
impl Entry for ClusterLockEntry {
    #[inline]
    async fn try_lock(&self) -> Result<Box<dyn Entry>> {
        let msg = RaftMessage::HandshakeTryLock { id: self.id() }.encode()?;
        let raft_mailbox = self.cluster_shared.router.raft_mailbox().await;
        let reply = async move { raft_mailbox.send_proposal(msg).await.map_err(anyhow::Error::new) }
            .spawn(task_exec_queue())
            .result()
            .await
            .map_err(|_| anyhow!("ClusterLockEntry::try_lock(..), task execution failure"))??;
        let mut prev_node_id = None;
        if !reply.is_empty() {
            match RaftMessageReply::decode(&reply)? {
                RaftMessageReply::Error(e) => {
                    return Err(anyhow!(e));
                }
                RaftMessageReply::HandshakeTryLock(prev_id) => {
                    prev_node_id = prev_id.map(|id| id.node_id);
                    log::debug!(
                        "{:?} ClusterLockEntry try_lock prev_node_id: {:?}",
                        self.session().map(|s| s.id.clone()),
                        prev_node_id
                    );
                }
                _ => unreachable!(),
            }
        }
        Ok(Box::new(ClusterLockEntry::new(
            self.scx.clone(),
            self.inner.try_lock().await?,
            self.cluster_shared.clone(),
            prev_node_id,
        )))
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
        let msg = RaftMessage::Connected { id: session.id.clone() }.encode()?;
        let raft_mailbox = self.cluster_shared.router.raft_mailbox().await;
        let reply = async move { raft_mailbox.send_proposal(msg).await.map_err(anyhow::Error::new) }
            .spawn(task_exec_queue())
            .result()
            .await
            .map_err(|_| anyhow!("ClusterLockEntry::set(..), task execution failure"))??;
        if !reply.is_empty() {
            let reply = RaftMessageReply::decode(&reply)?;
            match reply {
                RaftMessageReply::Error(e) => {
                    log::error!("RaftMessage::Connected reply: {:?}", e);
                    return Err(anyhow!(e));
                }
                _ => {
                    log::error!("unreachable!(), {:?}", reply);
                    unreachable!()
                }
            }
        }
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
        log::debug!(
            "{:?} ClusterLockEntry kick ..., clean_start: {}, clear_subscriptions: {}, is_admin: {}",
            self.session().map(|s| s.id.clone()),
            clean_start,
            clear_subscriptions,
            is_admin
        );
        let id = self.id();

        let prev_node_id = if is_admin {
            let raft_mailbox = self.cluster_shared.router.raft_mailbox().await;
            let node_id = get_client_node_id(raft_mailbox, &id.client_id).await?;
            node_id.unwrap_or(id.node_id)
        } else {
            self.prev_node_id.unwrap_or(id.node_id)
        };
        log::debug!("{:?} kick, prev_node_id: {:?}, is_admin: {}", id, self.prev_node_id, is_admin);
        if prev_node_id == id.node_id {
            //kicked from local
            self.inner.kick(clean_start, clear_subscriptions, is_admin).await
        } else {
            //kicked from other node
            if let Some(client) = self.cluster_shared.grpc_client(prev_node_id) {
                let message_type = self.cluster_shared.message_type;
                let id1 = id.clone();
                let kick_fut = async move {
                    let msg_sender = MessageSender::new(
                        client,
                        message_type,
                        Message::Kick(id1, clean_start, true, is_admin), //clear_subscriptions
                        Some(Duration::from_secs(10)),
                    );
                    match msg_sender.send().await {
                        Ok(reply) => {
                            if let MessageReply::Kick(kicked) = reply {
                                log::debug!("{:?} kicked: {:?}", id, kicked);
                                kicked
                            } else {
                                log::info!(
                                    "{:?} Message::Kick from other node, prev_node_id: {:?}, reply: {:?}",
                                    id,
                                    prev_node_id,
                                    reply
                                );
                                OfflineSession::NotExist
                            }
                        }
                        Err(e) => {
                            log::error!(
                                "{:?} Message::Kick from other node, prev_node_id: {:?}, error: {:?}",
                                id,
                                prev_node_id,
                                e
                            );
                            OfflineSession::NotExist
                        }
                    }
                };

                let reply =
                    kick_fut.spawn(task_exec_queue()).result().await.map_err(|e| anyhow!(e.to_string()))?;
                Ok(reply)
            } else {
                return Err(anyhow!(format!(
                    "kick error, grpc_client is not exist, prev_node_id: {:?}",
                    prev_node_id
                )));
            }
        }
    }

    #[inline]
    async fn online(&self) -> bool {
        self.cluster_shared.router.is_online(0, &self.id().client_id).await
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
        let id = self.cluster_shared.router.id(&self.id().client_id)?;
        if id.node_id == self.scx.node.id() {
            self.inner.subscriptions().await
        } else {
            //from other node
            if let Some(client) = self.cluster_shared.grpc_client(id.node_id) {
                let reply = MessageSender::new(
                    client,
                    self.cluster_shared.message_type,
                    Message::SubscriptionsGet(id.client_id.clone()),
                    Some(Duration::from_secs(10)),
                )
                .send()
                .await;
                match reply {
                    Ok(MessageReply::SubscriptionsGet(subs)) => subs,
                    Err(e) => {
                        log::warn!("Message::SubscriptionsGet, error: {:?}", e);
                        None
                    }
                    _ => unreachable!(),
                }
            } else {
                log::error!("get subscriptions error, grpc_client is not exist, node_id: {}", id.node_id,);
                None
            }
        }
    }
}

#[derive(Clone)]
pub struct ClusterShared {
    scx: ServerContext,
    inner: DefaultShared,
    router: ClusterRouter,
    grpc_clients: GrpcClients,
    node_names: Arc<HashMap<NodeId, NodeName>>,
    pub(crate) message_type: MessageType,
    exec_queue_busy_limit: isize,
    exec_queue_workers_busy_limit: isize,
}

impl ClusterShared {
    #[inline]
    pub(crate) fn new(
        scx: ServerContext,
        router: ClusterRouter,
        grpc_clients: GrpcClients,
        node_names: HashMap<NodeId, NodeName>,
        message_type: MessageType,
        exec_queue_max: usize,
        exec_queue_workers: usize,
    ) -> ClusterShared {
        let scx1 = scx.clone();
        Self {
            scx,
            inner: DefaultShared::new(Some(scx1)),
            router,
            grpc_clients,
            node_names: Arc::new(node_names),
            message_type,
            exec_queue_busy_limit: (exec_queue_max as f64 * 0.7) as isize,
            exec_queue_workers_busy_limit: (exec_queue_workers as f64 * 0.9) as isize,
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
}

#[async_trait]
impl Shared for ClusterShared {
    #[inline]
    fn entry(&self, id: Id) -> Box<dyn Entry> {
        Box::new(ClusterLockEntry::new(self.scx.clone(), self.inner.entry(id), self.clone(), None))
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
        log::debug!("[forwards] from: {:?}, publish: {:?}", from, publish);

        let topic = &publish.topic;
        let mut relations_map = match self.scx.extends.router().await.matches(from.id.clone(), topic).await {
            Ok(relations_map) => relations_map,
            Err(e) => {
                log::warn!("forwards, from:{:?}, topic:{:?}, error: {:?}", from, topic, e);
                SubRelationsMap::default()
            }
        };

        let sub_client_ids = self.inner()._collect_subscription_client_ids(&relations_map);

        let mut errs = Vec::new();

        let this_node_id = self.scx.node.id();
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
                if let Some(client) = self.grpc_client(node_id) {
                    let from = from.clone();
                    let publish = publish.clone();
                    let message_type = self.message_type;
                    let fut_sender = async move {
                        let msg_sender = MessageSender::new(
                            client,
                            message_type,
                            Message::ForwardsTo(from, publish, relations),
                            Some(Duration::from_secs(10)),
                        );
                        (node_id, msg_sender.send().await)
                    };
                    fut_senders.push(fut_sender.boxed());
                } else {
                    log::warn!(
                        "forwards error, grpc_client is not exist, node_id: {}, relations: {:?}",
                        node_id,
                        relations
                    );
                    //@TODO messasge dropped
                }
            }

            //@TODO ... If the received message is greater than the sending rate, it is necessary to control the message receiving rate
            if !fut_senders.is_empty() {
                self.scx.stats.forwards.inc();
                let scx = self.scx.clone();
                let forwards_fut = async move {
                    let replys = futures::future::join_all(fut_senders).await;
                    for (node_id, reply) in replys {
                        if let Err(e) = reply {
                            log::warn!(
                                "forwards Message::ForwardsTo to other node, from: {:?}, to: {:?}, error: {:?}",
                                from,
                                node_id,
                                e
                            );
                            //@TODO messasge dropped
                        }
                    }
                    scx.stats.forwards.dec();
                };
                let _reply = self.scx.global_exec.spawn(forwards_fut).await;
            }
        }

        if errs.is_empty() {
            Ok(sub_client_ids)
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
    ) -> std::result::Result<(), Vec<(To, From, Publish, Reason)>> {
        self.inner.forwards_to(from, publish, relations).await
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
    fn iter(&self) -> Box<dyn Iterator<Item = Box<dyn Entry>> + Sync + Send + '_> {
        self.inner.iter()
    }

    #[inline]
    fn random_session(&self) -> Option<Session> {
        self.inner.random_session()
    }

    #[inline]
    async fn session_status(&self, client_id: &str) -> Option<SessionStatus> {
        let try_lock_timeout = self.router.try_lock_timeout;
        self.router.status(client_id).map(|s| SessionStatus {
            handshaking: s.handshaking(try_lock_timeout),
            id: s.id,
            online: s.online,
        })
    }

    #[inline]
    async fn client_states_count(&self) -> usize {
        self.router.states_count()
    }

    #[inline]
    fn sessions_count(&self) -> usize {
        self.inner.sessions_count()
    }

    #[inline]
    async fn query_subscriptions(&self, q: SubsSearchParams) -> Vec<SubsSearchResult> {
        self.inner.query_subscriptions(q).await
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

    #[inline]
    fn node_name(&self, id: NodeId) -> String {
        self.node_names.get(&id).cloned().unwrap_or_default()
    }

    #[inline]
    async fn check_health(&self) -> Result<HealthInfo> {
        let mut nodes_health_infos = Vec::new();
        let mailbox = self.router.raft_mailbox().await;
        let status = mailbox.status().await.map_err(Error::new)?;
        let mut leader_ids = HashSet::default();
        let (running, leader_id) = if status.available() { (true, status.leader_id) } else { (false, 0) };
        nodes_health_infos.push(NodeHealthStatus {
            node_id: status.id,
            running,
            leader_id: Some(leader_id),
            descr: None,
        });
        leader_ids.insert(leader_id);

        let data = RaftGrpcMessage::GetRaftStatus.encode()?;
        let replys = MessageBroadcaster::new(
            self.grpc_clients.clone(),
            self.message_type,
            Message::Data(data),
            Some(Duration::from_secs(10)),
        )
        .join_all()
        .await;

        for (node_id, reply) in replys {
            match reply {
                Ok(reply) => {
                    if let MessageReply::Data(data) = reply {
                        let RaftGrpcMessageReply::GetRaftStatus(o_status) =
                            RaftGrpcMessageReply::decode(&data)?;
                        let (running, leader_id) =
                            if o_status.available() { (true, o_status.leader_id) } else { (false, 0) };
                        nodes_health_infos.push(NodeHealthStatus {
                            node_id: o_status.id,
                            running,
                            leader_id: Some(leader_id),
                            descr: None,
                        });
                        leader_ids.insert(leader_id);
                    } else {
                        unreachable!()
                    }
                }
                Err(e) => {
                    log::error!("Get RaftGrpcMessage::GetRaftStatus from other node, error: {:?}", e);
                    nodes_health_infos.push(NodeHealthStatus {
                        node_id,
                        running: false,
                        leader_id: None,
                        descr: Some(e.to_string()),
                    });
                }
            }
        }
        let (running, descr) = if leader_ids.len() != 1 {
            (false, "Leader ID exception")
        } else if leader_ids.into_iter().next() == Some(0) {
            (false, "Leader does not exist")
        } else {
            (true, "Ok")
        };

        Ok(HealthInfo { running, descr: Some(descr.into()), nodes: nodes_health_infos })
    }

    #[inline]
    async fn health_status(&self) -> Result<NodeHealthStatus> {
        let mailbox = self.router.raft_mailbox().await;
        let status = mailbox.status().await.map_err(Error::new)?;
        Ok(NodeHealthStatus {
            node_id: status.id,
            running: status.leader_id > 0,
            leader_id: Some(status.leader_id),
            descr: None,
        })
    }

    #[inline]
    fn operation_is_busy(&self) -> bool {
        let exec = task_exec_queue();
        exec.active_count() > self.exec_queue_workers_busy_limit
            || exec.waiting_count() > self.exec_queue_busy_limit
    }
}
