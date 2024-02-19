#![deny(unsafe_code)]
#[macro_use]
extern crate serde;

#[macro_use]
extern crate rmqtt_macros;

use config::PluginConfig;
use retainer::Retainer;
use rmqtt::broker::RetainStorage;
use rmqtt::grpc::MessageType;
use rmqtt::{async_trait::async_trait, log, serde_json, tokio::sync::RwLock, tokio_cron_scheduler::Job};
use rmqtt::{
    broker::hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    grpc::{Message, MessageReply},
    plugin::{DynPlugin, DynPluginResult, PackageInfo, Plugin},
    Result, Runtime,
};
use std::sync::Arc;

mod config;
mod retainer;

#[inline]
pub async fn register(
    runtime: &'static Runtime,
    name: &'static str,
    default_startup: bool,
    immutable: bool,
) -> Result<()> {
    runtime
        .plugins
        .register(name, default_startup, immutable, move || -> DynPluginResult {
            Box::pin(
                async move { RetainerPlugin::new(runtime, name).await.map(|p| -> DynPlugin { Box::new(p) }) },
            )
        })
        .await?;
    Ok(())
}

#[derive(Plugin)]
struct RetainerPlugin {
    runtime: &'static Runtime,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
    retainer: &'static Retainer,
}

impl RetainerPlugin {
    #[inline]
    async fn new<N: Into<String>>(runtime: &'static Runtime, name: N) -> Result<Self> {
        let name = name.into();
        let cfg = runtime.settings.plugins.load_config::<PluginConfig>(&name)?;
        log::info!("{} RetainerPlugin cfg: {:?}", name, cfg);
        let register = runtime.extends.hook_mgr().await.register();
        let message_type = cfg.message_type;
        let cfg = Arc::new(RwLock::new(cfg));
        let retainer = Retainer::get_or_init(cfg.clone(), message_type);

        Ok(Self { runtime, register, cfg, retainer })
    }
}

#[async_trait]
impl Plugin for RetainerPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        let cfg = &self.cfg;
        let message_type = cfg.read().await.message_type;
        self.register
            .add(Type::GrpcMessageReceived, Box::new(RetainHandler::new(self.retainer, cfg, message_type)))
            .await;

        let retainer = self.retainer;
        let async_jj = Job::new_async("0 1/10 * * * *", move |_uuid, _l| {
            Box::pin(async move {
                let c = retainer.count();
                retainer.remove_expired_messages().await;
                let removeds = c - retainer.count();
                if removeds > 0 {
                    log::info!(
                        "{:?} remove_expired_messages, removed count: {}",
                        std::thread::current().id(),
                        removeds
                    );
                }
            })
        })
        .unwrap();
        self.runtime.sched.add(async_jj).await.unwrap();

        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.runtime.settings.plugins.load_config::<PluginConfig>(self.name())?;
        *self.cfg.write().await = new_cfg;
        log::debug!("load_config ok,  {:?}", self.cfg);
        Ok(())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name());
        *self.runtime.extends.retain_mut().await = Box::new(self.retainer);
        self.register.start().await;
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::warn!("{} stop, the default Retainer plug-in, it cannot be stopped", self.name());
        //self.register.stop().await;
        Ok(false)
    }
}

struct RetainHandler {
    retainer: &'static Retainer,
    _cfg: Arc<RwLock<PluginConfig>>,
    message_type: MessageType,
}

impl RetainHandler {
    fn new(retainer: &'static Retainer, cfg: &Arc<RwLock<PluginConfig>>, message_type: MessageType) -> Self {
        Self { retainer, _cfg: cfg.clone(), message_type }
    }
}

#[async_trait]
impl Handler for RetainHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::GrpcMessageReceived(typ, msg) => {
                log::debug!("GrpcMessageReceived, type: {}, msg: {:?}", typ, msg);
                if self.message_type != *typ {
                    return (true, acc);
                }
                match msg {
                    Message::GetRetains(topic_filter) => {
                        let new_acc = match self.retainer.inner().get(topic_filter).await {
                            Ok(retains) => {
                                HookResult::GrpcMessageReply(Ok(MessageReply::GetRetains(retains)))
                            }
                            Err(e) => HookResult::GrpcMessageReply(Err(e)),
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
