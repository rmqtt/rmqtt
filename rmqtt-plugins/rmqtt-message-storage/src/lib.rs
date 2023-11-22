#![deny(unsafe_code)]
#[macro_use]
extern crate serde;

use std::sync::Arc;

use rmqtt::{
    async_trait::async_trait,
    log,
    serde_json::{self, json},
    tokio,
    tokio::sync::RwLock,
};

use rmqtt::{
    broker::hook::Register,
    plugin::{DynPlugin, DynPluginResult, Plugin},
    Result, Runtime,
};

use config::PluginConfig;
use message::StorageMessageManager;
use store::{init_store_db, storage::Storage as _, StorageDB, StorageKV};

mod config;
mod message;
mod store;

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
    _cfg: Arc<RwLock<PluginConfig>>,
    _storage_db: StorageDB,

    //All received messages.
    #[allow(dead_code)]
    messages_received_kv: StorageKV,
    //All unexpired messages
    messages_unexpired_kv: StorageKV,
    //All forwarded messages
    #[allow(dead_code)]
    messages_forwarded_kv: StorageKV,

    register: Box<dyn Register>,
    message_mgr: &'static StorageMessageManager,
}

impl StoragePlugin {
    #[inline]
    async fn new<S: Into<String>>(runtime: &'static Runtime, name: S, descr: S) -> Result<Self> {
        let name = name.into();
        let cfg = runtime.settings.plugins.load_config_default::<PluginConfig>(&name)?;
        log::info!("{} StoragePlugin cfg: {:?}", name, cfg);

        let storage_db = init_store_db(&cfg)?;

        let messages_received_kv = storage_db.open("messages_received")?;
        log::info!("{} StoragePlugin open messages_received storage ok", name);
        let messages_unexpired_kv = storage_db.open("messages_unexpired")?;
        log::info!("{} StoragePlugin open messages_unexpired storage ok", name);
        let messages_forwarded_kv = storage_db.open("messages_forwarded")?;
        log::info!("{} StoragePlugin open messages_forwarded storage ok", name);

        let register = runtime.extends.hook_mgr().await.register();
        let message_mgr = StorageMessageManager::get_or_init(
            storage_db.clone(),
            messages_received_kv.clone(),
            messages_unexpired_kv.clone(),
            messages_forwarded_kv.clone(),
        );
        let cfg = Arc::new(RwLock::new(cfg));
        Ok(Self {
            runtime,
            name,
            descr: descr.into(),
            _cfg: cfg,
            _storage_db: storage_db,

            messages_received_kv,
            messages_unexpired_kv,
            messages_forwarded_kv,
            register,
            message_mgr,
        })
    }
}

#[async_trait]
impl Plugin for StoragePlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name);
        Ok(())
    }

    #[inline]
    fn name(&self) -> &str {
        &self.name
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
        let messages_received_kv = self.messages_received_kv.clone();
        let messages_unexpired_kv = self.messages_unexpired_kv.clone();
        let messages_forwarded_kv = self.messages_forwarded_kv.clone();

        let storage_db = self._storage_db.clone();

        tokio::task::spawn_blocking(move || {
            json!(
                {
                    "message": {
                        "receiveds": messages_received_kv.len(),
                        "unexpireds": messages_unexpired_kv.len(),
                        "forwardeds": messages_forwarded_kv.len(),
                    },
                    "size_on_disk": storage_db.size_on_disk().unwrap_or_default(),
                }
            )
        })
        .await
        .unwrap_or_default()
    }
}
