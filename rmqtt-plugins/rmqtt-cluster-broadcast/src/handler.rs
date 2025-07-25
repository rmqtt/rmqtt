use async_trait::async_trait;

use crate::message::{BroadcastGrpcMessage, BroadcastGrpcMessageReply};
use rmqtt::context::ServerContext;
use rmqtt::router::Router;
use rmqtt::shared::Shared;
use rmqtt::types::Id;
use rmqtt::{
    grpc::{Message, MessageReply},
    hook::{Handler, HookResult, Parameter, ReturnType},
    types::{From, Publish, SubRelationsMap, SubscriptionClientIds},
};

use super::{hook_message_dropped, router::ClusterRouter, shared::ClusterShared};

pub(crate) struct HookHandler {
    scx: ServerContext,
    shared: ClusterShared,
    router: ClusterRouter,
}

impl HookHandler {
    pub(crate) fn new(scx: ServerContext, shared: ClusterShared, router: ClusterRouter) -> Self {
        Self { scx, shared, router }
    }
}

#[async_trait]
impl Handler for HookHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::GrpcMessageReceived(typ, msg) => {
                log::debug!("GrpcMessageReceived, type: {typ}, msg: {msg:?}");
                if self.shared.message_type != *typ {
                    return (true, acc);
                }
                match msg {
                    Message::Forwards(from, publish) => {
                        let (shared_subs, subs_size) =
                            forwards(&self.scx, from.clone(), publish.clone()).await;
                        let new_acc =
                            HookResult::GrpcMessageReply(Ok(MessageReply::Forwards(shared_subs, subs_size)));
                        return (false, Some(new_acc));
                    }
                    Message::ForwardsTo(from, publish, sub_rels) => {
                        if let Err(droppeds) =
                            self.shared.inner().forwards_to(from.clone(), publish, sub_rels.clone()).await
                        {
                            hook_message_dropped(&self.scx, droppeds).await;
                        }
                        return (false, acc);
                    }
                    Message::Kick(id, clean_start, clear_subscriptions, is_admin) => {
                        let entry = self.shared.inner().entry(id.clone());
                        log::debug!("{id:?}");
                        let new_acc = match entry.try_lock().await {
                            Ok(mut entry) => {
                                match entry.kick(*clean_start, *clear_subscriptions, *is_admin).await {
                                    Ok(o) => {
                                        log::debug!("{id:?} offline info: {o:?}");
                                        HookResult::GrpcMessageReply(Ok(MessageReply::Kick(o)))
                                    }
                                    Err(e) => HookResult::GrpcMessageReply(Err(e)),
                                }
                            }
                            Err(e) => {
                                log::warn!("{id:?}, try_lock error, {e:?}");
                                HookResult::GrpcMessageReply(Err(e))
                            }
                        };
                        return (false, Some(new_acc));
                    }
                    Message::NumberOfClients => {
                        let new_acc = HookResult::GrpcMessageReply(Ok(MessageReply::NumberOfClients(
                            //self.shared.inner().clients().await,
                            self.scx.stats.connections.count() as usize,
                        )));
                        return (false, Some(new_acc));
                    }
                    Message::NumberOfSessions => {
                        let new_acc = HookResult::GrpcMessageReply(Ok(MessageReply::NumberOfSessions(
                            //self.shared.inner().sessions().await,
                            self.scx.stats.sessions.count() as usize,
                        )));
                        return (false, Some(new_acc));
                    }
                    Message::GetRetains(topic_filter) => {
                        log::error!("unreachable!(), Message::GetRetains({topic_filter})");
                    }
                    Message::Online(clientid) => {
                        let new_acc = HookResult::GrpcMessageReply(Ok(MessageReply::Online(
                            self.scx.extends.router().await.is_online(self.scx.node.id(), clientid).await,
                        )));
                        return (false, Some(new_acc));
                    }
                    Message::SubscriptionsSearch(q) => {
                        let new_acc = HookResult::GrpcMessageReply(Ok(MessageReply::SubscriptionsSearch(
                            self.shared.inner().query_subscriptions(q).await,
                        )));
                        return (false, Some(new_acc));
                    }
                    Message::SubscriptionsGet(clientid) => {
                        let id = Id::from(self.scx.node.id(), clientid.clone());
                        let entry = self.shared.inner().entry(id);
                        let new_acc = HookResult::GrpcMessageReply(Ok(MessageReply::SubscriptionsGet(
                            entry.subscriptions().await,
                        )));
                        return (false, Some(new_acc));
                    }
                    Message::RoutesGet(limit) => {
                        let new_acc = HookResult::GrpcMessageReply(Ok(MessageReply::RoutesGet(
                            self.router._inner().gets(*limit).await,
                        )));
                        return (false, Some(new_acc));
                    }
                    Message::RoutesGetBy(topic) => {
                        let routes = match self.router._inner()._get_routes(topic).await {
                            Ok(routes) => Ok(MessageReply::RoutesGetBy(routes)),
                            Err(e) => Err(e),
                        };
                        let new_acc = HookResult::GrpcMessageReply(routes);
                        return (false, Some(new_acc));
                    }
                    Message::SessionStatus(clientid) => {
                        let status = self.shared.inner().session_status(clientid).await;
                        let new_acc = HookResult::GrpcMessageReply(Ok(MessageReply::SessionStatus(status)));
                        return (false, Some(new_acc));
                    }
                    Message::Data(data) => {
                        let new_acc = match BroadcastGrpcMessage::decode(data) {
                            Err(e) => {
                                log::error!("Message::decode, error: {e:?}");
                                HookResult::GrpcMessageReply(Ok(MessageReply::Error(e.to_string())))
                            }
                            Ok(BroadcastGrpcMessage::GetNodeHealthStatus) => {
                                match self.shared.health_status().await {
                                    Ok(status) => {
                                        match BroadcastGrpcMessageReply::GetNodeHealthStatus(status).encode()
                                        {
                                            Ok(ress) => {
                                                HookResult::GrpcMessageReply(Ok(MessageReply::Data(ress)))
                                            }
                                            Err(e) => HookResult::GrpcMessageReply(Ok(MessageReply::Error(
                                                e.to_string(),
                                            ))),
                                        }
                                    }
                                    Err(e) => {
                                        HookResult::GrpcMessageReply(Ok(MessageReply::Error(e.to_string())))
                                    }
                                }
                            }
                        };
                        return (false, Some(new_acc));
                    }
                    _ => {
                        log::error!("unimplemented, {param:?}")
                    }
                }
            }
            _ => {
                log::error!("unimplemented, {param:?}")
            }
        }
        (true, acc)
    }
}

async fn forwards(
    scx: &ServerContext,
    from: From,
    publish: Publish,
) -> (SubRelationsMap, SubscriptionClientIds) {
    log::debug!("forwards, From: {from:?}, publish: {publish:?}");
    match scx.extends.shared().await.forwards_and_get_shareds(from, publish).await {
        Err(droppeds) => {
            hook_message_dropped(scx, droppeds).await;
            (SubRelationsMap::default(), None)
        }
        Ok(relations_map) => relations_map,
    }
}
