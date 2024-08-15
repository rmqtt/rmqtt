use std::convert::From as _f;
use std::sync::Arc;
use std::time::Duration;

use once_cell::sync::OnceCell;

use crate::broker::session::{Session, SessionOfflineInfo};
use crate::broker::types::*;
use crate::grpc::{GrpcClients, MessageBroadcaster, MessageReply, MESSAGE_TYPE_MESSAGE_GET};
use crate::settings::listener::Listener;
use crate::stats::Counter;
use crate::{grpc, MqttError, Result, Runtime};

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

pub mod default;
pub mod error;
pub mod executor;
pub mod fitter;
pub mod hook;
pub mod inflight;
pub mod metrics;
pub mod queue;
pub mod retain;
pub mod session;
pub mod stats;
pub mod topic;
pub mod types;
pub mod v3;
pub mod v5;

#[async_trait]
pub trait Entry: Sync + Send {
    async fn try_lock(&self) -> Result<Box<dyn Entry>>;
    fn id(&self) -> Id;
    fn id_same(&self) -> Option<bool>;
    async fn set(&mut self, session: Session, tx: Tx) -> Result<()>;
    async fn remove(&mut self) -> Result<Option<(Session, Tx)>>;
    async fn remove_with(&mut self, id: &Id) -> Result<Option<(Session, Tx)>>;
    async fn kick(
        &mut self,
        clean_start: bool,
        clear_subscriptions: bool,
        is_admin: IsAdmin,
    ) -> Result<Option<SessionOfflineInfo>>;
    async fn online(&self) -> bool;
    async fn is_connected(&self) -> bool;
    fn session(&self) -> Option<Session>;
    fn exist(&self) -> bool;
    fn tx(&self) -> Option<Tx>;
    async fn subscribe(&self, subscribe: &Subscribe) -> Result<SubscribeReturn>;
    async fn unsubscribe(&self, unsubscribe: &Unsubscribe) -> Result<bool>;
    async fn publish(&self, from: From, p: Publish) -> Result<(), (From, Publish, Reason)>;
    async fn subscriptions(&self) -> Option<Vec<SubsSearchResult>>;
}

#[async_trait]
pub trait Shared: Sync + Send {
    /// Entry of the id
    fn entry(&self, id: Id) -> Box<dyn Entry>;

    /// Check if client_id exist
    fn exist(&self, client_id: &str) -> bool;

    /// Route and dispense publish message
    async fn forwards(
        &self,
        from: From,
        publish: Publish,
    ) -> Result<SubscriptionClientIds, Vec<(To, From, Publish, Reason)>>;

    ///Route and dispense publish message and return shared subscription relations
    async fn forwards_and_get_shareds(
        &self,
        from: From,
        publish: Publish,
    ) -> Result<(SubRelationsMap, SubscriptionClientIds), Vec<(To, From, Publish, Reason)>>;

    /// dispense publish message
    async fn forwards_to(
        &self,
        from: From,
        publish: &Publish,
        relations: SubRelations,
    ) -> Result<(), Vec<(To, From, Publish, Reason)>>;

    /// Iter
    fn iter(&self) -> Box<dyn Iterator<Item = Box<dyn Entry>> + Sync + Send>;

    /// choose a session if exist
    fn random_session(&self) -> Option<Session>;

    /// Get session status with client id specific
    async fn session_status(&self, client_id: &str) -> Option<SessionStatus>;

    /// Count of th client States
    async fn client_states_count(&self) -> usize;

    /// Sessions count
    fn sessions_count(&self) -> usize;

    /// Subscriptions from SubSearchParams
    async fn query_subscriptions(&self, q: SubsSearchParams) -> Vec<SubsSearchResult>;

    async fn subscriptions_count(&self) -> usize;

    ///This node is not included
    #[inline]
    fn get_grpc_clients(&self) -> GrpcClients {
        Arc::new(HashMap::default())
    }

    #[inline]
    fn node_name(&self, id: NodeId) -> String {
        format!("{}@127.0.0.1", id)
    }

    #[inline]
    async fn check_health(&self) -> Result<Option<serde_json::Value>> {
        Ok(Some(json!({"status": "Ok", "nodes": []})))
    }

    #[inline]
    async fn message_load(
        &self,
        client_id: &str,
        topic_filter: &str,
        group: Option<&SharedGroup>,
    ) -> Result<Vec<(MsgID, From, Publish)>> {
        let message_mgr = Runtime::instance().extends.message_mgr().await;
        if message_mgr.should_merge_on_get() {
            let mut msgs = message_mgr.get(client_id, topic_filter, group).await?;
            let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
            if !grpc_clients.is_empty() {
                let replys = MessageBroadcaster::new(
                    grpc_clients,
                    MESSAGE_TYPE_MESSAGE_GET,
                    grpc::Message::MessageGet(
                        ClientId::from(client_id),
                        TopicFilter::from(topic_filter),
                        group.cloned(),
                    ),
                )
                .join_all()
                .await;
                for (_, reply) in replys {
                    match reply? {
                        MessageReply::Error(e) => return Err(MqttError::Error(e)),
                        MessageReply::MessageGet(res) => {
                            msgs.extend(res.into_iter());
                        }
                        _ => {
                            unreachable!()
                        }
                    }
                }
            }
            Ok(msgs)
        } else {
            message_mgr.get(client_id, topic_filter, group).await
        }
    }
}

#[async_trait]
pub trait Router: Sync + Send {
    /// Id add with topic filter
    async fn add(&self, topic_filter: &str, id: Id, opts: SubscriptionOptions) -> Result<()>;

    /// Remove with id topic filter
    async fn remove(&self, topic_filter: &str, id: Id) -> Result<bool>;

    /// Match with id and topic
    async fn matches(&self, id: Id, topic: &TopicName) -> Result<SubRelationsMap>;

    ///Check online or offline
    #[inline]
    async fn is_online(&self, node_id: NodeId, client_id: &str) -> bool {
        Runtime::instance()
            .extends
            .shared()
            .await
            .entry(Id::from(node_id, ClientId::from(client_id)))
            .is_connected()
            .await
    }

    /// Gets by limit
    async fn gets(&self, limit: usize) -> Vec<Route>;

    /// Get by topic
    async fn get(&self, topic: &str) -> Result<Vec<Route>>;

    /// Topics tree
    async fn topics_tree(&self) -> usize;

    ///Return number of subscribed topics
    fn topics(&self) -> Counter;

    ///Returns the number of Subscription relationship
    fn routes(&self) -> Counter;

    /// merge Topics with another
    fn merge_topics(&self, topics_map: &HashMap<NodeId, Counter>) -> Counter;

    /// merge routes with another
    fn merge_routes(&self, routes_map: &HashMap<NodeId, Counter>) -> Counter;

    ///get topic tree
    async fn list_topics(&self, top: usize) -> Vec<String>;

    ///get subscription relations
    async fn list_relations(&self, top: usize) -> Vec<serde_json::Value>;

    ///all relations
    fn relations(&self) -> &AllRelationsMap;
}

#[async_trait]
pub trait SharedSubscription: Sync + Send {
    ///Whether shared subscriptions are supported
    #[inline]
    fn is_supported(&self, listen_cfg: &Listener) -> bool {
        listen_cfg.shared_subscription
    }

    ///Shared subscription strategy, select a subscriber, default is "random"
    #[inline]
    async fn choice(
        &self,
        ncs: &[(
            NodeId,
            ClientId,
            SubscriptionOptions,
            Option<Vec<SubscriptionIdentifier>>,
            Option<IsOnline>,
        )],
    ) -> Option<(usize, IsOnline)> {
        if ncs.is_empty() {
            return None;
        }

        let mut tmp_ncs = ncs
            .iter()
            .enumerate()
            .map(|(idx, (node_id, client_id, _, _, is_online))| (idx, node_id, client_id, is_online))
            .collect::<Vec<_>>();

        while !tmp_ncs.is_empty() {
            let r_idx = if tmp_ncs.len() == 1 { 0 } else { rand::random::<usize>() % tmp_ncs.len() };

            let (idx, node_id, client_id, is_online) = tmp_ncs.remove(r_idx);

            let is_online = if let Some(is_online) = is_online {
                *is_online
            } else {
                Runtime::instance().extends.router().await.is_online(*node_id, client_id).await
            };

            if is_online {
                return Some((idx, true));
            }

            if tmp_ncs.is_empty() {
                return Some((idx, is_online));
            }
        }
        return None;
    }
}

#[async_trait]
pub trait RetainStorage: Sync + Send {
    ///Whether retain is supported
    #[inline]
    fn is_supported(&self, listen_cfg: &Listener) -> bool {
        listen_cfg.retain_available
    }

    ///topic - concrete topic
    async fn set(&self, topic: &TopicName, retain: Retain, expiry_interval: Option<Duration>) -> Result<()>;

    ///topic_filter - Topic filter
    async fn get(&self, topic_filter: &TopicFilter) -> Result<Vec<(TopicName, Retain)>>;

    async fn count(&self) -> isize;

    async fn max(&self) -> isize;

    #[inline]
    fn stats_merge_mode(&self) -> StatsMergeMode {
        StatsMergeMode::None
    }
}

#[async_trait]
pub trait MessageManager: Sync + Send {
    #[inline]
    fn next_msg_id(&self) -> MsgID {
        0
    }

    ///Store messages
    ///
    ///_msg_id           - MsgID <br>
    ///_from             - From <br>
    ///_p                - Message <br>
    ///_expiry_interval  - Message expiration time <br>
    ///_sub_client_ids   - Indicate that certain subscribed clients have already forwarded the specified message.
    #[inline]
    async fn store(
        &self,
        _msg_id: MsgID,
        _from: From,
        _p: Publish,
        _expiry_interval: Duration,
        _sub_client_ids: Option<Vec<(ClientId, Option<(TopicFilter, SharedGroup)>)>>,
    ) -> Result<()> {
        Ok(())
    }

    #[inline]
    async fn get(
        &self,
        _client_id: &str,
        _topic_filter: &str,
        _group: Option<&SharedGroup>,
    ) -> Result<Vec<(MsgID, From, Publish)>> {
        Ok(Vec::new())
    }

    ///Indicate whether merging data from various nodes is needed during the 'get' operation.
    #[inline]
    fn should_merge_on_get(&self) -> bool {
        false
    }

    #[inline]
    async fn count(&self) -> isize {
        -1
    }

    #[inline]
    async fn max(&self) -> isize {
        -1
    }

    #[inline]
    fn enable(&self) -> bool {
        false
    }
}

pub struct DefaultMessageManager {}

impl DefaultMessageManager {
    #[inline]
    pub fn instance() -> &'static DefaultMessageManager {
        static INSTANCE: OnceCell<DefaultMessageManager> = OnceCell::new();
        INSTANCE.get_or_init(|| Self {})
    }
}

impl MessageManager for &'static DefaultMessageManager {}

#[async_trait]
pub trait DelayedSender: Sync + Send {
    ///Parse the topic and extract the delayed sending parameters.
    fn parse(&self, publish: Publish) -> Result<Publish>;

    ///Delayed publish
    async fn delay_publish(
        &self,
        from: From,
        publish: Publish,
        retain_available: bool,
        message_storage_available: bool,
        message_expiry_interval: Option<Duration>,
    ) -> Result<Option<(From, Publish)>>;

    ///Delayed message count
    async fn len(&self) -> usize;

    #[inline]
    async fn is_empty(&self) -> bool {
        self.len().await == 0
    }
}
