use rmqtt::{
    async_trait::async_trait,
    log,
};
use rmqtt::{
    broker::{
        hook::{Handler, HookResult, Parameter, ReturnType},
    },
    grpc::{Message, MessageReply, MessageType},
    Runtime,
};

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
                    Message::BrokerInfo => {
                        let new_acc = HookResult::GrpcMessageReply(Ok(MessageReply::BrokerInfo(
                            Runtime::instance().node.broker_info().await
                        )));
                        return (false, Some(new_acc));
                    }
                    Message::NodeInfo => {
                        let new_acc = HookResult::GrpcMessageReply(Ok(MessageReply::NodeInfo(
                            Runtime::instance().node.node_info().await
                        )));
                        return (false, Some(new_acc));
                    }
                    Message::StateInfo => {
                        let node_status = Runtime::instance().node.status().await;
                        let state = Runtime::instance().stats.clone().await;
                        let new_acc = HookResult::GrpcMessageReply(Ok(MessageReply::StateInfo(
                            node_status, Box::new(state),
                        )));
                        return (false, Some(new_acc));
                    }
                    Message::MetricsInfo => {
                        let new_acc = HookResult::GrpcMessageReply(Ok(MessageReply::MetricsInfo(
                            Runtime::instance().metrics.clone()
                        )));
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