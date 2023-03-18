use itertools::Itertools;
use once_cell::sync::OnceCell;

use rmqtt::stats::Counter;
use rmqtt::{async_trait::async_trait, itertools, log, once_cell, serde_json};
use rmqtt::{
    broker::{
        default::DefaultRouter,
        types::{Id, NodeId, QoS, Route, SharedGroup, TopicName},
        Router, SubRelationsMap,
    },
    grpc::{GrpcClients, Message, MessageBroadcaster, MessageReply, MessageSender, MessageType},
    HashMap, Result, TopicFilter,
};

pub(crate) struct ClusterRouter {
    inner: &'static DefaultRouter,
    grpc_clients: GrpcClients,
    message_type: MessageType,
}

impl ClusterRouter {
    #[inline]
    pub(crate) fn get_or_init(grpc_clients: GrpcClients, message_type: MessageType) -> &'static Self {
        static INSTANCE: OnceCell<ClusterRouter> = OnceCell::new();
        INSTANCE.get_or_init(|| Self { inner: DefaultRouter::instance(), grpc_clients, message_type })
    }

    #[inline]
    pub(crate) fn _inner(&self) -> &'static DefaultRouter {
        self.inner
    }
}

#[async_trait]
impl Router for &'static ClusterRouter {
    #[inline]
    async fn add(
        &self,
        topic_filter: &str,
        id: Id,
        qos: QoS,
        shared_group: Option<SharedGroup>,
    ) -> Result<()> {
        self.inner.add(topic_filter, id, qos, shared_group).await
    }

    #[inline]
    async fn remove(&self, topic_filter: &str, id: Id) -> Result<bool> {
        self.inner.remove(topic_filter, id).await
    }

    #[inline]
    async fn matches(&self, topic: &TopicName) -> Result<SubRelationsMap> {
        self.inner.matches(topic).await
    }

    ///Check online or offline
    async fn is_online(&self, node_id: NodeId, client_id: &str) -> bool {
        self.inner.is_online(node_id, client_id).await
    }

    #[inline]
    async fn gets(&self, limit: usize) -> Vec<Route> {
        let mut routes = self.inner.gets(limit).await;
        for (_id, (_addr, c)) in self.grpc_clients.iter() {
            if routes.len() < limit {
                let reply = MessageSender::new(
                    c.clone(),
                    self.message_type,
                    Message::RoutesGet(limit - routes.len()),
                )
                .send()
                .await;
                match reply {
                    Ok(MessageReply::RoutesGet(ress)) => {
                        routes.extend(ress);
                    }
                    Err(e) => {
                        log::warn!("gets, error: {:?}", e);
                    }
                    _ => unreachable!(),
                };
            } else {
                break;
            }
        }
        routes
    }

    #[inline]
    async fn get(&self, topic: &str) -> Result<Vec<Route>> {
        let routes = self.inner._get_routes(topic).await?;

        let mut replys = MessageBroadcaster::new(
            self.grpc_clients.clone(),
            self.message_type,
            Message::RoutesGetBy(TopicFilter::from(topic)),
        )
        .join_all()
        .await
        .into_iter()
        .map(|(_, replys)| match replys {
            Ok(MessageReply::RoutesGetBy(routes)) => Ok(routes),
            Ok(_) => unreachable!(),
            Err(e) => Err(e),
        })
        .collect::<Result<Vec<_>>>()?;
        replys.push(routes);
        Ok(replys.into_iter().flatten().unique().collect())
    }

    #[inline]
    async fn topics_tree(&self) -> usize {
        self.inner.topics_tree().await
    }

    #[inline]
    fn topics(&self) -> Counter {
        self.inner.topics()
    }

    #[inline]
    fn routes(&self) -> Counter {
        self.inner.routes()
    }

    #[inline]
    fn merge_topics(&self, topics_map: &HashMap<NodeId, Counter>) -> Counter {
        let topics = Counter::new();
        for (_, counter) in topics_map.iter() {
            topics.add(counter);
        }
        topics
    }

    #[inline]
    fn merge_routes(&self, routes_map: &HashMap<NodeId, Counter>) -> Counter {
        let routes = Counter::new();
        for (_, counter) in routes_map.iter() {
            routes.add(counter);
        }
        routes
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
