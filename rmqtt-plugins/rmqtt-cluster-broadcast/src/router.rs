use std::time::Duration;

use async_trait::async_trait;
use itertools::Itertools;

use rmqtt::context::ServerContext;
use rmqtt::types::{SubsSearchParams, SubsSearchResult};
use rmqtt::utils::Counter;
use rmqtt::{
    grpc::{GrpcClients, Message, MessageBroadcaster, MessageReply, MessageSender, MessageType},
    router::{DefaultRouter, Router},
    types::{
        AllRelationsMap, HashMap, Id, NodeId, Route, SubRelationsMap, SubscriptionOptions, TopicFilter,
        TopicName,
    },
    Result,
};

#[derive(Clone)]
pub(crate) struct ClusterRouter {
    inner: DefaultRouter,
    grpc_clients: GrpcClients,
    message_type: MessageType,
}

impl ClusterRouter {
    #[inline]
    pub(crate) fn new(scx: ServerContext, grpc_clients: GrpcClients, message_type: MessageType) -> Self {
        Self { inner: DefaultRouter::new(Some(scx)), grpc_clients, message_type }
    }

    #[inline]
    pub(crate) fn _inner(&self) -> &DefaultRouter {
        &self.inner
    }
}

#[async_trait]
impl Router for ClusterRouter {
    #[inline]
    async fn add(&self, topic_filter: &str, id: Id, opts: SubscriptionOptions) -> Result<()> {
        self.inner.add(topic_filter, id, opts).await
    }

    #[inline]
    async fn remove(&self, topic_filter: &str, id: Id) -> Result<bool> {
        self.inner.remove(topic_filter, id).await
    }

    #[inline]
    async fn matches(&self, id: Id, topic: &TopicName) -> Result<SubRelationsMap> {
        self.inner.matches(id, topic).await
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
                    Some(Duration::from_secs(10)),
                )
                .send()
                .await;
                match reply {
                    Ok(MessageReply::RoutesGet(ress)) => {
                        routes.extend(ress);
                    }
                    Err(e) => {
                        log::warn!("gets, error: {e:?}");
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
            Some(Duration::from_secs(10)),
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
    async fn query_subscriptions(&self, q: &SubsSearchParams) -> Vec<SubsSearchResult> {
        self.inner.query_subscriptions(q).await
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

    #[inline]
    fn relations(&self) -> &AllRelationsMap {
        &self.inner.relations
    }
}
