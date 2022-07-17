#[macro_use]
extern crate serde;

use std::sync::Arc;

use config::PluginConfig;
use rmqtt::{
    async_trait::async_trait,
    log, RwLock,
    serde_json,
    tokio::{self, sync::oneshot},
};
use rmqtt::{
    broker::hook::{Register, Type},
    plugin::{DynPlugin, DynPluginResult, Plugin},
    Result, Runtime,
};

mod api;
mod clients;
mod config;
mod handler;
mod subs;
mod types;

type ShutdownTX = oneshot::Sender<()>;
type PluginConfigType = Arc<RwLock<PluginConfig>>;

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
                HttpApiPlugin::new(runtime, name, descr).await.map(|p| -> DynPlugin { Box::new(p) })
            })
        })
        .await?;
    Ok(())
}

struct HttpApiPlugin {
    runtime: &'static Runtime,
    name: String,
    descr: String,
    register: Box<dyn Register>,
    cfg: PluginConfigType,
    shutdown_tx: Option<ShutdownTX>,
}

impl HttpApiPlugin {
    #[inline]
    async fn new<S: Into<String>>(runtime: &'static Runtime, name: S, descr: S) -> Result<Self> {
        let name = name.into();
        let cfg = Arc::new(RwLock::new(runtime.settings.plugins.load_config::<PluginConfig>(&name)?));
        log::debug!("{} HttpApiPlugin cfg: {:?}", name, cfg.read());
        let register = runtime.extends.hook_mgr().await.register();
        let shutdown_tx = Some(Self::start(runtime, cfg.clone()));
        Ok(Self { runtime, name, descr: descr.into(), register, cfg, shutdown_tx })
    }

    fn start(_runtime: &'static Runtime, cfg: PluginConfigType) -> ShutdownTX {
        let (shutdown_tx, shutdown_rx): (oneshot::Sender<()>, oneshot::Receiver<()>) = oneshot::channel();

        let _child = std::thread::Builder::new().name("http-api".to_string()).spawn(move || {
            let cfg1 = cfg.clone();
            let runner = async move {
                let laddr = cfg1.read().http_laddr;
                if let Err(e) = api::listen_and_serve(laddr, cfg1, shutdown_rx).await {
                    log::error!("{:?}", e);
                }
            };

            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .worker_threads(cfg.read().workers)
                .thread_name("http-api-worker")
                .thread_stack_size(4 * 1024 * 1024)
                .build()
                .unwrap();
            rt.block_on(runner);
            log::info!("Exit HTTP API Server, ..., http://{:?}", cfg.read().http_laddr);
        });
        shutdown_tx
    }
}

#[async_trait]
impl Plugin for HttpApiPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name);
        let mgs_type = self.cfg.read().message_type;
        self.register.add(Type::GrpcMessageReceived, Box::new(handler::HookHandler::new(mgs_type))).await;
        Ok(())
    }

    #[inline]
    fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().to_json()
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.runtime.settings.plugins.load_config::<PluginConfig>(&self.name)?;
        if !self.cfg.read().changed(&new_cfg) {
            return Ok(());
        }
        let restart_enable = self.cfg.read().restart_enable(&new_cfg);
        if restart_enable {
            let new_cfg = Arc::new(RwLock::new(new_cfg));
            if let Some(tx) = self.shutdown_tx.take() {
                if let Err(e) = tx.send(()) {
                    log::warn!("shutdown_tx send fail, {:?}", e);
                }
            }
            self.shutdown_tx = Some(Self::start(self.runtime, new_cfg.clone()));
            self.cfg = new_cfg;
        } else {
            *self.cfg.write() = new_cfg;
        }

        log::debug!("load_config ok,  {:?}", self.cfg);
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
        log::info!("{} stop", self.name);
        //self.register.stop().await;
        Ok(false)
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
