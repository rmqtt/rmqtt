#![deny(unsafe_code)]

use std::sync::atomic::Ordering;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{self, json};

use rmqtt::{
    context::ServerContext,
    hook::Register,
    macros::Plugin,
    message::MessageManager,
    plugin::{PackageInfo, Plugin},
    register, Result,
};
use rmqtt_storage::init_db;

use config::{Config, PluginConfig};
use ram::RamMessageManager;
use storage::StorageMessageManager;

mod config;
mod ram;
mod storage;

register!(StoragePlugin::new);

#[derive(Plugin)]
struct StoragePlugin {
    scx: ServerContext,
    cfg: Arc<PluginConfig>,
    register: Box<dyn Register>,
    message_mgr: MessageMgr,
}

impl StoragePlugin {
    #[inline]
    async fn new<S: Into<String>>(scx: ServerContext, name: S) -> Result<Self> {
        let name = name.into();
        let node_id = scx.node.id();
        let mut cfg = scx.plugins.read_config_default::<PluginConfig>(&name)?;

        let (message_mgr, cfg) = match &mut cfg.storage {
            Config::Ram(ram_cfg) => {
                let message_mgr = RamMessageManager::new(ram_cfg.clone(), cfg.cleanup_count).await?;
                (MessageMgr::Ram(message_mgr), Arc::new(cfg))
            }
            Config::Storage(s_cfg) => {
                s_cfg.redis.prefix = s_cfg.redis.prefix.replace("{node}", &format!("{node_id}"));
                s_cfg.redis_cluster.prefix =
                    s_cfg.redis_cluster.prefix.replace("{node}", &format!("{node_id}"));

                let storage_db = match init_db(s_cfg).await {
                    Err(e) => {
                        log::error!("{name} init storage db error, {e:?}");
                        return Err(e);
                    }
                    Ok(db) => db,
                };

                let cfg = Arc::new(cfg);
                let message_mgr =
                    StorageMessageManager::new(node_id, cfg.clone(), storage_db.clone(), true).await?;
                (MessageMgr::Storage(message_mgr), cfg)
            }
        };
        log::info!("{name} StoragePlugin cfg: {cfg:?}");
        let register = scx.extends.hook_mgr().register();
        Ok(Self { scx, cfg, register, message_mgr })
    }
}

#[async_trait]
impl Plugin for StoragePlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        self.message_mgr.restore_topic_tree().await?;
        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        Ok(self.cfg.to_json())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name());
        let mgr: Box<dyn MessageManager> = match &self.message_mgr {
            MessageMgr::Storage(mgr) => Box::new(mgr.clone()),
            MessageMgr::Ram(mgr) => Box::new(mgr.clone()),
        };
        *self.scx.extends.message_mgr_mut().await = mgr;
        self.register.start().await;
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::warn!("{} stop, if the message-storage plugin is started, it cannot be stopped", self.name());
        Ok(false)
    }

    #[inline]
    async fn attrs(&self) -> serde_json::Value {
        self.message_mgr.info().await
    }
}

enum MessageMgr {
    Ram(RamMessageManager),
    Storage(StorageMessageManager),
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
