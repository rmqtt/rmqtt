use rmqtt::{
    async_trait::async_trait,
    log,
};
use rmqtt::{
    broker::{
        hook::{Handler, HookResult, Parameter, ReturnType},
    },
    grpc::{Message as GrpcMessage, MessageReply as GrpcMessageReply, MessageType},
    Runtime,
};

use super::clients;
use super::types::{Message, MessageReply};

pub(crate) struct HookHandler {
    pub message_type: MessageType,
}

impl HookHandler {
    pub(crate) fn new(message_type: MessageType) -> Self {
        Self { message_type }
    }
}

#[async_trait]
impl Handler for HookHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::GrpcMessageReceived(typ, msg) => {
                log::debug!("GrpcMessageReceived, type: {}, msg: {:?}", typ, msg);
                if self.message_type != *typ {
                    return (true, acc);
                }
                match msg {
                    GrpcMessage::BrokerInfo => {
                        let new_acc = HookResult::GrpcMessageReply(Ok(GrpcMessageReply::BrokerInfo(
                            Runtime::instance().node.broker_info().await
                        )));
                        return (false, Some(new_acc));
                    }
                    GrpcMessage::NodeInfo => {
                        let new_acc = HookResult::GrpcMessageReply(Ok(GrpcMessageReply::NodeInfo(
                            Runtime::instance().node.node_info().await
                        )));
                        return (false, Some(new_acc));
                    }
                    GrpcMessage::StateInfo => {
                        let node_status = Runtime::instance().node.status().await;
                        let state = Runtime::instance().stats.clone().await;
                        let new_acc = HookResult::GrpcMessageReply(Ok(GrpcMessageReply::StateInfo(
                            node_status, Box::new(state),
                        )));
                        return (false, Some(new_acc));
                    }
                    GrpcMessage::MetricsInfo => {
                        let new_acc = HookResult::GrpcMessageReply(Ok(GrpcMessageReply::MetricsInfo(
                            Runtime::instance().metrics.clone()
                        )));
                        return (false, Some(new_acc));
                    }
                    GrpcMessage::Bytes(data) => {
                        let new_acc = match Message::decode(data){
                            Err(e) => {
                                log::error!("Message::decode, error: {:?}", e);
                                HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(e.to_string())))
                            },
                            Ok(Message::ClientSearch(q)) => {
                                match MessageReply::ClientSearch(clients::search(&q).await).encode(){
                                    Ok(ress) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Bytes(ress))),
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(e.to_string())))
                                }
                            },
                            Ok(_) => unreachable!()
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