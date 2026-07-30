//! HTTP API plugin for RMQTT.
//!
//! Provides a RESTful HTTP API for broker management and monitoring.
//! ...
#![deny(unsafe_code)]

use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::{self, sync::oneshot, sync::RwLock};

use rmqtt::{
    context::ServerContext,
    hook::{Register, Type},
    macros::Plugin,
    plugin::{PackageInfo, Plugin},
    register, Result,
};

use config::PluginConfig;
use flusher::{new_cache, start_flusher, start_recovery_loop, start_warmup, HistoryCaches};

mod api;
mod clients;
mod config;
mod embed;
mod flusher;
mod handler;
mod plugin;
mod prome;
mod subs;
mod types;

type ShutdownTX = oneshot::Sender<()>;
type PluginConfigType = Arc<RwLock<PluginConfig>>;

register!(HttpApiPlugin::new);

#[derive(Plugin)]
struct HttpApiPlugin {
    scx: ServerContext,
    register: Box<dyn Register>,
    cfg: PluginConfigType,
    shutdown_tx: Option<ShutdownTX>,
    /// LRU caches — created in new(), populated in init().
    history_caches: Option<HistoryCaches>,
    /// Handle to abort background tasks on stop.
    flush_handle: Option<tokio::task::JoinHandle<()>>,
}

impl HttpApiPlugin {
    /// Plugin construction — happens for ALL registered plugins, whether enabled or not.
    /// Only parse config, create empty LRU caches, and start the HTTP server.
    /// Storage connection and background tasks are deferred to `init()`.
    #[inline]
    async fn new<S: Into<String>>(scx: ServerContext, name: S) -> Result<Self> {
        let name = name.into();
        let mut cfg = scx.plugins.read_config::<PluginConfig>(&name)?;
        log::info!("{name} HttpApiPlugin cfg: {cfg:?}");

        // Replace {node} placeholder in storage config paths (before init_db)
        if let Some(ref mut storage_cfg) = cfg.storage {
            let node_id_str = format!("{}", scx.node.id());
            match storage_cfg.typ {
                #[cfg(feature = "sled")]
                rmqtt_storage::StorageType::Sled => {
                    storage_cfg.sled.path = storage_cfg.sled.path.replace("{node}", &node_id_str);
                    log::info!("{name} sled storage path: {}", storage_cfg.sled.path);
                }
                #[cfg(feature = "redis")]
                rmqtt_storage::StorageType::Redis => {
                    storage_cfg.redis.prefix = storage_cfg.redis.prefix.replace("{node}", &node_id_str);
                }
                #[cfg(feature = "redis-cluster")]
                rmqtt_storage::StorageType::RedisCluster => {
                    storage_cfg.redis_cluster.prefix =
                        storage_cfg.redis_cluster.prefix.replace("{node}", &node_id_str);
                }
                #[cfg(feature = "redb")]
                rmqtt_storage::StorageType::Redb => {
                    storage_cfg.redb.path = storage_cfg.redb.path.replace("{node}", &node_id_str);
                    log::info!("{name} redb storage path: {}", storage_cfg.redb.path);
                }
                #[allow(unreachable_patterns)]
                _ => return Err(anyhow!("Unsupported storage engine config")),
            }
        }

        // Create empty LRU caches (always, for depot injection).
        // Data is filled in init() via warmup + flusher.
        let interval_secs = cfg.flush_interval.as_secs().max(1);
        let retention_secs = cfg.history_retention.as_secs().max(interval_secs);
        let capacity = (retention_secs / interval_secs) as usize;
        let stats_cache = new_cache(capacity)?;
        let metrics_cache = new_cache(capacity)?;
        let hc = HistoryCaches::new(stats_cache, metrics_cache);

        let cfg = Arc::new(RwLock::new(cfg));
        let register = scx.extends.hook_mgr().register();
        let shutdown_tx = Some(Self::start(scx.clone(), cfg.clone(), Some(hc.clone())).await?);
        Ok(Self { scx, register, cfg, shutdown_tx, history_caches: Some(hc), flush_handle: None })
    }

    async fn start(
        scx: ServerContext,
        cfg: PluginConfigType,
        history_caches: Option<HistoryCaches>,
    ) -> Result<ShutdownTX> {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (started_tx, started_rx) = oneshot::channel();
        let http_laddr = cfg.read().await.http_laddr;
        tokio::spawn(async move {
            if let Err(e) =
                api::listen_and_serve(scx, http_laddr, cfg, history_caches, shutdown_rx, started_tx).await
            {
                log::error!("{e}");
            }
            log::info!("Exit HTTP API Server, ..., http://{http_laddr:?}");
        });

        started_rx.await.map_err(|_| anyhow!("HTTP API server failed to bind on {http_laddr}"))?;
        Ok(shutdown_tx)
    }
}

#[async_trait]
impl Plugin for HttpApiPlugin {
    /// Called when the plugin is actually enabled.
    /// Connect to storage, start flusher / recovery / warmup, register gRPC handler.
    #[inline]
    async fn init(&mut self) -> Result<()> {
        let name = self.name();
        log::info!("{} init", name);

        // ── Initialise storage + background tasks (if configured) ──
        if let Some(ref storage_cfg) = self.cfg.read().await.storage.clone() {
            match rmqtt_storage::init_db(storage_cfg).await {
                Ok(db) => {
                    let node_id = self.scx.node.id();
                    let retention = self.cfg.read().await.history_retention;
                    let flush_intv = self.cfg.read().await.flush_interval;

                    // These are safe: history_caches was set in new()
                    let hc = self.history_caches.as_ref().unwrap();

                    let fh = start_flusher(
                        self.scx.clone(),
                        db.clone(),
                        hc.stats.clone(),
                        hc.metrics.clone(),
                        flush_intv,
                        retention,
                    );
                    let rh = start_recovery_loop(
                        hc.stats.clone(),
                        hc.metrics.clone(),
                        db.clone(),
                        retention,
                        node_id,
                    );
                    let wh =
                        start_warmup(hc.stats.clone(), hc.metrics.clone(), db.clone(), node_id, retention);

                    let combined = tokio::spawn(async move {
                        let _ = tokio::join!(fh, rh, wh);
                    });
                    self.flush_handle = Some(combined);
                }
                Err(e) => {
                    log::error!("{name} init storage db error, disabling history: {e}");
                }
            }
        }

        // ── Register gRPC message handler ──
        let mgs_type = self.cfg.read().await.message_type;
        let flush_ms = self.cfg.read().await.flush_interval.as_millis() as u64;
        self.register
            .add(
                Type::GrpcMessageReceived,
                Box::new(handler::HookHandler::new(
                    self.scx.clone(),
                    mgs_type,
                    self.history_caches.clone(),
                    flush_ms,
                )),
            )
            .await;
        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.scx.plugins.read_config::<PluginConfig>(self.name())?;
        if !self.cfg.read().await.changed(&new_cfg) {
            return Ok(());
        }
        let restart_enable = self.cfg.read().await.restart_enable(&new_cfg);
        if restart_enable {
            let new_cfg = Arc::new(RwLock::new(new_cfg));
            match Self::start(self.scx.clone(), new_cfg.clone(), self.history_caches.clone()).await {
                Ok(tx) => {
                    if let Some(old_tx) = self.shutdown_tx.take() {
                        let _ = old_tx.send(());
                    }
                    self.shutdown_tx = Some(tx);
                    self.cfg = new_cfg;
                }
                Err(e) => {
                    return Err(e);
                }
            }
        } else {
            *self.cfg.write().await = new_cfg;
        }
        log::debug!("load_config ok,  {:?}", self.cfg);
        Ok(())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name());
        self.register.start().await;
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::info!("{} stop", self.name());
        if let Some(handle) = self.flush_handle.take() {
            handle.abort();
        }
        Ok(false)
    }
}
