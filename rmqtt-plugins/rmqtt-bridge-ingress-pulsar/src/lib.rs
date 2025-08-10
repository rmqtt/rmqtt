#![allow(clippy::result_large_err)]
#![deny(unsafe_code)]
#[macro_use]
extern crate serde;

#[macro_use]
extern crate rmqtt_macros;

use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use rmqtt::{
    async_trait::async_trait,
    log,
    serde_json::{self, json},
    tokio,
    tokio::sync::mpsc,
    tokio::sync::RwLock,
};
use rmqtt::{
    broker::hook::Register,
    plugin::{PackageInfo, Plugin},
    register, Result, Runtime,
};

use bridge::{BridgeManager, SystemCommand};
use config::PluginConfig;

mod bridge;
mod config;

register!(BridgePulsarIngressPlugin::new);

#[derive(Plugin)]
struct BridgePulsarIngressPlugin {
    _runtime: &'static Runtime,
    cfg: Arc<RwLock<PluginConfig>>,
    register: Box<dyn Register>,
    bridge_mgr: BridgeManager,
    bridge_mgr_cmd_tx: mpsc::Sender<SystemCommand>,
}

impl BridgePulsarIngressPlugin {
    #[inline]
    async fn new(runtime: &'static Runtime, name: &'static str) -> Result<Self> {
        let cfg = Self::load_cfg(runtime, name)?;
        let cfg = Arc::new(RwLock::new(cfg));
        log::info!("{} BridgePulsarIngressPlugin cfg: {:?}", name, cfg.read().await);
        let register = runtime.extends.hook_mgr().await.register();
        let bridge_mgr = BridgeManager::new(runtime.node.id(), cfg.clone()).await;

        let bridge_mgr_cmd_tx = Self::start(name.into(), bridge_mgr.clone());
        Ok(Self { _runtime: runtime, cfg, register, bridge_mgr, bridge_mgr_cmd_tx })
    }

    fn start(name: String, mut bridge_mgr: BridgeManager) -> mpsc::Sender<SystemCommand> {
        let (bridge_mgr_cmd_tx, mut bridge_mgr_cmd_rx) = mpsc::channel(10);
        let sys_cmd_tx = bridge_mgr_cmd_tx.clone();
        std::thread::spawn(move || {
            let runner = async move {
                while let Some(cmd) = bridge_mgr_cmd_rx.recv().await {
                    match cmd {
                        SystemCommand::Start => {
                            bridge_mgr.start(sys_cmd_tx.clone()).await;
                            log::info!("start bridge-ingress-pulsar ok.");
                        }
                        SystemCommand::Restart => {
                            log::info!("{name} restart bridge-ingress-pulsar ...");
                            bridge_mgr.stop().await;
                            tokio::time::sleep(Duration::from_millis(3000)).await;
                            bridge_mgr.start(sys_cmd_tx.clone()).await;
                            log::info!("start bridge-ingress-pulsar ok.");
                        }
                        SystemCommand::Close => {
                            bridge_mgr.stop().await;
                        }
                    }
                }
            };
            tokio::runtime::Runtime::new().unwrap().block_on(runner);
        });
        bridge_mgr_cmd_tx
    }

    fn load_cfg(runtime: &'static Runtime, name: &str) -> Result<PluginConfig> {
        let mut cfg = runtime.settings.plugins.load_config::<PluginConfig>(name)?;
        cfg.prepare();
        Ok(cfg)
    }
}

#[async_trait]
impl Plugin for BridgePulsarIngressPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        Ok(())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name());
        self.register.start().await;
        self.bridge_mgr_cmd_tx.send(SystemCommand::Start).await?;
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::info!("{} stop", self.name());
        self.register.stop().await;
        self.bridge_mgr_cmd_tx.send(SystemCommand::Close).await?;
        Ok(true)
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self.cfg.read().await.deref())?)
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        *self.cfg.write().await = Self::load_cfg(self._runtime, self.name())?;
        //@TODO stop and start ...
        Ok(())
    }

    #[inline]
    async fn attrs(&self) -> serde_json::Value {
        let bridges = self
            .bridge_mgr
            .sources()
            .iter()
            .map(|entry| {
                let ((bridge_name, entry_idx), mailbox) = entry.pair();
                json!({
                    "consumer_name": mailbox.consumer_name,
                    "name": bridge_name,
                    "entry_idx": entry_idx,
                })
            })
            .collect::<Vec<serde_json::Value>>();
        json!({
            "bridges": bridges,
        })
    }
}
