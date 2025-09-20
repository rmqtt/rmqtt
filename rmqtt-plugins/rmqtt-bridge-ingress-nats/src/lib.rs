#![deny(unsafe_code)]

use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{self, json};
use tokio::sync::mpsc;
use tokio::sync::RwLock;

use rmqtt::{
    context::ServerContext,
    hook::Register,
    macros::Plugin,
    plugin::{PackageInfo, Plugin},
    register, Result,
};

use bridge::{BridgeManager, SystemCommand};
use config::PluginConfig;

mod bridge;
mod config;

register!(BridgeNatsIngressPlugin::new);

#[derive(Plugin)]
struct BridgeNatsIngressPlugin {
    scx: ServerContext,
    cfg: Arc<RwLock<PluginConfig>>,
    register: Box<dyn Register>,
    bridge_mgr: BridgeManager,
    bridge_mgr_cmd_tx: mpsc::Sender<SystemCommand>,
}

impl BridgeNatsIngressPlugin {
    #[inline]
    async fn new(scx: ServerContext, name: &'static str) -> Result<Self> {
        let cfg = Self::load_cfg(&scx, name)?;
        let cfg = Arc::new(RwLock::new(cfg));
        log::info!("{} BridgeNatsIngressPlugin cfg: {:?}", name, cfg.read().await);
        let register = scx.extends.hook_mgr().register();
        let bridge_mgr = BridgeManager::new(scx.clone(), scx.node.id(), cfg.clone()).await;
        let bridge_mgr_cmd_tx = Self::start(name.into(), bridge_mgr.clone());
        Ok(Self { scx, cfg, register, bridge_mgr, bridge_mgr_cmd_tx })
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
                            log::info!("start bridge-ingress-nats ok.");
                        }
                        SystemCommand::Restart => {
                            log::info!("{name} restart bridge-ingress-nats ...");
                            bridge_mgr.stop().await;
                            tokio::time::sleep(Duration::from_millis(3000)).await;
                            bridge_mgr.start(sys_cmd_tx.clone()).await;
                            log::info!("start bridge-ingress-nats ok.");
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

    fn load_cfg(scx: &ServerContext, name: &str) -> Result<PluginConfig> {
        scx.plugins.read_config::<PluginConfig>(name)
    }
}

#[async_trait]
impl Plugin for BridgeNatsIngressPlugin {
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
        *self.cfg.write().await = Self::load_cfg(&self.scx, self.name())?;
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
