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
    MqttError, Result, Runtime,
};
use rmqtt_storage::{init_db, SledStorageDB, StorageType};

use config::PluginConfig;
use message::{get_or_init, StorageMessageManager};

mod config;
mod message;

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
    message_mgr: &'static StorageMessageManager,
}

fn db_cleanup(_db: &SledStorageDB) {
    {
        log::info!("*** db_cleanup start ...");
        let db = _db.clone();
        std::thread::spawn(move || {
            let limit = 100;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(10));
                let mut total_cleanups = 0;
                let now = std::time::Instant::now();
                loop {
                    // log::info!("cleanup start ...");
                    let now = std::time::Instant::now();
                    let count = db.cleanup(limit);
                    // log::info!("cleanup end ... cost time: {:?}", now.elapsed());
                    total_cleanups += count;
                    if count > 0 {
                        log::info!(
                            "def_cleanup: {}, total cleanups: {}, active_count(): {}, cost time: {:?}",
                            count,
                            total_cleanups,
                            db.active_count(),
                            now.elapsed()
                        );
                    }
                    if count < limit {
                        break;
                    }
                    if db.active_count() > 50 {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(0));
                    }
                }
                if now.elapsed().as_secs() > 3 {
                    log::info!("total cleanups: {}, cost time: {:?}", total_cleanups, now.elapsed());
                }
            }
        });
    }
}

impl StoragePlugin {
    #[inline]
    async fn new<S: Into<String>>(runtime: &'static Runtime, name: S, descr: S) -> Result<Self> {
        let name = name.into();
        let node_id = runtime.node.id();
        let mut cfg = runtime.settings.plugins.load_config_default::<PluginConfig>(&name)?;
        let should_merge_on_get = match cfg.storage.typ {
            StorageType::Sled => {
                cfg.storage.sled.path = cfg.storage.sled.path.replace("{node}", &format!("{}", node_id));
                cfg.storage.sled.cleanup_f = db_cleanup;
                true
            }
            StorageType::Redis => {
                cfg.storage.redis.prefix =
                    cfg.storage.redis.prefix.replace("{node}", &format!("{}", node_id));
                true
            }
            _ => return Err(MqttError::from("Unsupported storage type")),
        };
        log::info!("{} StoragePlugin cfg: {:?}", name, cfg);

        let storage_db = init_db(&cfg.storage).await?;

        let register = runtime.extends.hook_mgr().await.register();

        let cfg = Arc::new(cfg);
        let message_mgr = get_or_init(node_id, cfg.clone(), storage_db.clone(), should_merge_on_get).await?;

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
        *self.runtime.extends.message_mgr_mut().await = Box::new(self.message_mgr);

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
        let now = std::time::Instant::now();
        let msg_queue_count = self.message_mgr.msg_queue_count.load(Ordering::Relaxed);
        let topics_nodes = self.message_mgr.topic_tree.read().await.nodes_size();
        let receiveds = self.message_mgr.topic_tree.read().await.values_size();
        let exec_active_count = self.message_mgr.exec.active_count();
        let exec_waiting_count = self.message_mgr.exec.waiting_count();
        let storage_info = self.message_mgr.storage_db.info().await.unwrap_or_default();
        let cost_time = format!("{:?}", now.elapsed());
        json!(
            {
                "storage_info": storage_info,
                "msg_queue_count": msg_queue_count,
                "message": {
                    "topics_nodes": topics_nodes,
                    "receiveds": receiveds,
                    "cost_time":cost_time,
                },
                "exec_active_count": exec_active_count,
                "exec_waiting_count": exec_waiting_count,
            }
        )
    }
}
