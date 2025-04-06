#![deny(unsafe_code)]

use std::ops::Deref;
use std::sync::Arc;

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

use bridge::{BridgeManager, Command};
use config::PluginConfig;

mod bridge;
mod config;
mod v4;
mod v5;

register!(BridgeMqttIngressPlugin::new);

#[derive(Plugin)]
struct BridgeMqttIngressPlugin {
    cfg: Arc<RwLock<PluginConfig>>,
    register: Box<dyn Register>,
    bridge_mgr: BridgeManager,
    bridge_mgr_cmd_tx: mpsc::Sender<Command>,
}

impl BridgeMqttIngressPlugin {
    #[inline]
    async fn new(scx: ServerContext, name: &'static str) -> Result<Self> {
        let cfg = Arc::new(RwLock::new(scx.plugins.read_config::<PluginConfig>(name)?));
        log::info!("{} BridgeMqttIngressPlugin cfg: {:?}", name, cfg.read().await);
        let register = scx.extends.hook_mgr().register();
        let bridge_mgr = BridgeManager::new(scx, cfg.clone());

        let bridge_mgr_cmd_tx = Self::start(name.to_owned(), bridge_mgr.clone());
        Ok(Self { cfg, register, bridge_mgr, bridge_mgr_cmd_tx })
    }

    fn start(name: String, mut bridge_mgr: BridgeManager) -> mpsc::Sender<Command> {
        let (bridge_mgr_cmd_tx, mut bridge_mgr_cmd_rx) = mpsc::channel(10);
        std::thread::spawn(move || {
            let runner = async move {
                while let Some(cmd) = bridge_mgr_cmd_rx.recv().await {
                    match cmd {
                        Command::Connect => {
                            bridge_mgr.start().await;
                            log::info!("start bridge-ingress-mqtt ok.");
                        }
                        Command::Close => {
                            bridge_mgr.stop().await;
                        }
                    }
                }
            };
            ntex::rt::System::new(&name).block_on(runner);
        });
        bridge_mgr_cmd_tx
    }
}

#[async_trait]
impl Plugin for BridgeMqttIngressPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        Ok(())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name());
        self.register.start().await;
        self.bridge_mgr_cmd_tx.send(Command::Connect).await?;
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::info!("{} stop", self.name());
        self.register.stop().await;
        self.bridge_mgr_cmd_tx.send(Command::Close).await?;
        Ok(true)
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self.cfg.read().await.deref())?)
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        Ok(())
    }

    #[inline]
    async fn attrs(&self) -> serde_json::Value {
        let bridges = self
            .bridge_mgr
            .sources()
            .iter()
            .map(|entry| {
                let ((bridge_name, entry_idx, client_no), mailbox) = entry.pair();
                json!({
                    "client_id": mailbox.client_id,
                    "name": bridge_name,
                    "entry_idx": entry_idx,
                    "client_no": client_no,
                })
            })
            .collect::<Vec<serde_json::Value>>();
        json!({
            "bridges": bridges
        })
    }
}
