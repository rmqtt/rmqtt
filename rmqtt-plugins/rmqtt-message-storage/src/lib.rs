#![deny(unsafe_code)]

use rmqtt::{
    async_trait::async_trait,
    log,
    serde_json::{self, json},
};

use rmqtt::{
    broker::hook::Register,
    plugin::{DynPlugin, DynPluginResult, Plugin},
    Result, Runtime,
};

use rmqtt_storage::{init_db, Config, List, StorageType};

use message::{get_or_init, StorageMessageManager};

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
    // cfg: Arc<Config>,
    // storage_db: DefaultStorageDB,
    register: Box<dyn Register>,
    message_mgr: &'static StorageMessageManager,
}

impl StoragePlugin {
    #[inline]
    async fn new<S: Into<String>>(runtime: &'static Runtime, name: S, descr: S) -> Result<Self> {
        let name = name.into();
        let mut cfg = runtime.settings.plugins.load_config_default::<Config>(&name)?;
        match cfg.storage_type {
            StorageType::Sled => {
                cfg.sled.path = cfg.sled.path.replace("{node}", &format!("{}", runtime.node.id()));
            }
            StorageType::Redis => {
                cfg.redis.prefix = cfg.redis.prefix.replace("{node}", &format!("{}", runtime.node.id()));
            }
        }
        log::info!("{} StoragePlugin cfg: {:?}", name, cfg);

        let storage_db = init_db(&cfg).await?;

        let register = runtime.extends.hook_mgr().await.register();
        let message_mgr = get_or_init(storage_db.clone()).await?;
        // let cfg = Arc::new(cfg);
        Ok(Self { runtime, name, descr: descr.into(), register, message_mgr })
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
        // let receiveds = self.message_mgr.messages_received_map.len().await.unwrap_or_default();
        let unexpireds = self.message_mgr.messages_unexpired_list.len().await.unwrap_or_default();
        // let forwardeds = self.message_mgr.messages_forwarded_map.len().await.unwrap_or_default();

        json!(
            {
                "message": {
                    // "receiveds": receiveds,
                    "unexpireds": unexpireds,
                    // "forwardeds": forwardeds,
                }
            }
        )
    }
}
