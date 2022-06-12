use rmqtt_raft::Mailbox;

use rmqtt::{
    broker::hook::{Handler, HookResult, Parameter, ReturnType},
    grpc::{Message as GrpcMessage, MessageReply},
};
use rmqtt::broker::Shared;

use super::{hook_message_dropped, retainer::ClusterRetainer, shared::ClusterShared};
use super::config::{BACKOFF_STRATEGY, retry};
use super::message::Message;

pub(crate) struct HookHandler {
    shared: &'static ClusterShared,
    retainer: &'static ClusterRetainer,
    raft_mailbox: Mailbox,
}

impl HookHandler {
    pub(crate) fn new(
        shared: &'static ClusterShared,
        retainer: &'static ClusterRetainer,
        raft_mailbox: Mailbox,
    ) -> Self {
        Self { shared, retainer, raft_mailbox }
    }
}

#[async_trait]
impl Handler for HookHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        log::debug!("hook, Parameter type: {:?}", param.get_type());
        match param {
            Parameter::ClientDisconnected(_s, c, r) => {
                log::debug!("{:?} hook::ClientDisconnected reason: {:?}", c.id, r);
                if !r.contains("Kicked") {
                    let msg = Message::Disconnected { id: c.id.clone() }.encode().unwrap();
                    let raft_mailbox = self.raft_mailbox.clone();
                    tokio::spawn(async move {
                        if let Err(e) = retry(BACKOFF_STRATEGY.clone(), || async {
                            Ok(raft_mailbox.send(msg.clone()).await?)
                        }).await {
                            log::warn!("HookHandler, Message::Disconnected, raft mailbox send error, {:?}", e);
                        }
                    });
                }
            }

            Parameter::SessionTerminated(_s, c, _r) => {
                let msg = Message::SessionTerminated { id: c.id.clone() }.encode().unwrap();
                let raft_mailbox = self.raft_mailbox.clone();
                tokio::spawn(async move {
                    if let Err(e) = retry(BACKOFF_STRATEGY.clone(), || async {
                        Ok(raft_mailbox.send(msg.clone()).await?)
                    }).await {
                        log::warn!("HookHandler, Message::SessionTerminated, raft mailbox send error, {:?}", e);
                    }
                });
            }

            Parameter::GrpcMessageReceived(typ, msg) => {
                log::debug!("GrpcMessageReceived, type: {}, msg: {:?}", typ, msg);
                if self.shared.message_type != *typ {
                    return (true, acc);
                }
                match msg {
                    GrpcMessage::ForwardsTo(from, publish, sub_rels) => {
                        if let Err(droppeds) =
                        self.shared.forwards_to(from.clone(), publish, sub_rels.clone()).await
                        {
                            hook_message_dropped(droppeds).await;
                        }
                        return (false, acc);
                    }
                    GrpcMessage::Kick(id, clear_subscriptions) => {
                        let mut entry = self.shared.inner().entry(id.clone());
                        let new_acc = match entry.kick(*clear_subscriptions).await {
                            Ok(o) => HookResult::GrpcMessageReply(Ok(MessageReply::Kick(o))),
                            Err(e) => HookResult::GrpcMessageReply(Err(e)),
                        };
                        return (false, Some(new_acc));
                    }
                    GrpcMessage::GetRetains(topic_filter) => {
                        log::debug!("[GrpcMessage::GetRetains] topic_filter: {:?}", topic_filter);
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

// async fn forwards(from: From, publish: Publish) -> SharedSubRelations {
//     match Runtime::instance().extends.shared().await.forwards_and_get_shareds(from, publish).await {
//         Err(droppeds) => {
//             hook_message_dropped(droppeds).await;
//             SharedSubRelations::default()
//         }
//         Ok(shared_subs) => shared_subs,
//     }
// }
