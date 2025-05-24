use anyhow::anyhow;
use async_trait::async_trait;
use rust_box::task_exec_queue::{SpawnExt, TaskExecQueue};

use rmqtt::context::ServerContext;
use rmqtt::{
    grpc::{Message as GrpcMessage, MessageReply},
    hook::{Handler, HookResult, Parameter, ReturnType},
    shared::Shared,
    types::Id,
};
use rmqtt_raft::Mailbox;

use super::config::{retry, BACKOFF_STRATEGY};
use super::message::{Message, RaftGrpcMessage, RaftGrpcMessageReply};
use super::{hook_message_dropped, shared::ClusterShared};

pub(crate) struct HookHandler {
    scx: ServerContext,
    exec: TaskExecQueue,
    shared: ClusterShared,
    raft_mailbox: Mailbox,
}

impl HookHandler {
    pub(crate) fn new(
        scx: ServerContext,
        exec: TaskExecQueue,
        shared: ClusterShared,
        raft_mailbox: Mailbox,
    ) -> Self {
        Self { scx, exec, shared, raft_mailbox }
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
                    let msg = Message::Disconnected { id: s.id.clone() }.encode();
                    match msg {
                        Err(e) => {
                            log::warn!("HookHandler, Message::Disconnected, message encode error, {:?}", e);
                        }
                        Ok(msg) => {
                            let raft_mailbox = self.raft_mailbox.clone();
                            let exec = self.exec.clone();
                            tokio::spawn(async move {
                                if let Err(e) = retry(BACKOFF_STRATEGY.clone(), || async {
                                    let msg = msg.clone();
                                    let mailbox = raft_mailbox.clone();
                                    let res = async move { mailbox.send_proposal(msg).await }
                                        .spawn(&exec)
                                        .result()
                                        .await
                                        .map_err(|_| {
                                             anyhow!(
                                                "Handler::hook(Message::Disconnected), task execution failure",
                                            )
                                        })?
                                        .map_err(|e|  anyhow!(e.to_string()))?;
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
                    };
                }
            }

            Parameter::SessionTerminated(s, _r) => {
                let msg = Message::SessionTerminated { id: s.id.clone() }.encode();
                match msg {
                    Err(e) => {
                        log::warn!("HookHandler, Message::SessionTerminated, message encode error, {:?}", e);
                    }
                    Ok(msg) => {
                        let raft_mailbox = self.raft_mailbox.clone();
                        let exec = self.exec.clone();
                        tokio::spawn(async move {
                            if let Err(e) = retry(BACKOFF_STRATEGY.clone(), || async {
                                let msg = msg.clone();
                                let mailbox = raft_mailbox.clone();
                                let res = async move { mailbox.send_proposal(msg).await }
                                    .spawn(&exec)
                                    .result()
                                    .await
                                    .map_err(|_| {
                                         anyhow!(
                                            "Handler::hook(Message::SessionTerminated), task execution failure",
                                        )
                                    })?
                                    .map_err(|e|  anyhow!(e.to_string()))?;
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
                }
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
                            hook_message_dropped(&self.scx, droppeds).await;
                        }
                        return (false, acc);
                    }
                    GrpcMessage::Kick(id, clean_start, clear_subscriptions, is_admin) => {
                        let mut entry = self.shared.inner().entry(id.clone());
                        let new_acc = match entry.kick(*clean_start, *clear_subscriptions, *is_admin).await {
                            Ok(o) => HookResult::GrpcMessageReply(Ok(MessageReply::Kick(o))),
                            Err(e) => HookResult::GrpcMessageReply(Err(e)),
                        };
                        return (false, Some(new_acc));
                    }
                    GrpcMessage::GetRetains(topic_filter) => {
                        log::debug!("[GrpcMessage::GetRetains] topic_filter: {:?}", topic_filter);
                        unreachable!()
                    }
                    GrpcMessage::SubscriptionsGet(clientid) => {
                        let id = Id::from(self.scx.node.id(), clientid.clone());
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
