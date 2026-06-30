//! Retained message storage plugin for RMQTT.
//!
//! Provides persistent storage for MQTT retained messages with
//! pluggable backends: in-memory (RAM), Sled embedded database,
//! and Redis.
//!
//! # Features
//!
//! - Persistent storage of retained messages across restarts.
//! - Configurable storage backend (ram, sled, redis).
//! - Message expiry with configurable cleanup intervals.
//! - Cluster-aware retained message synchronization.
//!
#![deny(unsafe_code)]

use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use serde_json::{self, json};
use tokio::sync::RwLock;

use rmqtt::{
    context::ServerContext,
    hook::Register,
    macros::Plugin,
    plugin::Plugin,
    register,
    utils::{CircuitBreaker, CircuitBreakerConfig},
    Result,
};

#[cfg(feature = "ram")]
use ram::RamRetainer;
#[cfg(any(feature = "sled", feature = "redis"))]
use rmqtt_storage::{init_db, StorageType};

#[cfg_attr(not(any(feature = "ram", feature = "sled", feature = "redis")), allow(unused_imports))]
use config::{Config, PluginConfig};
use rmqtt::plugin::PackageInfo;
use rmqtt::retain::RetainStorage;

#[cfg(any(feature = "sled", feature = "redis"))]
use futures::channel::mpsc;
#[cfg(any(feature = "sled", feature = "redis"))]
use storage::{Msg, SyncMsg};

mod config;
#[cfg(feature = "ram")]
mod ram;
#[cfg(any(feature = "sled", feature = "redis"))]
mod storage;
#[cfg_attr(not(any(feature = "sled", feature = "redis")), allow(dead_code))]
mod value_cached;

register!(RetainerPlugin::new);

#[derive(Plugin)]
struct RetainerPlugin {
    scx: ServerContext,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
    retainer: Retainer,
    #[cfg(any(feature = "sled", feature = "redis"))]
    serve_state: Option<ServeState>,
}

impl RetainerPlugin {
    #[inline]
    #[cfg_attr(
        not(any(feature = "ram", feature = "sled", feature = "redis")),
        allow(unused, unreachable_code)
    )]
    async fn new<N: Into<String>>(scx: ServerContext, name: N) -> Result<Self> {
        let name = name.into();

        let cfg = scx.plugins.read_config_default::<PluginConfig>(&name)?;
        log::info!("{name} RetainerPlugin cfg: {cfg:?}");
        let register = scx.extends.hook_mgr().register();
        let cfg = Arc::new(RwLock::new(cfg));

        #[cfg(any(feature = "sled", feature = "redis"))]
        let mut serve_state: Option<ServeState> = None;

        let retainer = {
            let cfg_guard = cfg.read().await;
            let batch_messages_limit = cfg_guard.batch_messages_limit;
            // Build the circuit breaker here (before cfg.write() below) to
            // avoid deadlock: tokio::sync::RwLock is NOT reentrant, and
            // cfg.write() is held inside the match block.
            let circuit_breaker = CircuitBreaker::new(CircuitBreakerConfig {
                enabled: cfg_guard.circuit_breaker_enabled,
                failure_threshold: cfg_guard.circuit_failure_threshold,
                reset_timeout: cfg_guard.circuit_reset_timeout,
                half_open_success_threshold: cfg_guard.circuit_half_open_success_threshold,
            });
            drop(cfg_guard);

            match &mut cfg.write().await.storage {
                #[cfg(feature = "ram")]
                Some(Config::Ram) => Retainer::Ram(RamRetainer::new(cfg.clone())),
                #[cfg(any(feature = "sled", feature = "redis"))]
                Some(Config::Storage(s_cfg)) => {
                    let node_id = scx.node.id();
                    match s_cfg.typ {
                        #[cfg(feature = "sled")]
                        StorageType::Sled => {
                            s_cfg.sled.path = s_cfg.sled.path.replace("{node}", &format!("{node_id}"));
                        }
                        #[cfg(feature = "redis")]
                        StorageType::Redis => {
                            s_cfg.redis.prefix = s_cfg.redis.prefix.replace("{node}", &format!("{node_id}"));
                        }
                        #[allow(unreachable_patterns)]
                        _ => return Err(anyhow::anyhow!("unsupported storage type")),
                    };
                    let storage_db = init_db(s_cfg).await?;
                    let exec = scx.get_exec(("RETAINER_EXEC", 1000, 10_000));
                    let (r, msg_rx, sync_rx) = storage::Retainer::new(
                        node_id,
                        cfg.clone(),
                        storage_db,
                        s_cfg.typ.clone(),
                        exec,
                        circuit_breaker,
                    )
                    .await?;
                    serve_state = Some(ServeState { msg_rx, sync_rx, batch_messages_limit });
                    Retainer::Storage(r)
                }
                #[allow(unreachable_patterns)]
                Some(other) => {
                    return Err(anyhow!("Unsupported storage engine: {other:?} (feature not enabled)"))
                }
                None => return Err(anyhow!("No storage engine specified (ram, sled, or redis)")),
            }
        };

        Ok(Self {
            scx,
            register,
            cfg,
            retainer,
            #[cfg(any(feature = "sled", feature = "redis"))]
            serve_state,
        })
    }
}

#[async_trait]
impl Plugin for RetainerPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());

        // Start bg message processing tasks (deferred from new()).
        #[cfg(any(feature = "sled", feature = "redis"))]
        if let Some(state) = self.serve_state.take() {
            if let Retainer::Storage(r) = &self.retainer {
                r.serve(state.msg_rx, state.sync_rx, state.batch_messages_limit);
            }
        }

        // Rebuild topics tree from storage (startup-time initialization).
        // This must complete before start() exposes RetainStorage to other plugins.
        self.retainer.rebuild_topics().await?;

        let _retainer = self.retainer.clone();
        //I run every 10 seconds
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(10)).await;
                let removeds = _retainer.remove_expired_messages().await;
                if removeds > 0 {
                    log::debug!(
                        "{:?} remove_expired_messages, removed count: {}",
                        std::thread::current().id(),
                        removeds
                    );
                }
            }
        });

        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.scx.plugins.read_config::<PluginConfig>(self.name())?;
        *self.cfg.write().await = new_cfg;
        log::debug!("load_config ok,  {:?}", self.cfg);
        Ok(())
    }

    #[inline]
    #[cfg_attr(
        not(any(feature = "ram", feature = "sled", feature = "redis")),
        allow(unused, unreachable_code)
    )]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name());
        let r: Box<dyn RetainStorage> = match self.retainer.clone() {
            #[cfg(feature = "ram")]
            Retainer::Ram(r) => Box::new(r),
            #[cfg(any(feature = "sled", feature = "redis"))]
            Retainer::Storage(r) => Box::new(r),
        };
        *self.scx.extends.retain_mut().await = r;
        self.register.start().await;

        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::warn!("{} stop, the default Retainer plug-in, it cannot be stopped", self.name());
        //self.register.stop().await;
        Ok(false)
    }

    #[inline]
    async fn attrs(&self) -> serde_json::Value {
        self.retainer.info().await
    }
}

#[derive(Clone)]
enum Retainer {
    #[cfg(feature = "ram")]
    Ram(RamRetainer),
    #[cfg(any(feature = "sled", feature = "redis"))]
    Storage(storage::Retainer),
}

/// Deferred serve state: holds channel receivers between new() and init().
#[cfg(any(feature = "sled", feature = "redis"))]
struct ServeState {
    msg_rx: mpsc::Receiver<Msg>,
    sync_rx: mpsc::Receiver<SyncMsg>,
    batch_messages_limit: usize,
}

impl Retainer {
    async fn rebuild_topics(&self) -> Result<()> {
        match self {
            #[cfg(feature = "ram")]
            Retainer::Ram(_r) => Ok(()),
            #[cfg(any(feature = "sled", feature = "redis"))]
            Retainer::Storage(r) => r.rebuild_topics().await,
            #[allow(unreachable_patterns)]
            _ => Ok(()),
        }
    }

    async fn remove_expired_messages(&self) -> usize {
        match self {
            #[cfg(feature = "ram")]
            Retainer::Ram(r) => r.remove_expired_messages().await,
            #[cfg(any(feature = "sled", feature = "redis"))]
            Retainer::Storage(_r) => 0,
            #[allow(unreachable_patterns)]
            _ => 0,
        }
    }

    #[allow(unused_mut)]
    async fn info(&self) -> serde_json::Value {
        match self {
            #[cfg(feature = "ram")]
            Retainer::Ram(r) => {
                let msg_max = r.max().await;
                let msg_count = r.count().await;
                let topic_nodes = r.inner.messages.read().await.nodes_size();
                let topic_values = r.inner.messages.read().await.values_size();
                json!({
                    "storage_engine": "Ram",
                    "message": {
                        "max": msg_max,
                        "count": msg_count,
                        "topic_nodes": topic_nodes,
                        "topic_values": topic_values,
                    },
                })
            }
            #[cfg(any(feature = "sled", feature = "redis"))]
            Retainer::Storage(r) => {
                let msg_max = r.max().await;
                let msg_count = r.count().await;
                let storage_info = r.info().await;
                let topics_size = r.topics.read().await.values_size();
                let cb_state = format!("{:?}", r.circuit_breaker.state());
                let cb_enabled = r.circuit_breaker.config().enabled;
                let cb_failure_count = r.circuit_breaker.failure_count();
                let cb_success_count = r.circuit_breaker.success_count();
                let mut info = json!({
                    "storage_info": storage_info,
                    "topics_size": topics_size,
                    "circuit_breaker": {
                        "success_count": cb_success_count,
                        "failure_count": cb_failure_count,
                        "state": cb_state,
                        "enabled": cb_enabled,
                    },
                    "message": {
                        "max": msg_max,
                        "count": msg_count,
                    },
                });

                #[cfg(feature = "rate-counter")]
                {
                    info.as_object_mut().unwrap().insert("set_rate".into(), r.set_rate_counter.to_json());
                    info.as_object_mut().unwrap().insert("sync_rate".into(), r.sync_rate_counter.to_json());
                }

                info
            }
            #[allow(unreachable_patterns)]
            _ => json!({ "storage_engine": "unknown" }),
        }
    }
}
