use rmqtt::{async_trait::async_trait, log};
use rmqtt::{
    broker::hook::{Handler, HookResult, Parameter, ReturnType},
    grpc::{Message as GrpcMessage, MessageReply as GrpcMessageReply, MessageType},
    Runtime,
};

use super::clients;
use super::plugin;
use super::subs;
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
                    GrpcMessage::Data(data) => {
                        let new_acc = match Message::decode(data) {
                            Err(e) => {
                                log::error!("Message::decode, error: {:?}", e);
                                HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(e.to_string())))
                            }
                            Ok(Message::BrokerInfo) => {
                                let broker_info = Runtime::instance().node.broker_info().await;
                                match MessageReply::BrokerInfo(broker_info).encode() {
                                    Ok(ress) => {
                                        HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                    }
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                }
                            }
                            Ok(Message::NodeInfo) => {
                                let node_info = Runtime::instance().node.node_info().await;
                                match MessageReply::NodeInfo(node_info).encode() {
                                    Ok(ress) => {
                                        HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                    }
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                }
                            }
                            Ok(Message::StatsInfo) => {
                                let node_status = Runtime::instance().node.status().await;
                                let stats = Runtime::instance().stats.clone().await;
                                match MessageReply::StatsInfo(node_status, Box::new(stats)).encode() {
                                    Ok(ress) => {
                                        HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                    }
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                }
                            }
                            Ok(Message::MetricsInfo) => {
                                let metrics = Runtime::instance().metrics.clone();
                                match MessageReply::MetricsInfo(metrics).encode() {
                                    Ok(ress) => {
                                        HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                    }
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                }
                            }
                            Ok(Message::ClientSearch(q)) => {
                                match MessageReply::ClientSearch(clients::search(&q).await).encode() {
                                    Ok(ress) => {
                                        HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                    }
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                }
                            }
                            Ok(Message::ClientGet { clientid }) => {
                                match MessageReply::ClientGet(clients::get(clientid).await).encode() {
                                    Ok(ress) => {
                                        HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                    }
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                }
                            }
                            Ok(Message::Subscribe(params)) => {
                                let replys = match subs::subscribe(params).await {
                                    Ok(replys) => {
                                        let ress = replys
                                            .into_iter()
                                            .map(|(t, res)| match res {
                                                Ok(b) => (t, (b, None)),
                                                Err(e) => (t, (false, Some(e.to_string()))),
                                            })
                                            .collect();
                                        match MessageReply::Subscribe(ress).encode() {
                                            Ok(ress) => {
                                                HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                            }
                                            Err(e) => HookResult::GrpcMessageReply(Ok(
                                                GrpcMessageReply::Error(e.to_string()),
                                            )),
                                        }
                                    }
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                };
                                replys
                            }
                            Ok(Message::Unsubscribe(params)) => {
                                let replys = match subs::unsubscribe(params).await {
                                    Ok(()) => match MessageReply::Unsubscribe.encode() {
                                        Ok(ress) => {
                                            HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                        }
                                        Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                            e.to_string(),
                                        ))),
                                    },
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                };
                                replys
                            }
                            Ok(Message::GetPlugins) => {
                                let replys = match plugin::get_plugins().await {
                                    Ok(plugins) => match MessageReply::GetPlugins(plugins).encode() {
                                        Ok(ress) => {
                                            HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                        }
                                        Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                            e.to_string(),
                                        ))),
                                    },
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                };
                                replys
                            }
                            Ok(Message::GetPlugin { name }) => {
                                let reply = match plugin::get_plugin(name).await {
                                    Ok(plugin) => match MessageReply::GetPlugin(plugin).encode() {
                                        Ok(ress) => {
                                            HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                        }
                                        Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                            e.to_string(),
                                        ))),
                                    },
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                };
                                reply
                            }
                            Ok(Message::GetPluginConfig { name }) => {
                                let reply = match plugin::get_plugin_config(name).await {
                                    Ok(cfg) => match MessageReply::GetPluginConfig(cfg).encode() {
                                        Ok(ress) => {
                                            HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                        }
                                        Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                            e.to_string(),
                                        ))),
                                    },
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                };
                                reply
                            }
                            Ok(Message::ReloadPluginConfig { name }) => {
                                let reply = match Runtime::instance().plugins.load_config(name).await {
                                    Ok(()) => match MessageReply::ReloadPluginConfig.encode() {
                                        Ok(ress) => {
                                            HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                        }
                                        Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                            e.to_string(),
                                        ))),
                                    },
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                };
                                reply
                            }
                            Ok(Message::LoadPlugin { name }) => {
                                let reply = match Runtime::instance().plugins.start(name).await {
                                    Ok(()) => match MessageReply::LoadPlugin.encode() {
                                        Ok(ress) => {
                                            HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                        }
                                        Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                            e.to_string(),
                                        ))),
                                    },
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                };
                                reply
                            }
                            Ok(Message::UnloadPlugin { name }) => {
                                let reply = match Runtime::instance().plugins.stop(name).await {
                                    Ok(ok) => match MessageReply::UnloadPlugin(ok).encode() {
                                        Ok(ress) => {
                                            HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                        }
                                        Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                            e.to_string(),
                                        ))),
                                    },
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                };
                                reply
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
