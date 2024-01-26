#![deny(unsafe_code)]
#[macro_use]
extern crate serde;

use rmqtt::{
    async_trait::async_trait,
    log,
    serde_json::{self, json},
};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use rmqtt::{
    broker::hook::Register,
    plugin::{DynPlugin, DynPluginResult, Plugin},
    Result, Runtime,
};
use rmqtt_storage::init_db;

use config::Config;
use config::PluginConfig;
use ram::RamMessageManager;
use rmqtt::broker::MessageManager;
use storage::StorageMessageManager;

mod config;
mod ram;
mod storage;

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
                StoragePlugin::new(runtime, name, descr).await.map(|p| -> DynPlugin { Box::new(p) })
            })
        })
        .await?;
    Ok(())
}

struct StoragePlugin {
    runtime: &'static Runtime,
    name: String,
    descr: String,
    cfg: Arc<PluginConfig>,
    register: Box<dyn Register>,
    message_mgr: MessageMgr,
}

impl StoragePlugin {
    #[inline]
    async fn new<S: Into<String>>(runtime: &'static Runtime, name: S, descr: S) -> Result<Self> {
        let name = name.into();
        let node_id = runtime.node.id();
        let mut cfg = runtime.settings.plugins.load_config_default::<PluginConfig>(&name)?;

        let (message_mgr, cfg) = match &mut cfg.storage {
            Config::Ram(ram_cfg) => {
                let message_mgr = ram::get_or_init(ram_cfg.clone(), cfg.cleanup_count).await?;
                (MessageMgr::Ram(message_mgr), Arc::new(cfg))
            }
            Config::Storage(s_cfg) => {
                s_cfg.redis.prefix = s_cfg.redis.prefix.replace("{node}", &format!("{}", node_id));
                let storage_db = init_db(s_cfg).await?;
                let cfg = Arc::new(cfg);
                let message_mgr =
                    storage::get_or_init(node_id, cfg.clone(), storage_db.clone(), true).await?;
                (MessageMgr::Storage(message_mgr), cfg)
            } // _ => return Err(MqttError::from(format!("unsupported storage type({:?})", cfg.storage.typ()))),
        };
        log::info!("{} StoragePlugin cfg: {:?}", name, cfg);
        let register = runtime.extends.hook_mgr().await.register();
        Ok(Self { runtime, name, descr: descr.into(), cfg, register, message_mgr })
    }
}

#[async_trait]
impl Plugin for StoragePlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name);
        self.message_mgr.restore_topic_tree().await?;
        Ok(())
    }

    #[inline]
    fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        Ok(self.cfg.to_json())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name);
        let mgr: Box<dyn MessageManager> = match self.message_mgr {
            MessageMgr::Storage(mgr) => Box::new(mgr),
            MessageMgr::Ram(mgr) => Box::new(mgr),
        };
        *self.runtime.extends.message_mgr_mut().await = mgr;
        self.register.start().await;
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::warn!("{} stop, if the message-storage plugin is started, it cannot be stopped", self.name);
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

    #[inline]
    async fn attrs(&self) -> serde_json::Value {
        self.message_mgr.info().await
    }
}

enum MessageMgr {
    Ram(&'static RamMessageManager),
    Storage(&'static StorageMessageManager),
}

impl MessageMgr {
    async fn restore_topic_tree(&self) -> Result<()> {
        match self {
            MessageMgr::Storage(mgr) => {
                mgr.restore_topic_tree().await?;
            }
            MessageMgr::Ram(_) => {}
        }
        Ok(())
    }

    async fn info(&self) -> serde_json::Value {
        match self {
            MessageMgr::Ram(mgr) => {
                let msg_max = mgr.max().await;
                let msg_count = mgr.count().await;
                let topic_nodes = mgr.topic_tree.read().await.nodes_size();
                let topic_values = mgr.topic_tree.read().await.values_size();
                let forwardeds = mgr.forwardeds_count().await;
                let expiries = mgr.expiries.read().await.len();
                let exec_active_count = mgr.exec.active_count();
                let exec_waiting_count = mgr.exec.waiting_count();
                let messages_bytes_size = mgr.messages_bytes_size_get();
                json!({
                    "storage_engine": "Ram",
                    "message": {
                        "topic_nodes": topic_nodes,
                        "topic_values": topic_values,
                        "receiveds": msg_count,
                        "receiveds_max":msg_max,
                        "forwardeds": forwardeds,
                        "expiries": expiries,
                        "bytes_size": messages_bytes_size,
                    },
                    "exec_active_count": exec_active_count,
                    "exec_waiting_count": exec_waiting_count,
                })
            }
            MessageMgr::Storage(mgr) => {
                let now = std::time::Instant::now();
                let msg_queue_count = mgr.msg_queue_count.load(Ordering::Relaxed);
                let topic_nodes = mgr.topic_tree.read().await.nodes_size();
                let receiveds = mgr.topic_tree.read().await.values_size();
                let exec_active_count = mgr.exec.active_count();
                let exec_waiting_count = mgr.exec.waiting_count();
                let storage_info = mgr.storage_db.info().await.unwrap_or_default();
                let cost_time = format!("{:?}", now.elapsed());
                json!({
                    "storage_info": storage_info,
                    "msg_queue_count": msg_queue_count,
                    "message": {
                        "topic_nodes": topic_nodes,
                        "receiveds": receiveds,
                        "cost_time":cost_time,
                    },
                    "exec_active_count": exec_active_count,
                    "exec_waiting_count": exec_waiting_count
                })
            }
        }
    }
}
