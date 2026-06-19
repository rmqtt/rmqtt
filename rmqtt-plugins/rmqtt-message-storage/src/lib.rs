//! Message storage plugin for RMQTT.
//!
//! Persists offline messages for MQTT clients with configurable
//! storage backends:
//! - **RAM**: In-memory storage (default, non-persistent).
//! - **Redis**: External Redis server.
//! - **Redis Cluster**: Distributed Redis setup.
//!
//! # Architecture
//!
//! - Plugins register `HookHandler` for hook events.
//! - Messages are stored per `(client_id, topic_filter, shared_group)`
//!   and are delivered when the client reconnects.
//! - Supports message expiry with configurable TTL.
//! - Uses an async channel for lock-free message ingestion.
//!
#![deny(unsafe_code)]

use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;

use rmqtt::{
    context::ServerContext,
    hook::Register,
    macros::Plugin,
    plugin::{PackageInfo, Plugin},
    register, Result,
};

#[cfg(any(feature = "redis", feature = "redis-cluster"))]
use rmqtt_storage::init_db;

use config::PluginConfig;
#[cfg(feature = "ram")]
use ram::RamMessageManager;
#[cfg(feature = "ram")]
use rmqtt::message::MessageManager;
#[cfg(any(feature = "redis", feature = "redis-cluster"))]
use storage::StorageMessageManager;

mod config;
#[cfg(feature = "ram")]
mod ram;
#[cfg(any(feature = "redis", feature = "redis-cluster"))]
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
        let cfg = scx.plugins.read_config_default::<PluginConfig>(&name)?;

        let result: Result<(MessageMgr, Arc<PluginConfig>)> = match cfg.storage.clone() {
            #[cfg(feature = "ram")]
            Some(config::Config::Ram(ram_cfg)) => {
                let message_mgr =
                    RamMessageManager::new(ram_cfg.clone(), cfg.cleanup_count, cfg.timeout).await?;
                Ok((MessageMgr::Ram(message_mgr), Arc::new(cfg)))
            }
            #[cfg(any(feature = "redis", feature = "redis-cluster"))]
            Some(config::Config::Storage(mut s_cfg)) => {
                let node_id = scx.node.id();
                #[cfg(feature = "redis")]
                {
                    s_cfg.redis.prefix = s_cfg.redis.prefix.replace("{node}", &format!("{node_id}"));
                }
                #[cfg(feature = "redis-cluster")]
                {
                    s_cfg.redis_cluster.prefix =
                        s_cfg.redis_cluster.prefix.replace("{node}", &format!("{node_id}"));
                }
                let storage_db = init_db(&s_cfg).await.map_err(|e| {
                    log::error!("{name} init storage db error, {e:?}");
                    e
                })?;

                let cfg = Arc::new(cfg);
                let message_mgr = StorageMessageManager::new(cfg.clone(), storage_db.clone()).await?;
                Ok((MessageMgr::Storage(message_mgr), cfg))
            }
            None => Err(anyhow!("No storage engine specified (ram, redis, or redis-cluster)")),
            #[allow(unreachable_patterns)]
            Some(_) => Err(anyhow!("Unsupported storage engine config")),
        };

        let (message_mgr, cfg) = result?;
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
    #[allow(unreachable_code)]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name());
        *self.scx.extends.message_mgr_mut().await = match &self.message_mgr {
            #[cfg(any(feature = "redis", feature = "redis-cluster"))]
            MessageMgr::Storage(mgr) => Box::new(mgr.clone()),
            #[cfg(feature = "ram")]
            MessageMgr::Ram(mgr) => Box::new(mgr.clone()),
            #[allow(unreachable_patterns)]
            _ => unreachable!("no storage backend enabled"),
        };
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
    #[cfg(feature = "ram")]
    Ram(RamMessageManager),
    #[cfg(any(feature = "redis", feature = "redis-cluster"))]
    Storage(StorageMessageManager),
}

impl MessageMgr {
    async fn restore_topic_tree(&self) -> Result<()> {
        match self {
            #[cfg(any(feature = "redis", feature = "redis-cluster"))]
            MessageMgr::Storage(mgr) => {
                mgr.restore_topic_tree().await?;
            }
            #[cfg(feature = "ram")]
            MessageMgr::Ram(_) => {}
            #[allow(unreachable_patterns)]
            _ => {}
        }
        Ok(())
    }

    async fn info(&self) -> serde_json::Value {
        match self {
            #[cfg(feature = "ram")]
            MessageMgr::Ram(mgr) => {
                let msg_max = mgr.max().await;
                let msg_count = mgr.count().await;
                let topic_nodes = mgr.topic_tree.read().await.nodes_size();
                let topic_values = mgr.topic_tree.read().await.values_size();
                let forwardeds = mgr.forwardeds_count().await;
                let expiries = mgr.expiries.read().await.len();
                let messages_bytes_size = mgr.messages_bytes_size_get();
                let msg_queue_count = mgr.msg_queue_count_get();
                serde_json::json!({
                    "storage_engine": "Ram",
                    "message": {
                        "topic_nodes": topic_nodes,
                        "topic_values": topic_values,
                        "receiveds": msg_count,
                        "receiveds_max":msg_max,
                        "forwardeds": forwardeds,
                        "expiries": expiries,
                        "bytes_size": messages_bytes_size,
                        "msg_queue_count": msg_queue_count,
                    },
                })
            }
            #[cfg(any(feature = "redis", feature = "redis-cluster"))]
            MessageMgr::Storage(mgr) => {
                let now = std::time::Instant::now();
                let msg_queue_count = mgr.msg_queue_count.load(std::sync::atomic::Ordering::Relaxed);
                let topic_nodes = mgr.topic_tree.read().await.nodes_size();
                let receiveds = mgr.topic_tree.read().await.values_size();
                let storage_info = mgr.storage_info().await;
                let cost_time = format!("{:?}", now.elapsed());
                serde_json::json!({
                    "storage_info": storage_info,
                    "msg_queue_count": msg_queue_count,
                    "message": {
                        "topic_nodes": topic_nodes,
                        "receiveds": receiveds,
                        "cost_time":cost_time,
                    },
                    "circuit_breaker": {
                        "enabled": mgr.circuit_breaker.config().enabled,
                        "state": format!("{:?}", mgr.circuit_breaker.state()),
                    },
                })
            }
            #[allow(unreachable_patterns)]
            _ => serde_json::Value::Null,
        }
    }
}
