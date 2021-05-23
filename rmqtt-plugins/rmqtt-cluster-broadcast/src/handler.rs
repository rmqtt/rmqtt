use super::{retainer::ClusterRetainer, shared::ClusterShared, GrpcClients, MessageBroadcaster};
use rmqtt::{
    broker::{
        hook::{Handler, HookResult, Parameter, ReturnType},
        types::{From, Publish},
    },
    grpc::{Message, MessageReply},
    MqttError, Runtime,
};

pub(crate) struct HookHandler {
    grpc_clients: GrpcClients,
    shared: &'static ClusterShared,
    retainer: &'static ClusterRetainer,
}

impl HookHandler {
    pub(crate) fn new(
        grpc_clients: GrpcClients,
        shared: &'static ClusterShared,
        retainer: &'static ClusterRetainer,
    ) -> Self {
        Self { grpc_clients, shared, retainer }
    }
}

#[async_trait]
impl Handler for HookHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::MessagePublish(_session, client_info, publish) => {
                log::debug!("{:?} message publish, {:?}", client_info.id, publish);

                let grpc_clients = self.grpc_clients.clone();
                let message_type = self.shared.message_type;
                let msg = Message::Forward(client_info.id.clone(), (*publish).clone());
                let message_replys =
                    async move { MessageBroadcaster::new(grpc_clients, message_type, msg).join_all().await };

                let id = client_info.id.clone();
                let grpc_clients = self.grpc_clients.clone();
                tokio::spawn(async move {
                    for (i, res) in message_replys.await.iter().enumerate() {
                        if let Err(e) = res {
                            log::error!(
                                "{:?} send_message error, {:?}, {:?}",
                                id,
                                grpc_clients.get(i).map(|(a, _)| a),
                                e
                            );
                        }
                    }
                });
            }
            Parameter::GrpcMessageReceived(typ, msg) => {
                log::debug!("GrpcMessageReceived, type: {}, msg: {:?}", typ, msg);
                if self.shared.message_type != *typ {
                    return (true, acc);
                }
                match msg {
                    Message::Forward(from, publish) => {
                        tokio::spawn(forwards(vec![(from.clone(), publish.clone())]));
                    }
                    Message::Forwards(publishs) => {
                        tokio::spawn(forwards(publishs.clone()));
                    }
                    Message::Kick(id) => {
                        let entry = self.shared.inner().entry(id.clone());
                        log::debug!("{:?}", id);
                        let new_acc = match entry.try_lock() {
                            Ok(mut entry) => match entry.kick(true).await {
                                Ok(o) => {
                                    log::debug!("{:?} offline info: {:?}", id, o);
                                    HookResult::GrpcMessageReply(Ok(MessageReply::Kick(o)))
                                }
                                Err(e) => HookResult::GrpcMessageReply(Err(MqttError::Anyhow(e))),
                            },
                            Err(e) => {
                                log::warn!("{:?}, try_lock error, {:?}", id, e);
                                HookResult::GrpcMessageReply(Err(MqttError::Anyhow(e)))
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
                            Err(e) => HookResult::GrpcMessageReply(Err(MqttError::Anyhow(e))),
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

async fn forwards(publishs: Vec<(From, Publish)>) {
    for (from, publish) in publishs {
        if let Err(droppeds) = { Runtime::instance().extends.shared().await.forwards(from, publish).await } {
            for (to, from, publish, reason) in droppeds {
                //hook, message_dropped
                Runtime::instance()
                    .extends
                    .hook_mgr()
                    .await
                    .message_dropped(Some(to), from, publish, reason)
                    .await;
            }
        }
    }
}
