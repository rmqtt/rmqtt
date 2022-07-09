use rmqtt::{
    broker::{
        hook::{Handler, HookResult, Parameter, ReturnType},
        SubRelationsMap,
        types::{From, Publish},
    },
    grpc::{Message, MessageReply},
    Runtime,
};

use super::{hook_message_dropped, retainer::ClusterRetainer, shared::ClusterShared};

pub(crate) struct HookHandler {
    shared: &'static ClusterShared,
    retainer: &'static ClusterRetainer,
}

impl HookHandler {
    pub(crate) fn new(shared: &'static ClusterShared, retainer: &'static ClusterRetainer) -> Self {
        Self { shared, retainer }
    }
}

#[async_trait]
impl Handler for HookHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::GrpcMessageReceived(typ, msg) => {
                log::debug!("GrpcMessageReceived, type: {}, msg: {:?}", typ, msg);
                if self.shared.message_type != *typ {
                    return (true, acc);
                }
                match msg {
                    Message::Forwards(from, publish) => {
                        let shared_subs = forwards(from.clone(), publish.clone()).await;
                        let new_acc = HookResult::GrpcMessageReply(Ok(MessageReply::Forwards(shared_subs)));
                        return (false, Some(new_acc));
                    }
                    Message::ForwardsTo(from, publish, sub_rels) => {
                        if let Err(droppeds) =
                        self.shared.inner().forwards_to(from.clone(), publish, sub_rels.clone()).await
                        {
                            hook_message_dropped(droppeds).await;
                        }
                        return (false, acc);
                    }
                    Message::Kick(id, clear_subscriptions, is_admin) => {
                        let entry = self.shared.inner().entry(id.clone());
                        log::debug!("{:?}", id);
                        let new_acc = match entry.try_lock().await {
                            Ok(mut entry) => match entry.kick(*clear_subscriptions, *is_admin).await {
                                Ok(o) => {
                                    log::debug!("{:?} offline info: {:?}", id, o);
                                    HookResult::GrpcMessageReply(Ok(MessageReply::Kick(o)))
                                }
                                Err(e) => HookResult::GrpcMessageReply(Err(e)),
                            },
                            Err(e) => {
                                log::warn!("{:?}, try_lock error, {:?}", id, e);
                                HookResult::GrpcMessageReply(Err(e))
                            }
                        };
                        return (false, Some(new_acc));
                    }
                    Message::NumberOfClients => {
                        let new_acc = HookResult::GrpcMessageReply(Ok(MessageReply::NumberOfClients(
                            self.shared.inner().clients().await,
                        )));
                        return (false, Some(new_acc));
                    }
                    Message::NumberOfSessions => {
                        let new_acc = HookResult::GrpcMessageReply(Ok(MessageReply::NumberOfSessions(
                            self.shared.inner().sessions().await,
                        )));
                        return (false, Some(new_acc));
                    }
                    Message::GetRetains(topic_filter) => {
                        let new_acc = match self.retainer.inner().get(topic_filter).await {
                            Ok(retains) => {
                                HookResult::GrpcMessageReply(Ok(MessageReply::GetRetains(retains)))
                            }
                            Err(e) => HookResult::GrpcMessageReply(Err(e)),
                        };
                        return (false, Some(new_acc));
                    }
                    _ => {
                        log::error!("unimplemented, {:?}", param)
                    }
                }
            }
            _ => {
                log::error!("unimplemented, {:?}", param)
            }
        }
        (true, acc)
    }
}

async fn forwards(from: From, publish: Publish) -> SubRelationsMap {
    log::debug!("forwards, From: {:?}, publish: {:?}", from, publish);
    match Runtime::instance().extends.shared().await.forwards_and_get_shareds(from, publish).await {
        Err(droppeds) => {
            hook_message_dropped(droppeds).await;
            SubRelationsMap::default()
        }
        Ok(relations_map) => relations_map,
    }
}
