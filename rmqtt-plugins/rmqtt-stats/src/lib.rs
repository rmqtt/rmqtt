use rmqtt::{
    broker::hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    plugin::{DynPlugin, DynPluginResult, Plugin},
    Result, Runtime,
    stats::DefaultStats,
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
        self.register.add(Type::ClientConnack, Box::new(StatsHandler::new())).await;
        self.register.add(Type::ClientConnected, Box::new(StatsHandler::new())).await;
        Ok(())
    }

    #[inline]
    fn name(&self) -> &str {
        &self.name
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

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        Ok(())
    }
}


struct StatsHandler {
    #[allow(dead_code)]
    stats: &'static DefaultStats,
}

impl StatsHandler {
    fn new() -> Self {
        Self {
            stats: DefaultStats::instance(),
        }
    }
}

#[async_trait]
impl Handler for StatsHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::ClientConnect(_connect_info) => {

            }

            Parameter::ClientConnack(_connect_info, _reason) => {

            }

            Parameter::ClientConnected(_session, _client) => {}

            _ => {
                log::error!("parameter is: {:?}", param);
            }
        }
        (true, acc)
    }
}

