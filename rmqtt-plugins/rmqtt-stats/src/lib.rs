use rmqtt::{
    broker::hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    plugin::{DynPlugin, DynPluginResult, Plugin},
    Result, Runtime,
    stats::DefaultStats,
    broker::{metrics::Metrics}
};
use rmqtt::async_trait::async_trait;

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
                StatsPlugin::new(runtime, name, descr).await.map(|p| -> DynPlugin { Box::new(p) })
            })
        })
        .await?;
    Ok(())
}


struct StatsPlugin {
    name: String,
    descr: String,
    register: Box<dyn Register>,
}

impl StatsPlugin {
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
impl Plugin for StatsPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name);
        self.register.add(Type::ClientConnect, Box::new(StatsHandler::new())).await;
        self.register.add(Type::ClientAuthenticate, Box::new(StatsHandler::new())).await;
        self.register.add(Type::ClientConnack, Box::new(StatsHandler::new())).await;
        self.register.add(Type::ClientConnected, Box::new(StatsHandler::new())).await;
        self.register.add(Type::ClientDisconnected, Box::new(StatsHandler::new())).await;

        self.register.add(Type::ClientSubscribeCheckAcl, Box::new(StatsHandler::new())).await;
        self.register.add(Type::MessagePublishCheckAcl, Box::new(StatsHandler::new())).await;

        self.register.add(Type::SessionCreated, Box::new(StatsHandler::new())).await;
        self.register.add(Type::SessionSubscribed, Box::new(StatsHandler::new())).await;

        self.register.add(Type::MessagePublish, Box::new(StatsHandler::new())).await;
        self.register.add(Type::MessageDelivered, Box::new(StatsHandler::new())).await;
        self.register.add(Type::MessageAcked, Box::new(StatsHandler::new())).await;

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
        log::warn!("{} stop, the Stats plug-in, it cannot be stopped", self.name);
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


struct StatsHandler {
    #[allow(dead_code)]
    stats: &'static DefaultStats,
    metrics: &'static Metrics
}

impl StatsHandler {
    fn new() -> Self {
        Self {
            stats: DefaultStats::instance(),
            metrics: Metrics::instance(),
        }
    }
}

#[async_trait]
impl Handler for StatsHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::ClientConnect(connect_info) => {
                self.metrics.client_connect_inc();
                if connect_info.username().is_none(){
                    self.metrics.client_auth_anonymous_inc();
                }
            }
            Parameter::ClientAuthenticate(_session, _client, _p) => {
                self.metrics.client_authenticate_inc();
            }
            Parameter::ClientConnack(_connect_info, _reason) => {
                self.metrics.client_connack_inc()
            }
            Parameter::ClientConnected(_session, _client) => {
                self.stats.connections_max_inc();
                self.metrics.client_connected_inc();
            }
            Parameter::ClientDisconnected(_session, _client, _r) => {
                self.metrics.client_disconnected_inc();
            }
            Parameter::ClientSubscribeCheckAcl(_session, _client, _s) => {
                self.metrics.client_subscribe_check_acl_inc();
            }
            Parameter::MessagePublishCheckAcl(_session, _client, _p) => {
                self.metrics.client_publish_check_acl_inc();
            }

            Parameter::SessionCreated(_session, _client) => {
                self.stats.sessions_max_inc();
            }
            Parameter::SessionSubscribed(s, _client, sub) => {
                self.stats.subscriptions_max_inc();
                if let Some(true) = s.is_shared_subscriptions(sub.topic_filter_ref()){
                    self.stats.subscriptions_shared_max_inc();
                }
            }
            Parameter::MessagePublish(_session, _client, _p) => {
                self.metrics.publishs_inc();
            }
            Parameter::MessageDelivered(_session, _client, _f, _p) => {
                self.metrics.delivers_inc();
            }
            Parameter::MessageAcked(_session, _client, _f, _p) => {
                self.metrics.ackeds_inc();
            }
            _ => {
                log::error!("parameter is: {:?}", param);
            }
        }
        (true, acc)
    }
}

