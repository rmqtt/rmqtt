#![deny(unsafe_code)]

use std::ops::Deref;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{self, json};
use tokio::sync::mpsc;
use tokio::sync::RwLock;

use rmqtt::{
    context::ServerContext,
    hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    macros::Plugin,
    plugin::{PackageInfo, Plugin},
    register, Result,
};

use bridge::{BridgeManager, Command};
use config::PluginConfig;

mod bridge;
mod config;

register!(BridgePulsarEgressPlugin::new);

#[derive(Plugin)]
struct BridgePulsarEgressPlugin {
    cfg: Arc<RwLock<PluginConfig>>,
    register: Box<dyn Register>,
    bridge_mgr: BridgeManager,
    bridge_mgr_cmd_tx: mpsc::Sender<Command>,
}

impl BridgePulsarEgressPlugin {
    #[inline]
    async fn new(scx: ServerContext, name: &'static str) -> Result<Self> {
        let cfg = Arc::new(RwLock::new(scx.plugins.read_config::<PluginConfig>(name)?));
        log::info!("{} BridgePulsarEgressPlugin cfg: {:?}", name, cfg.read().await);
        let register = scx.extends.hook_mgr().register();
        let bridge_mgr = BridgeManager::new(scx.node.id(), cfg.clone()).await;

        let bridge_mgr_cmd_tx = Self::start(name.to_owned(), bridge_mgr.clone());
        Ok(Self { cfg, register, bridge_mgr, bridge_mgr_cmd_tx })
    }

    fn start(_name: String, mut bridge_mgr: BridgeManager) -> mpsc::Sender<Command> {
        let (bridge_mgr_cmd_tx, mut bridge_mgr_cmd_rx) = mpsc::channel(10);

        std::thread::spawn(move || {
            let runner = async move {
                while let Some(cmd) = bridge_mgr_cmd_rx.recv().await {
                    match cmd {
                        Command::Start => {
                            bridge_mgr.start().await;
                            log::info!("start bridge-egress-pulsar ok.");
                        }
                        Command::Close => {
                            bridge_mgr.stop().await;
                        }
                        Command::Message(_, _) => {}
                    }
                }
            };
            tokio::runtime::Runtime::new().unwrap().block_on(runner);
        });
        bridge_mgr_cmd_tx
    }
}

#[async_trait]
impl Plugin for BridgePulsarEgressPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        self.register.add(Type::MessagePublish, Box::new(HookHandler::new(self.bridge_mgr.clone()))).await;
        Ok(())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name());
        self.register.start().await;
        self.bridge_mgr_cmd_tx.send(Command::Start).await?;
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
            .sinks()
            .iter()
            .map(|entry| {
                let ((bridge_name, entry_idx), producer) = entry.pair();

                json!({
                    "producer_name": producer.name,
                    "bridge_name": bridge_name,
                    "entry_idx": entry_idx,
                })
            })
            .collect::<Vec<serde_json::Value>>();
        json!({
            "bridges": bridges,
        })
    }
}

struct HookHandler {
    bridge_mgr: BridgeManager,
}

impl HookHandler {
    fn new(bridge_mgr: BridgeManager) -> Self {
        Self { bridge_mgr }
    }
}

#[async_trait]
impl Handler for HookHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::MessagePublish(s, f, publish) => {
                log::debug!("{:?} message publish, {:?}", s.map(|s| &s.id), publish);
                if let Err(e) = self.bridge_mgr.send(f, publish).await {
                    log::error!("{:?}", e);
                }
            }
            _ => {
                log::error!("unimplemented, {:?}", param)
            }
        }
        (true, acc)
    }
}
