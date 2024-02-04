use async_trait::async_trait;
use rmqtt::{
    broker::hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
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
                Template::new(runtime, name, descr).await.map(|p| -> DynPlugin { Box::new(p) })
            })
        })
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
    async fn new<S: Into<String>>(runtime: &'static Runtime, name: S, descr: S) -> Result<Self> {
        let register = runtime.extends.hook_mgr().await.register();
        Ok(Self { _runtime: runtime, name: name.into(), descr: descr.into(), register })
    }
}

#[async_trait]
impl Plugin for Template {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::debug!("{} init", self.name);
        self.register.add(Type::ClientConnack, Box::new(HookHandler::new())).await;
        self.register.add(Type::ClientSubscribe, Box::new(HookHandler::new())).await;
        self.register.add(Type::ClientUnsubscribe, Box::new(HookHandler::new())).await;
        self.register.add(Type::MessageDelivered, Box::new(HookHandler::new())).await;
        self.register.add(Type::MessagePublish, Box::new(HookHandler::new())).await;
        self.register.add_priority(Type::ClientSubscribeCheckAcl, 10, Box::new(HookHandler::new())).await;
        self.register.add_priority(Type::GrpcMessageReceived, 10, Box::new(HookHandler::new())).await;

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
        log::info!("{} stop", self.name);
        self.register.stop().await;
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

impl HookHandler {
    fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl Handler for HookHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::ClientConnack(connect_info, r) => {
                log::debug!("client connack, {:?}, {:?}", connect_info, r);
            }
            Parameter::ClientSubscribe(s, subscribe) => {
                log::debug!("{:?} client subscribe, {:?}", s.id, subscribe);
                //let mut topic_filter = subscribe.topic_filter.clone();
                //topic_filter.insert(0, Level::Normal("PPP".into()));
                //return (true, Some(HookResult::TopicFilter(Some(topic_filter))))
            }
            Parameter::ClientUnsubscribe(s, unsubscribe) => {
                log::debug!("{:?} client unsubscribe, {:?}", s.id, unsubscribe);
                //let mut topic_filter = (*unsubscribe).clone();
                //topic_filter.insert(0, Level::Normal("PPP".into()));
                //return (true, Some(HookResult::TopicFilter(Some(topic_filter))))
            }
            Parameter::MessagePublish(s, _f, publish) => {
                log::debug!("{:?} message publish, {:?}", s.map(|s| &s.id), publish);
            }
            Parameter::MessageDelivered(s, f, _publish) => {
                log::debug!("{:?} MessageDelivered, {:?}", s.id, f);
            }
            Parameter::ClientSubscribeCheckAcl(s, subscribe) => {
                log::debug!("{:?} ClientSubscribeCheckAcl, {:?}", s.id, subscribe);
            }
            _ => {
                log::error!("unimplemented, {:?}", param)
            }
        }
        (true, acc)
    }
}
