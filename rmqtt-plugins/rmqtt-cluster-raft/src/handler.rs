use rmqtt_raft::Mailbox;

use rmqtt::broker::Shared;
use rmqtt::rust_box::task_exec_queue::SpawnExt;
use rmqtt::{async_trait::async_trait, log, tokio, MqttError};
use rmqtt::{
    broker::hook::{Handler, HookResult, Parameter, ReturnType},
    grpc::{Message as GrpcMessage, MessageReply},
    Id, Runtime,
};

use super::config::{retry, BACKOFF_STRATEGY};
use super::message::{Message, RaftGrpcMessage, RaftGrpcMessageReply};
use super::{hook_message_dropped, retainer::ClusterRetainer, shared::ClusterShared, task_exec_queue};

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
            Parameter::ClientDisconnected(s, r) => {
                log::debug!("{:?} hook::ClientDisconnected reason: {:?}", s.id, r);
                if !r.is_kicked(false) {
                    let msg = Message::Disconnected { id: s.id.clone() }.encode().unwrap();
                    let raft_mailbox = self.raft_mailbox.clone();
                    tokio::spawn(async move {
                        if let Err(e) = retry(BACKOFF_STRATEGY.clone(), || async {
                            let msg = msg.clone();
                            let mailbox = raft_mailbox.clone();
                            let res = async move { mailbox.send_proposal(msg).await }
                                .spawn(task_exec_queue())
                                .result()
                                .await
                                .map_err(|_| {
                                    MqttError::from(
                                        "Handler::hook(Message::Disconnected), task execution failure",
                                    )
                                })?
                                .map_err(|e| MqttError::from(e.to_string()))?;
                            Ok(res)
                        })
                        .await
                        {
                            log::warn!(
                                "HookHandler, Message::Disconnected, raft mailbox send error, {:?}",
                                e
                            );
                        }
                    });
                }
            }

            Parameter::SessionTerminated(s, _r) => {
                let msg = Message::SessionTerminated { id: s.id.clone() }.encode().unwrap();
                let raft_mailbox = self.raft_mailbox.clone();
                tokio::spawn(async move {
                    if let Err(e) = retry(BACKOFF_STRATEGY.clone(), || async {
                        let msg = msg.clone();
                        let mailbox = raft_mailbox.clone();
                        let res = async move { mailbox.send_proposal(msg).await }
                            .spawn(task_exec_queue())
                            .result()
                            .await
                            .map_err(|_| {
                                MqttError::from(
                                    "Handler::hook(Message::SessionTerminated), task execution failure",
                                )
                            })?
                            .map_err(|e| MqttError::from(e.to_string()))?;
                        Ok(res)
                    })
                    .await
                    {
                        log::warn!(
                            "HookHandler, Message::SessionTerminated, raft mailbox send error, {:?}",
                            e
                        );
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
                    GrpcMessage::Kick(id, clean_start, clear_subscriptions, is_admin) => {
                        let mut entry = self.shared.inner().entry(id.clone());
                        let new_acc = match entry.kick(*clean_start, *clear_subscriptions, *is_admin).await {
                            Ok(Some(o)) => HookResult::GrpcMessageReply(Ok(MessageReply::Kick(Some(o)))),
                            Ok(None) => HookResult::GrpcMessageReply(Ok(MessageReply::Kick(None))),
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
                    GrpcMessage::SubscriptionsGet(clientid) => {
                        let id = Id::from(Runtime::instance().node.id(), clientid.clone());
                        let entry = self.shared.inner().entry(id);
                        let new_acc = HookResult::GrpcMessageReply(Ok(MessageReply::SubscriptionsGet(
                            entry.subscriptions().await,
                        )));
                        return (false, Some(new_acc));
                    }
                    GrpcMessage::Data(data) => {
                        let new_acc = match RaftGrpcMessage::decode(data) {
                            Err(e) => {
                                log::error!("Message::decode, error: {:?}", e);
                                HookResult::GrpcMessageReply(Ok(MessageReply::Error(e.to_string())))
                            }
                            Ok(RaftGrpcMessage::GetRaftStatus) => {
                                let raft_mailbox = self.raft_mailbox.clone();
                                match raft_mailbox.status().await {
                                    Ok(status) => {
                                        match RaftGrpcMessageReply::GetRaftStatus(status).encode() {
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
