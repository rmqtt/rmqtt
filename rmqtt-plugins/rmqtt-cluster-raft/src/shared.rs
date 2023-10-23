use std::collections::HashSet;
use std::time::Duration;

use futures::future::FutureExt;
use once_cell::sync::OnceCell;

use rmqtt::anyhow::Error;
use rmqtt::broker::Router;
use rmqtt::grpc::MessageBroadcaster;
use rmqtt::serde_json::json;
use rmqtt::{anyhow, async_trait::async_trait, futures, log, once_cell, serde_json};
use rmqtt::{
    broker::{
        default::DefaultShared,
        session::{Session, SessionOfflineInfo},
        types::{
            From, Id, IsAdmin, NodeId, NodeName, Publish, Reason, SessionStatus, SubRelations,
            SubRelationsMap, SubsSearchParams, SubsSearchResult, Subscribe, SubscribeReturn,
            SubscriptionSize, To, Tx, Unsubscribe,
        },
        Entry, Shared,
    },
    grpc::{Message, MessageReply, MessageType},
    MqttError, Result, Runtime,
};

use super::message::{
    get_client_node_id, Message as RaftMessage, MessageReply as RaftMessageReply, RaftGrpcMessage,
    RaftGrpcMessageReply,
};
use super::{ClusterRouter, GrpcClients, HashMap, MessageSender, NodeGrpcClient};

pub struct ClusterLockEntry {
    inner: Box<dyn Entry>,
    cluster_shared: &'static ClusterShared,
    prev_node_id: Option<NodeId>,
}

impl ClusterLockEntry {
    #[inline]
    pub fn new(
        inner: Box<dyn Entry>,
        cluster_shared: &'static ClusterShared,
        prev_node_id: Option<NodeId>,
    ) -> Self {
        Self { inner, cluster_shared, prev_node_id }
    }
}

#[async_trait]
impl Entry for ClusterLockEntry {
    #[inline]
    async fn try_lock(&self) -> Result<Box<dyn Entry>> {
        let msg = RaftMessage::HandshakeTryLock { id: self.id() }.encode()?;
        let raft_mailbox = self.cluster_shared.router.raft_mailbox().await;
        let reply = raft_mailbox.send_proposal(msg).await.map_err(anyhow::Error::new)?;
        let mut prev_node_id = None;
        if !reply.is_empty() {
            match RaftMessageReply::decode(&reply)? {
                RaftMessageReply::Error(e) => {
                    return Err(MqttError::Msg(e));
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
        Ok(Box::new(ClusterLockEntry::new(self.inner.try_lock().await?, self.cluster_shared, prev_node_id)))
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
        let reply = raft_mailbox.send_proposal(msg).await.map_err(anyhow::Error::new)?;
        if !reply.is_empty() {
            let reply = RaftMessageReply::decode(&reply)?;
            match reply {
                RaftMessageReply::Error(e) => {
                    log::error!("RaftMessage::Connected reply: {:?}", e);
                    return Err(MqttError::Msg(e));
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
    ) -> Result<Option<SessionOfflineInfo>> {
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
                let mut msg_sender = MessageSender {
                    client,
                    msg_type: self.cluster_shared.message_type,
                    msg: Message::Kick(id.clone(), clean_start, true, is_admin), //clear_subscriptions
                    max_retries: 0,
                    retry_interval: Duration::from_millis(500),
                };
                match msg_sender.send().await {
                    Ok(reply) => {
                        if let MessageReply::Kick(Some(kicked)) = reply {
                            log::debug!("{:?} kicked: {:?}", id, kicked);
                            Ok(Some(kicked))
                        } else {
                            log::info!(
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
                return Err(MqttError::Msg(format!(
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
    async fn publish(&self, from: From, p: Publish) -> Result<(), (From, Publish, Reason)> {
        self.inner.publish(from, p).await
    }

    #[inline]
    async fn subscriptions(&self) -> Option<Vec<SubsSearchResult>> {
        let id = self.cluster_shared.router.id(&self.id().client_id)?;
        if id.node_id == Runtime::instance().node.id() {
            self.inner.subscriptions().await
        } else {
            //from other node
            if let Some(client) = self.cluster_shared.grpc_client(id.node_id) {
                let reply = MessageSender {
                    client,
                    msg_type: self.cluster_shared.message_type,
                    msg: Message::SubscriptionsGet(id.client_id.clone()),
                    max_retries: 0,
                    retry_interval: Duration::from_millis(500),
                }
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

pub struct ClusterShared {
    inner: &'static DefaultShared,
    router: &'static ClusterRouter,
    grpc_clients: GrpcClients,
    node_names: HashMap<NodeId, NodeName>,
    pub message_type: MessageType,
}

impl ClusterShared {
    #[inline]
    pub(crate) fn get_or_init(
        router: &'static ClusterRouter,
        grpc_clients: GrpcClients,
        node_names: HashMap<NodeId, NodeName>,
        message_type: MessageType,
    ) -> &'static ClusterShared {
        static INSTANCE: OnceCell<ClusterShared> = OnceCell::new();
        INSTANCE.get_or_init(|| Self {
            inner: DefaultShared::instance(),
            router,
            grpc_clients,
            node_names,
            message_type,
        })
    }

    #[inline]
    pub(crate) fn inner(&self) -> Box<dyn Shared> {
        Box::new(self.inner)
    }

    #[inline]
    pub(crate) fn grpc_client(&self, node_id: u64) -> Option<NodeGrpcClient> {
        self.grpc_clients.get(&node_id).map(|(_, c)| c.clone())
    }
}

#[async_trait]
impl Shared for &'static ClusterShared {
    #[inline]
    fn entry(&self, id: Id) -> Box<dyn Entry> {
        Box::new(ClusterLockEntry::new(self.inner.entry(id), self, None))
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
    ) -> Result<SubscriptionSize, Vec<(To, From, Publish, Reason)>> {
        log::debug!("[forwards] from: {:?}, publish: {:?}", from, publish);

        let topic = publish.topic();
        let mut relations_map = match Runtime::instance()
            .extends
            .router()
            .await
            .matches(from.id.clone(), publish.topic())
            .await
        {
            Ok(relations_map) => relations_map,
            Err(e) => {
                log::warn!("forwards, from:{:?}, topic:{:?}, error: {:?}", from, topic, e);
                SubRelationsMap::default()
            }
        };

        let subs_size: SubscriptionSize = relations_map.values().map(|subs| subs.len()).sum();

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
                if let Some(client) = self.grpc_client(node_id) {
                    let from = from.clone();
                    let publish = publish.clone();
                    let message_type = self.message_type;
                    let fut_sender = async move {
                        let mut msg_sender = MessageSender {
                            client,
                            msg_type: message_type,
                            msg: Message::ForwardsTo(from, publish, relations),
                            max_retries: 1,
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

            //@TODO ... If the received message is greater than the sending rate, it is necessary to control the message receiving rate
            if !fut_senders.is_empty() {
                Runtime::instance().stats.forwards.inc();
                let forwards_fut = async move {
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
                    Runtime::instance().stats.forwards.dec();
                };
                let _reply = Runtime::instance().exec.spawn(forwards_fut).await;
            }
        }

        if errs.is_empty() {
            Ok(subs_size)
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
    ) -> Result<(SubRelationsMap, SubscriptionSize), Vec<(To, From, Publish, Reason)>> {
        self.inner.forwards_and_get_shareds(from, publish).await
    }

    #[inline]
    fn iter(&self) -> Box<dyn Iterator<Item = Box<dyn Entry>> + Sync + Send> {
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
    async fn check_health(&self) -> Result<Option<serde_json::Value>> {
        let mut node_statuses = Vec::new();
        let mailbox = self.router.raft_mailbox().await;
        let status = mailbox.status().await.map_err(Error::new)?;
        let mut leader_ids = HashSet::new();
        node_statuses.push(json!({
            "node_id": status.id,
            "leader_id": status.leader_id,
        }));
        leader_ids.insert(status.leader_id);

        let data = RaftGrpcMessage::GetRaftStatus.encode()?;
        let replys =
            MessageBroadcaster::new(self.grpc_clients.clone(), self.message_type, Message::Data(data))
                .join_all()
                .await;

        for (node_id, reply) in replys {
            match reply {
                Ok(reply) => {
                    if let MessageReply::Data(data) = reply {
                        let RaftGrpcMessageReply::GetRaftStatus(o_status) =
                            RaftGrpcMessageReply::decode(&data)?;
                        node_statuses.push(json!({
                            "node_id": o_status.id,
                            "leader_id": o_status.leader_id,
                        }));
                        leader_ids.insert(o_status.leader_id);
                    } else {
                        unreachable!()
                    }
                }
                Err(e) => {
                    log::error!("Get RaftGrpcMessage::GetRaftStatus from other node, error: {:?}", e);
                    node_statuses.push(json!({
                        "node_id": node_id,
                        "error": e.to_string(),
                    }));
                }
            }
        }

        let status = if leader_ids.len() != 1 {
            "Leader ID exception"
        } else if leader_ids.into_iter().next() == Some(0) {
            "Leader does not exist"
        } else {
            "Ok"
        };

        Ok(Some(json!({
            "status": status,
            "nodes": node_statuses,
        })))
    }
}
