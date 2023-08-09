#![deny(unsafe_code)]

use rmqtt::broker::hook::Priority;
use rmqtt::{async_trait::async_trait, log};
use rmqtt::{
    broker::hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    broker::metrics::Metrics,
    plugin::{DynPlugin, DynPluginResult, Plugin},
    Result, Runtime,
};

#[inline]
pub async fn register(
    runtime: &'static Runtime,
    name: &'static str,
    descr: &'static str,
    default_startup: bool,
    immutable: bool,
) -> Result<()> {
    runtime
        .plugins
        .register(name, default_startup, immutable, move || -> DynPluginResult {
            Box::pin(async move {
                CounterPlugin::new(runtime, name, descr).await.map(|p| -> DynPlugin { Box::new(p) })
            })
        })
        .await?;
    Ok(())
}

struct CounterPlugin {
    name: String,
    descr: String,
    register: Box<dyn Register>,
}

impl CounterPlugin {
    #[inline]
    async fn new<N: Into<String>, D: Into<String>>(
        runtime: &'static Runtime,
        name: N,
        descr: D,
    ) -> Result<Self> {
        let name = name.into();
        let register = runtime.extends.hook_mgr().await.register();
        Ok(Self { name, descr: descr.into(), register })
    }
}

#[async_trait]
impl Plugin for CounterPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name);
        self.register.add_priority(Type::ClientConnect, Priority::MAX, Box::new(CounterHandler::new())).await;
        self.register
            .add_priority(Type::ClientAuthenticate, Priority::MAX, Box::new(CounterHandler::new()))
            .await;
        self.register.add_priority(Type::ClientConnack, Priority::MAX, Box::new(CounterHandler::new())).await;
        self.register
            .add_priority(Type::ClientConnected, Priority::MAX, Box::new(CounterHandler::new()))
            .await;
        self.register
            .add_priority(Type::ClientDisconnected, Priority::MAX, Box::new(CounterHandler::new()))
            .await;
        self.register
            .add_priority(Type::ClientSubscribe, Priority::MAX, Box::new(CounterHandler::new()))
            .await;
        self.register
            .add_priority(Type::ClientUnsubscribe, Priority::MAX, Box::new(CounterHandler::new()))
            .await;

        self.register
            .add_priority(Type::ClientSubscribeCheckAcl, Priority::MAX, Box::new(CounterHandler::new()))
            .await;
        self.register
            .add_priority(Type::MessagePublishCheckAcl, Priority::MAX, Box::new(CounterHandler::new()))
            .await;

        self.register
            .add_priority(Type::SessionCreated, Priority::MAX, Box::new(CounterHandler::new()))
            .await;
        self.register
            .add_priority(Type::SessionTerminated, Priority::MAX, Box::new(CounterHandler::new()))
            .await;
        self.register
            .add_priority(Type::SessionSubscribed, Priority::MAX, Box::new(CounterHandler::new()))
            .await;
        self.register
            .add_priority(Type::SessionUnsubscribed, Priority::MAX, Box::new(CounterHandler::new()))
            .await;

        self.register
            .add_priority(Type::MessagePublish, Priority::MAX, Box::new(CounterHandler::new()))
            .await;
        self.register
            .add_priority(Type::MessageDelivered, Priority::MAX, Box::new(CounterHandler::new()))
            .await;
        self.register.add_priority(Type::MessageAcked, Priority::MAX, Box::new(CounterHandler::new())).await;
        self.register
            .add_priority(Type::MessageDropped, Priority::MAX, Box::new(CounterHandler::new()))
            .await;

        Ok(())
    }

    #[inline]
    fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        Ok(())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name);
        self.register.start().await;
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::warn!("{} stop, the Counter plug-in, it cannot be stopped", self.name);
        Ok(false)
    }

    #[inline]
    fn version(&self) -> &str {
        "0.1.0"
    }

    #[inline]
    fn descr(&self) -> &str {
        &self.descr
    }
}

struct CounterHandler {
    metrics: &'static Metrics,
}

impl CounterHandler {
    fn new() -> Self {
        Self { metrics: Metrics::instance() }
    }
}

#[async_trait]
impl Handler for CounterHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::ClientConnect(connect_info) => {
                self.metrics.client_connect_inc();
                if connect_info.username().is_none() {
                    self.metrics.client_auth_anonymous_inc();
                }
            }
            Parameter::ClientAuthenticate(_) => {
                self.metrics.client_authenticate_inc();
            }
            Parameter::ClientConnack(connect_info, reason) => {
                self.metrics.client_connack_inc();
                match reason.success_or_auth_error() {
                    (true, _) => {}
                    (false, not_authorized) => {
                        self.metrics.client_connack_error_inc();
                        if not_authorized {
                            self.metrics.client_connack_auth_error_inc();
                        }
                        if connect_info.username().is_none() {
                            self.metrics.client_auth_anonymous_error_inc();
                        }
                    }
                }
            }
            Parameter::ClientConnected(_session, client) => {
                self.metrics.client_connected_inc();
                if client.session_present {
                    self.metrics.session_resumed_inc();
                }
            }
            Parameter::ClientDisconnected(_session, _client, _r) => {
                self.metrics.client_disconnected_inc();
            }
            Parameter::ClientSubscribeCheckAcl(_session, _client, _s) => {
                self.metrics.client_subscribe_check_acl_inc();
            }
            Parameter::ClientSubscribe(_s, _client, _sub) => {
                self.metrics.client_subscribe_inc();
            }
            Parameter::ClientUnsubscribe(_s, _client, _unsub) => {
                self.metrics.client_unsubscribe_inc();
            }

            Parameter::SessionCreated(_session, _client) => {
                self.metrics.session_created_inc();
            }
            Parameter::SessionTerminated(_session, _client, _r) => {
                self.metrics.session_terminated_inc();
            }
            Parameter::SessionSubscribed(_s, _client, _sub) => {
                self.metrics.session_subscribed_inc();
            }
            Parameter::SessionUnsubscribed(_s, _client, _unsub) => {
                self.metrics.session_unsubscribed_inc();
            }

            Parameter::MessagePublishCheckAcl(_session, _client, _p) => {
                self.metrics.client_publish_check_acl_inc();
            }
            Parameter::MessagePublish(_session, _client, _p) => {
                // self.metrics.messages_received_inc();  //@TODO ... elaboration
                // match p.qos{
                //     QoS::AtMostOnce => self.metrics.messages_received_qos0_inc(),
                //     QoS::AtLeastOnce => self.metrics.messages_received_qos1_inc(),
                //     QoS::ExactlyOnce => self.metrics.messages_received_qos2_inc(),
                // }
                self.metrics.messages_publish_inc();
            }
            Parameter::MessageDelivered(_session, _client, from, _p) => {
                if !from.is_system() {
                    self.metrics.messages_delivered_inc();
                }
            }
            Parameter::MessageAcked(_session, _client, from, _p) => {
                if !from.is_system() {
                    self.metrics.messages_acked_inc();
                }
            }
            Parameter::MessageDropped(_to, _from, _p, _r) => {
                self.metrics.messages_dropped_inc(); //@TODO ... elaboration
            }

            _ => {
                log::error!("parameter is: {:?}", param);
            }
        }
        (true, acc)
    }
}
