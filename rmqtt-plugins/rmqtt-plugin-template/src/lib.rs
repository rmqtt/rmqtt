use async_trait::async_trait;
use rmqtt::{
    broker::hook::{self, Handler, HookResult, Parameter, Register, ReturnType},
    plugin::Plugin,
    Result, Runtime,
};

#[inline]
pub async fn init<N: Into<String>, D: Into<String>>(
    runtime: &'static Runtime,
    name: N,
    descr: D,
    default_startup: bool,
) -> Result<()> {
    runtime
        .plugins
        .register(Box::new(Template::new(runtime, name.into(), descr.into()).await), default_startup)
        .await?;
    Ok(())
}

struct Template {
    _runtime: &'static Runtime,
    name: String,
    descr: String,
    register: Box<dyn Register>,
}

impl Template {
    #[inline]
    async fn new(runtime: &'static Runtime, name: String, descr: String) -> Self {
        let register = runtime.extends.hook_mgr().await.register();
        Self { _runtime: runtime, name, descr, register }
    }
}

#[async_trait]
impl Plugin for Template {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name);

        self.register.add(hook::Type::BeforeStartup, Box::new(HookHandler {}));
        self.register.add(hook::Type::SessionCreated, Box::new(HookHandler {}));
        self.register.add(hook::Type::ClientConnect, Box::new(HookHandler {}));
        self.register.add(hook::Type::ClientConnack, Box::new(HookHandler {}));
        self.register.add(hook::Type::ClientConnected, Box::new(HookHandler {}));
        self.register.add(hook::Type::ClientDisconnected, Box::new(HookHandler {}));
        self.register.add(hook::Type::ClientSubscribe, Box::new(HookHandler {}));
        self.register.add(hook::Type::MessagePublish, Box::new(HookHandler {}));
        Ok(())
    }

    #[inline]
    fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name);
        self.register.start();
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::info!("{} stop", self.name);
        self.register.stop();
        Ok(true)
    }

    #[inline]
    fn version(&self) -> &str {
        "0.1.1"
    }

    #[inline]
    fn descr(&self) -> &str {
        &self.descr
    }
}

struct HookHandler {}

#[async_trait]
impl Handler for HookHandler {
    async fn hook(&mut self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::BeforeStartup => {
                log::debug!("before startup");
            }
            Parameter::SessionCreated(_session, c) => {
                log::debug!("{:?} session created", c.id);
            }
            Parameter::ClientConnect(connect_info) => {
                log::debug!("client connect, {:?}", connect_info);
            }
            Parameter::ClientConnack(connect_info, r) => {
                log::debug!("client connack, {:?}, {:?}", connect_info, r);
            }
            Parameter::ClientConnected(_session, c) => {
                log::debug!("{:?} client connected", c.id);
            }
            Parameter::ClientDisconnected(_session, c, reason) => {
                log::debug!("{:?} client disconnected, reason: {}", c.id, reason);
            }
            Parameter::ClientSubscribe(_session, c, subscribe) => {
                log::debug!("{:?} client subscribe, {:?}", c.id, subscribe);
            }
            Parameter::MessagePublish(_session, c, publish) => {
                log::debug!("{:?} message publish, {:?}", c.id, publish);
            }
            _ => {
                log::error!("unimplemented, {:?}", param)
            }
        }
        (true, acc)
    }
}
