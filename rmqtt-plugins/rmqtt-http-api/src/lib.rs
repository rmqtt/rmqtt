#![deny(unsafe_code)]

use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::{self, sync::oneshot, sync::RwLock};

use rmqtt::{
    context::ServerContext,
    hook::{Register, Type},
    macros::Plugin,
    plugin::{PackageInfo, Plugin},
    register, Result,
};

use config::PluginConfig;

mod api;
mod clients;
mod config;
mod handler;
mod plugin;
mod prome;
mod subs;
mod types;

type ShutdownTX = oneshot::Sender<()>;
type PluginConfigType = Arc<RwLock<PluginConfig>>;

register!(HttpApiPlugin::new);

#[derive(Plugin)]
struct HttpApiPlugin {
    scx: ServerContext,
    register: Box<dyn Register>,
    cfg: PluginConfigType,
    shutdown_tx: Option<ShutdownTX>,
}

impl HttpApiPlugin {
    #[inline]
    async fn new<S: Into<String>>(scx: ServerContext, name: S) -> Result<Self> {
        let name = name.into();
        let cfg = scx.plugins.read_config_default::<PluginConfig>(&name);
        log::info!("{name} HttpApiPlugin cfg: {cfg:?}");
        let cfg = Arc::new(RwLock::new(cfg?));
        let register = scx.extends.hook_mgr().register();
        let shutdown_tx = Some(Self::start(scx.clone(), cfg.clone()).await?);
        Ok(Self { scx, register, cfg, shutdown_tx })
    }

    async fn start(scx: ServerContext, cfg: PluginConfigType) -> Result<ShutdownTX> {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (started_tx, started_rx) = oneshot::channel();
        let http_laddr = cfg.read().await.http_laddr;
        tokio::spawn(async move {
            if let Err(e) = api::listen_and_serve(scx, http_laddr, cfg, shutdown_rx, started_tx).await {
                log::error!("{e:?}");
            }
            log::info!("Exit HTTP API Server, ..., http://{http_laddr:?}");
        });

        started_rx.await.map_err(|_| anyhow!("HTTP API server failed to bind on {http_laddr}"))?;

        Ok(shutdown_tx)
    }
}

#[async_trait]
impl Plugin for HttpApiPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        let mgs_type = self.cfg.read().await.message_type;
        self.register
            .add(Type::GrpcMessageReceived, Box::new(handler::HookHandler::new(self.scx.clone(), mgs_type)))
            .await;
        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.scx.plugins.read_config::<PluginConfig>(self.name())?;
        if !self.cfg.read().await.changed(&new_cfg) {
            return Ok(());
        }
        let restart_enable = self.cfg.read().await.restart_enable(&new_cfg);
        if restart_enable {
            let new_cfg = Arc::new(RwLock::new(new_cfg));
            match Self::start(self.scx.clone(), new_cfg.clone()).await {
                Ok(tx) => {
                    if let Some(old_tx) = self.shutdown_tx.take() {
                        let _ = old_tx.send(());
                    }
                    self.shutdown_tx = Some(tx);
                    self.cfg = new_cfg;
                }
                Err(e) => {
                    return Err(e);
                }
            }
        } else {
            *self.cfg.write().await = new_cfg;
        }

        log::debug!("load_config ok,  {:?}", self.cfg);
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
        //self.register.stop().await;
        Ok(false)
    }
}
