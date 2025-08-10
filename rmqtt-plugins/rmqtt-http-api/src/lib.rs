#![allow(clippy::result_large_err)]
#![deny(unsafe_code)]
#[macro_use]
extern crate serde;

#[macro_use]
extern crate rmqtt_macros;

use std::sync::Arc;

use config::PluginConfig;
use rmqtt::{
    async_trait::async_trait,
    log, serde_json,
    tokio::{self, sync::oneshot, sync::RwLock},
};
use rmqtt::{
    broker::hook::{Register, Type},
    plugin::{PackageInfo, Plugin},
    register, Result, Runtime,
};

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
    runtime: &'static Runtime,
    register: Box<dyn Register>,
    cfg: PluginConfigType,
    shutdown_tx: Option<ShutdownTX>,
}

impl HttpApiPlugin {
    #[inline]
    async fn new<S: Into<String>>(runtime: &'static Runtime, name: S) -> Result<Self> {
        let name = name.into();
        let cfg = Arc::new(RwLock::new(runtime.settings.plugins.load_config::<PluginConfig>(&name)?));
        log::debug!("{} HttpApiPlugin cfg: {:?}", name, cfg.read().await);
        let register = runtime.extends.hook_mgr().await.register();
        let shutdown_tx = Some(Self::start(runtime, cfg.clone()).await);
        Ok(Self { runtime, register, cfg, shutdown_tx })
    }

    async fn start(_runtime: &'static Runtime, cfg: PluginConfigType) -> ShutdownTX {
        let (shutdown_tx, shutdown_rx): (oneshot::Sender<()>, oneshot::Receiver<()>) = oneshot::channel();
        let workers = cfg.read().await.workers;
        let http_laddr = cfg.read().await.http_laddr;
        let _child = std::thread::Builder::new().name("http-api".to_string()).spawn(move || {
            let cfg1 = cfg.clone();
            let runner = async move {
                let laddr = cfg1.read().await.http_laddr;
                if let Err(e) = api::listen_and_serve(laddr, cfg1, shutdown_rx).await {
                    log::error!("{e:?}");
                }
            };

            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(workers)
                .thread_name("http-api-worker")
                .thread_stack_size(4 * 1024 * 1024)
                .build()
                .expect("tokio runtime build failed");
            rt.block_on(runner);
            log::info!("Exit HTTP API Server, ..., http://{http_laddr:?}");
        });
        shutdown_tx
    }
}

#[async_trait]
impl Plugin for HttpApiPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        let mgs_type = self.cfg.read().await.message_type;
        self.register.add(Type::GrpcMessageReceived, Box::new(handler::HookHandler::new(mgs_type))).await;
        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.runtime.settings.plugins.load_config::<PluginConfig>(self.name())?;
        if !self.cfg.read().await.changed(&new_cfg) {
            return Ok(());
        }
        let restart_enable = self.cfg.read().await.restart_enable(&new_cfg);
        if restart_enable {
            let new_cfg = Arc::new(RwLock::new(new_cfg));
            if let Some(tx) = self.shutdown_tx.take() {
                if let Err(e) = tx.send(()) {
                    log::warn!("shutdown_tx send fail, {e:?}");
                }
            }
            self.shutdown_tx = Some(Self::start(self.runtime, new_cfg.clone()).await);
            self.cfg = new_cfg;
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
