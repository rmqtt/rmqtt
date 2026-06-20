//! Hook handlers for the Raft cluster plugin.
//!
//! Processes cluster-related MQTT hook events and translates them
//! into Raft proposals for distributed state replication.
//!
//! # Hook Events Handled
//!
//! - `ClientDisconnected`: Proposes a `Disconnected` message to Raft.
//! - `SessionTerminated`: Proposes a `SessionTerminated` message to Raft.
//! - `GrpcMessageReceived`: Processes inter-node gRPC messages
//!   including message forwarding, client kick, and health queries.
//!
use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use backoff::{future::retry, ExponentialBackoff};
use rust_box::task_exec_queue::{SpawnExt, TaskExecQueue};

use rmqtt::context::ServerContext;
use rmqtt::{
    grpc::{Message as GrpcMessage, MessageReply},
    hook::{Handler, HookResult, Parameter, ReturnType},
    shared::Shared,
    types::Id,
};
use rmqtt_raft::Mailbox;

use super::config::PluginConfig;
use super::message::{Message, RaftGrpcMessage, RaftGrpcMessageReply};
use super::{hook_message_dropped, shared::ClusterShared};

/// Handles cluster-related hook events by translating them into
/// Raft proposals or processing inter-node gRPC messages.
///
/// Uses exponential backoff retry for Raft proposals to handle
/// transient leader election or network partitions.
pub(crate) struct HookHandler {
    scx: ServerContext,
    exec: TaskExecQueue,
    backoff_strategy: ExponentialBackoff,
    shared: ClusterShared,
    raft_mailbox: Mailbox,
}

impl HookHandler {
    pub(crate) fn new(
        scx: ServerContext,
        cfg: Arc<PluginConfig>,
        backoff_strategy: ExponentialBackoff,
        shared: ClusterShared,
        raft_mailbox: Mailbox,
    ) -> Self {
        let exec =
            scx.get_exec(("RAFT_DISCONNECTED_EXEC", cfg.task_exec_queue_workers, cfg.task_exec_queue_max));
        Self { scx, exec, backoff_strategy, shared, raft_mailbox }
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
                            log::warn!("HookHandler, Message::Disconnected, message encode error, {e:?}");
                        }
                        Ok(msg) => {
                            let raft_mailbox = self.raft_mailbox.clone();
                            let exec = self.exec.clone();
                            let backoff_strategy = self.backoff_strategy.clone();
                            tokio::spawn(async move {
                                if let Err(e) = retry(backoff_strategy, || async {
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
                                        "HookHandler, Message::Disconnected, raft mailbox send error, {e:?}"
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
                        log::warn!("HookHandler, Message::SessionTerminated, message encode error, {e:?}");
                    }
                    Ok(msg) => {
                        let raft_mailbox = self.raft_mailbox.clone();
                        let exec = self.exec.clone();
                        let backoff_strategy = self.backoff_strategy.clone();
                        tokio::spawn(async move {
                            if let Err(e) = retry(backoff_strategy, || async {
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
                                    "HookHandler, Message::SessionTerminated, raft mailbox send error, {e:?}"
                                );
                            }
                        });
                    }
                }
            }

            Parameter::GrpcMessageReceived(typ, msg) => {
                log::debug!("GrpcMessageReceived, type: {typ}, msg: {msg:?}");
                if self.shared.message_type != *typ {
                    return (true, acc);
                }
                match msg {
                    GrpcMessage::ForwardsTo(from, publish, sub_rels, msg_id) => {
                        if let Err((_, droppeds)) =
                            self.shared.forwards_to(from.clone(), publish, sub_rels.clone(), *msg_id).await
                        {
                            hook_message_dropped(&self.scx, droppeds).await;
                        }
                        return (false, acc);
                    }
                    GrpcMessage::ForwardsToAck(msg_id, node_id, subscribers) => {
                        log::debug!(
                            "ForwardsToAck received: msg_id={:?}, node_id={}, subscribers={:?}",
                            msg_id,
                            node_id,
                            subscribers
                        );
                        if let Err(e) = self
                            .scx
                            .extends
                            .message_mgr()
                            .await
                            .mark_forwarded(*msg_id, subscribers.clone())
                            .await
                        {
                            log::warn!("mark_forwarded error, {e:?}, msg_id: {msg_id}, from node: {node_id}, subscribers: {subscribers:?}");
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
                        let retains = self.scx.extends.retain().await.get(topic_filter).await;
                        let new_acc = match retains {
                            Err(e) => HookResult::GrpcMessageReply(Err(e)),
                            Ok(retains) => {
                                HookResult::GrpcMessageReply(Ok(MessageReply::GetRetains(retains)))
                            }
                        };
                        return (false, Some(new_acc));
                    }
                    GrpcMessage::SubscriptionsGet(clientid) => {
                        let id = Id::from(self.scx.node.id(), clientid.clone());
                        let entry = self.shared.inner().entry(id);
                        let new_acc = HookResult::GrpcMessageReply(Ok(MessageReply::SubscriptionsGet(
                            entry.subscriptions().await,
                        )));
                        return (false, Some(new_acc));
                    }
                    GrpcMessage::MessageGet(client_id, topic_filter, group) => {
                        let msgs = self
                            .scx
                            .extends
                            .message_mgr()
                            .await
                            .get(client_id, topic_filter, group.as_ref())
                            .await;
                        let new_acc = match msgs {
                            Err(e) => HookResult::GrpcMessageReply(Err(e)),
                            Ok(msgs) => HookResult::GrpcMessageReply(Ok(MessageReply::MessageGet(msgs))),
                        };
                        return (false, Some(new_acc));
                    }
                    GrpcMessage::Data(data) => {
                        let new_acc = match RaftGrpcMessage::decode(data) {
                            Err(e) => {
                                log::error!("Message::decode, error: {e:?}");
                                HookResult::GrpcMessageReply(Ok(MessageReply::Error(e.to_string())))
                            }
                            Ok(RaftGrpcMessage::GetNodeHealthStatus) => {
                                match self.shared.health_status().await {
                                    Ok(status) => {
                                        match RaftGrpcMessageReply::GetNodeHealthStatus(status).encode() {
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
