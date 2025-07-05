use async_trait::async_trait;

use rmqtt::{
    context::ServerContext,
    grpc::{Message as GrpcMessage, MessageReply as GrpcMessageReply, MessageType},
    hook::{Handler, HookResult, Parameter, ReturnType},
};

use super::{
    clients, plugin, subs,
    types::{Message, MessageReply},
};

pub(crate) struct HookHandler {
    scx: ServerContext,
    pub message_type: MessageType,
}

impl HookHandler {
    pub(crate) fn new(scx: ServerContext, message_type: MessageType) -> Self {
        Self { scx, message_type }
    }
}

#[async_trait]
impl Handler for HookHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::GrpcMessageReceived(typ, msg) => {
                log::debug!("GrpcMessageReceived, type: {typ}, msg: {msg:?}");
                if self.message_type != *typ {
                    return (true, acc);
                }
                match msg {
                    GrpcMessage::Data(data) => {
                        let new_acc = match Message::decode(data) {
                            Err(e) => {
                                log::error!("Message::decode, error: {e:?}");
                                HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(e.to_string())))
                            }
                            Ok(Message::BrokerInfo) => {
                                let broker_info = self.scx.node.broker_info(&self.scx).await;
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
                                let node_info = self.scx.node.node_info(&self.scx).await;
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
                                let node_status = self.scx.node.status(&self.scx).await;
                                let stats = self.scx.stats.clone(&self.scx).await;
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
                                let metrics = self.scx.metrics.clone();
                                match MessageReply::MetricsInfo(Box::new(metrics)).encode() {
                                    Ok(ress) => {
                                        HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                    }
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                }
                            }
                            Ok(Message::ClientSearch(q)) => {
                                match MessageReply::ClientSearch(clients::search(&self.scx, &q).await)
                                    .encode()
                                {
                                    Ok(ress) => {
                                        HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                    }
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                }
                            }
                            Ok(Message::ClientGet { clientid }) => {
                                match MessageReply::ClientGet(clients::get(&self.scx, clientid).await)
                                    .encode()
                                {
                                    Ok(ress) => {
                                        HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                    }
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                }
                            }
                            Ok(Message::Subscribe(params)) =>
                            {
                                #[allow(clippy::mutable_key_type)]
                                match subs::subscribe(&self.scx, params).await {
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
                                }
                            }
                            Ok(Message::Unsubscribe(params)) => {
                                match subs::unsubscribe(&self.scx, params).await {
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
                                }
                            }
                            Ok(Message::GetPlugins) => match plugin::get_plugins(&self.scx).await {
                                Ok(plugins) => match MessageReply::GetPlugins(plugins).encode() {
                                    Ok(ress) => {
                                        HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                    }
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                },
                                Err(e) => {
                                    HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(e.to_string())))
                                }
                            },
                            Ok(Message::GetPlugin { name }) => {
                                match plugin::get_plugin(&self.scx, name).await {
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
                                }
                            }
                            Ok(Message::GetPluginConfig { name }) => {
                                match plugin::get_plugin_config(&self.scx, name).await {
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
                                }
                            }
                            Ok(Message::ReloadPluginConfig { name }) => {
                                match self.scx.plugins.load_config(name).await {
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
                                }
                            }
                            Ok(Message::LoadPlugin { name }) => match self.scx.plugins.start(name).await {
                                Ok(()) => match MessageReply::LoadPlugin.encode() {
                                    Ok(ress) => {
                                        HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                    }
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                },
                                Err(e) => {
                                    HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(e.to_string())))
                                }
                            },
                            Ok(Message::UnloadPlugin { name }) => match self.scx.plugins.stop(name).await {
                                Ok(ok) => match MessageReply::UnloadPlugin(ok).encode() {
                                    Ok(ress) => {
                                        HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Data(ress)))
                                    }
                                    Err(e) => HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(
                                        e.to_string(),
                                    ))),
                                },
                                Err(e) => {
                                    HookResult::GrpcMessageReply(Ok(GrpcMessageReply::Error(e.to_string())))
                                }
                            },
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
