#![deny(unsafe_code)]
#[macro_use]
extern crate serde;

#[macro_use]
extern crate rmqtt_macros;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::config::Config;
use crate::ram::RamRetainer;
use config::PluginConfig;
use rmqtt::{
    async_trait::async_trait,
    log,
    serde_json::{self, json},
    tokio::sync::RwLock,
    tokio_cron_scheduler::Job,
    MqttError,
};
use rmqtt::{
    broker::hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    broker::RetainStorage,
    plugin::{PackageInfo, Plugin},
    register, Result, Runtime,
};
use rmqtt_storage::{init_db, StorageType};

mod config;
mod ram;
mod storage;

register!(RetainerPlugin::new);

#[derive(Plugin)]
struct RetainerPlugin {
    runtime: &'static Runtime,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
    retainer: Retainer,
    support_cluster: bool,
    retain_enable: Arc<AtomicBool>,
}

impl RetainerPlugin {
    #[inline]
    async fn new<N: Into<String>>(runtime: &'static Runtime, name: N) -> Result<Self> {
        let name = name.into();
        let node_id = runtime.node.id();
        let cfg = runtime.settings.plugins.load_config::<PluginConfig>(&name)?;
        log::info!("{} RetainerPlugin cfg: {:?}", name, cfg);
        let register = runtime.extends.hook_mgr().await.register();
        let cfg = Arc::new(RwLock::new(cfg));
        let retain_enable = Arc::new(AtomicBool::new(false));

        let (retainer, support_cluster) = match &mut cfg.write().await.storage {
            Config::Ram => {
                (Retainer::Ram(RamRetainer::get_or_init(cfg.clone(), retain_enable.clone())), false)
            }
            Config::Storage(s_cfg) => {
                let support_cluster = match s_cfg.typ {
                    StorageType::Sled => {
                        s_cfg.sled.path =
                            s_cfg.sled.path.replace("{node}", &format!("{}", runtime.node.id()));
                        false
                    }
                    StorageType::Redis => {
                        s_cfg.redis.prefix =
                            s_cfg.redis.prefix.replace("{node}", &format!("{}", runtime.node.id()));
                        true
                    }
                    #[allow(unreachable_patterns)]
                    _ => return Err(MqttError::from("unsupported storage type")),
                };
                let storage_db = init_db(s_cfg).await?;
                (
                    Retainer::Storage(
                        storage::get_or_init(node_id, cfg.clone(), storage_db, retain_enable.clone()).await?,
                    ),
                    support_cluster,
                )
            }
        };

        Ok(Self { runtime, register, cfg, retainer, support_cluster, retain_enable })
    }
}

#[async_trait]
impl Plugin for RetainerPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        self.register
            .add(
                Type::BeforeStartup,
                Box::new(RetainHandler::new(self.support_cluster, self.retain_enable.clone())),
            )
            .await;

        let retainer = self.retainer;
        //"0 1/10 * * * *"
        let async_jj = Job::new_async("1/10 * * * * *", move |_uuid, _l| {
            Box::pin(async move {
                let removeds = retainer.remove_expired_messages().await;
                if removeds > 0 {
                    log::info!(
                        "{:?} remove_expired_messages, removed count: {}",
                        std::thread::current().id(),
                        removeds
                    );
                }
            })
        })
        .unwrap();
        self.runtime.sched.add(async_jj).await.unwrap();

        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.runtime.settings.plugins.load_config::<PluginConfig>(self.name())?;
        *self.cfg.write().await = new_cfg;
        log::debug!("load_config ok,  {:?}", self.cfg);
        Ok(())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name());
        let r: Box<dyn RetainStorage> = match self.retainer {
            Retainer::Ram(r) => Box::new(r),
            Retainer::Storage(r) => Box::new(r),
        };
        *self.runtime.extends.retain_mut().await = r;
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

struct RetainHandler {
    support_cluster: bool,
    retain_enable: Arc<AtomicBool>,
}

impl RetainHandler {
    fn new(support_cluster: bool, retain_enable: Arc<AtomicBool>) -> Self {
        Self { support_cluster, retain_enable }
    }
}

#[async_trait]
impl Handler for RetainHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::BeforeStartup => {
                let grpc_clients = Runtime::instance().extends.shared().await.get_grpc_clients();
                log::info!("grpc_clients len: {}", grpc_clients.len());
                if !grpc_clients.is_empty() && !self.support_cluster {
                    log::error!("{}", ERR_NOT_SUPPORTED);
                    self.retain_enable.store(false, Ordering::SeqCst);
                } else {
                    self.retain_enable.store(true, Ordering::SeqCst);
                }
            }
            _ => {
                log::error!("unimplemented, {:?}", param)
            }
        }
        (true, acc)
    }
}

#[derive(Clone, Copy)]
enum Retainer {
    Ram(&'static RamRetainer),
    Storage(&'static storage::Retainer),
}

impl Retainer {
    async fn remove_expired_messages(&self) -> usize {
        match self {
            Retainer::Ram(r) => r.remove_expired_messages().await,
            Retainer::Storage(_r) => 0,
        }
    }

    async fn info(&self) -> serde_json::Value {
        match self {
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
            Retainer::Storage(r) => {
                let msg_max = r.max().await;
                let msg_count = r.count().await;
                let msg_queue_count = r.msg_queue_count.load(Ordering::Relaxed);
                let storage_info = r.storage_db.info().await.unwrap_or_default();
                json!({
                    "storage_info": storage_info,
                    "msg_queue_count": msg_queue_count,
                    "message": {
                        "max": msg_max,
                        "count": msg_count,
                    },
                })
            }
        }
    }
}

pub(crate) const ERR_NOT_SUPPORTED: &str =
    "The storage engine of the 'rmqtt-retainer' plugin does not support cluster mode!";
