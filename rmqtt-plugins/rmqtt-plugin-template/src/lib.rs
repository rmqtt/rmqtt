use async_trait::async_trait;
use rmqtt::{
    broker::hook::{Handler, Parameter, Results, ReturnType, Type},
    plugin::Plugin,
    Result, Runtime,
};

#[inline]
pub async fn init<N: Into<String>, D: Into<String>>(
    name: N,
    descr: D,
    default_startup: bool,
) -> Result<()> {
    Runtime::instance()
        .plugins
        .register(
            Box::new(Template::new(name.into(), descr.into()).await),
            default_startup,
        )
        .await?;
    Ok(())
}

struct Template {
    name: String,
    descr: String,
}

impl Template {
    #[inline]
    async fn new(name: String, descr: String) -> Self {
        Self { name, descr }
    }
}

#[async_trait]
impl Plugin for Template {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name);

        let mut register = Runtime::instance().extends.hook_mgr().await.register();
        register.add(Type::SessionCreated, Box::new(HookHandler{}));
        register.add(Type::ClientConnect, Box::new(HookHandler{}));


        Ok(())
    }

    #[inline]
    fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name);
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::info!("{} stop", self.name);
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

struct HookHandler{}

#[async_trait]
impl Handler for HookHandler{
    async fn hook(&mut self, param: &Parameter, results: Results) -> ReturnType{
        match param{
            Parameter::SessionCreated(_session, _connection) => {
                log::debug!("session created");
            },
            Parameter::ClientConnect(_session, _connection) => {
                log::debug!("client connect");
            }
            _ => {
                log::error!("unimplemented, {:?}", param)
            }
        }
        (true, results)
    }
}