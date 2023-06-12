#![deny(unsafe_code)]
#[macro_use]
extern crate serde;

use std::str::FromStr;
use std::sync::Arc;

use config::PluginConfig;
use rmqtt::{
    async_trait::async_trait,
    log, serde_json,
    tokio::{self, sync::RwLock},
};
use rmqtt::{
    broker::hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    broker::types::{AuthResult, PublishAclResult, SubscribeAckReason, SubscribeAclResult, Topic},
    grpc::{Message, MessageReply},
    plugin::{DynPlugin, DynPluginResult, Plugin},
    Result, Runtime,
};

use retainer::Retainer;
use rmqtt::grpc::MessageType;

mod config;
mod retainer;

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
                RetainerPlugin::new(runtime, name, descr).await.map(|p| -> DynPlugin { Box::new(p) })
            })
        })
        .await?;
    Ok(())
}

struct RetainerPlugin {
    runtime: &'static Runtime,
    name: String,
    descr: String,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
    retainer: &'static Retainer,
}

impl RetainerPlugin {
    #[inline]
    async fn new<N: Into<String>, D: Into<String>>(
        runtime: &'static Runtime,
        name: N,
        descr: D,
    ) -> Result<Self> {
        let name = name.into();
        let cfg = runtime.settings.plugins.load_config::<PluginConfig>(&name)?;
        log::debug!("{} RetainerPlugin cfg: {:?}", name, cfg);
        let register = runtime.extends.hook_mgr().await.register();
        let retainer = Retainer::get_or_init(cfg.message_type);
        let cfg = Arc::new(RwLock::new(cfg));
        Ok(Self { runtime, name, descr: descr.into(), register, cfg, retainer })
    }
}

#[async_trait]
impl Plugin for RetainerPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name);
        let cfg = &self.cfg;
        let message_type = cfg.read().await.message_type;
        self.register
            .add(Type::GrpcMessageReceived, Box::new(RetainHandler::new(self.retainer, cfg, message_type)))
            .await;
        Ok(())
    }

    #[inline]
    fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.runtime.settings.plugins.load_config::<PluginConfig>(&self.name)?;
        *self.cfg.write().await = new_cfg;
        log::debug!("load_config ok,  {:?}", self.cfg);
        Ok(())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name);
        *self.runtime.extends.retain_mut().await = Box::new(self.retainer);
        self.register.start().await;
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::warn!("{} stop, the default Retainer plug-in, it cannot be stopped", self.name);
        //self.register.stop().await;
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

struct RetainHandler {
    retainer: &'static Retainer,
    cfg: Arc<RwLock<PluginConfig>>,
    message_type: MessageType,
}

impl RetainHandler {
    fn new(retainer: &'static Retainer, cfg: &Arc<RwLock<PluginConfig>>, message_type: MessageType) -> Self {
        Self { retainer, cfg: cfg.clone(), message_type }
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
