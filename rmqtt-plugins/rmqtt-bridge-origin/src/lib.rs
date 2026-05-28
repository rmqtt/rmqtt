#![deny(unsafe_code)]

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use rmqtt::{
    context::ServerContext,
    hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    macros::Plugin,
    plugin::{PackageInfo, Plugin as _PluginTrait},
    register, Result,
};

use config::PluginConfig;

mod config;

register!(BridgeOriginPlugin::new);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeDirection {
    Ingress,
    Egress,
}

#[derive(Debug, Clone, Copy)]
pub struct BridgeOrigin {
    pub direction: BridgeDirection,
}

impl BridgeOrigin {
    #[inline]
    pub fn is_ingress(&self) -> bool {
        matches!(self.direction, BridgeDirection::Ingress)
    }

    #[inline]
    pub fn is_egress(&self) -> bool {
        matches!(self.direction, BridgeDirection::Egress)
    }
}

#[derive(Plugin)]
struct BridgeOriginPlugin {
    scx: ServerContext,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
}

impl BridgeOriginPlugin {
    #[inline]
    async fn new<N: Into<String>>(scx: ServerContext, name: N) -> Result<Self> {
        let name: String = name.into();
        let cfg = scx.plugins.read_config_default::<PluginConfig>(&name);
        let cfg = Arc::new(RwLock::new(cfg?));
        let register = scx.extends.hook_mgr().register();
        log::info!("{name} BridgeOriginPlugin cfg: {cfg:?}", cfg = cfg.read().await);
        Ok(Self { scx, register, cfg })
    }
}

#[async_trait]
impl _PluginTrait for BridgeOriginPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        self.register.add(Type::ClientConnected, Box::new(BridgeOriginHandler::new(self.cfg.clone()))).await;
        log::info!("{} registered ClientConnected hook", self.name());
        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.scx.plugins.read_config::<PluginConfig>(self.name())?;
        *self.cfg.write().await = new_cfg;
        log::info!("{} load_config ok", self.name());
        Ok(())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name());
        self.register.start().await;
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::info!("{} stop", self.name());
        self.register.stop().await;
        Ok(false)
    }
}

struct BridgeOriginHandler {
    cfg: Arc<RwLock<PluginConfig>>,
}

impl BridgeOriginHandler {
    fn new(cfg: Arc<RwLock<PluginConfig>>) -> Self {
        Self { cfg }
    }
}

#[async_trait]
impl Handler for BridgeOriginHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::ClientConnected(session) => {
                let client_id = &session.id.client_id;
                let cfg = self.cfg.read().await;

                let is_ingress = client_id.contains(&cfg.ingress_marker);
                let is_egress = client_id.contains(&cfg.egress_marker);

                let direction = if is_ingress {
                    log::debug!("bridge-origin: detected ingress bridge client, client_id={}", client_id,);
                    Some(BridgeDirection::Ingress)
                } else if is_egress {
                    log::debug!("bridge-origin: detected egress bridge client, client_id={}", client_id,);
                    Some(BridgeDirection::Egress)
                } else {
                    None
                };

                if let Some(direction) = direction {
                    let extra_attrs = session.extra_attrs.clone();
                    let key = cfg.attr_key.clone();
                    extra_attrs.write().await.insert::<BridgeOrigin>(key, BridgeOrigin { direction });
                }
            }
            _ => {
                log::error!("unimplemented, {param:?}")
            }
        }
        (true, acc)
    }
}
