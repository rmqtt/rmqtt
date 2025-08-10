#![allow(clippy::result_large_err)]
#![deny(unsafe_code)]
#[macro_use]
extern crate serde;

#[macro_use]
extern crate rmqtt_macros;

use std::sync::Arc;
use std::time::Duration;

use config::PluginConfig;
use handler::HookHandler;
use rmqtt::{
    ahash,
    async_trait::async_trait,
    log,
    serde_json::{self, json},
    tokio::sync::RwLock,
};
use rmqtt::{
    broker::{
        error::MqttError,
        hook::{Register, Type},
        types::{From, OfflineSession, Publish, Reason, To},
    },
    grpc::{GrpcClients, Message, MessageReply, MessageType},
    plugin::{PackageInfo, Plugin},
    register, Result, Runtime,
};
use router::ClusterRouter;
use shared::ClusterShared;

mod config;
mod handler;
mod router;
mod shared;

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

register!(ClusterPlugin::new);

#[derive(Plugin)]
struct ClusterPlugin {
    runtime: &'static Runtime,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
    grpc_clients: GrpcClients,
    shared: &'static ClusterShared,
    router: &'static ClusterRouter,
}

impl ClusterPlugin {
    #[inline]
    async fn new<S: Into<String>>(runtime: &'static Runtime, name: S) -> Result<Self> {
        let name = name.into();
        let cfg = Arc::new(RwLock::new(
            runtime.settings.plugins.load_config_with::<PluginConfig>(&name, &["node_grpc_addrs"])?,
        ));
        log::debug!("{} ClusterPlugin cfg: {:?}", name, cfg.read().await);

        let register = runtime.extends.hook_mgr().await.register();
        let mut grpc_clients = HashMap::default();
        let node_grpc_addrs = cfg.read().await.node_grpc_addrs.clone();
        for node_addr in &node_grpc_addrs {
            if node_addr.id != runtime.node.id() {
                grpc_clients.insert(
                    node_addr.id,
                    (node_addr.addr.clone(), runtime.node.new_grpc_client(&node_addr.addr).await?),
                );
            }
        }
        let grpc_clients = Arc::new(grpc_clients);
        let message_type = cfg.read().await.message_type;
        let router = ClusterRouter::get_or_init(grpc_clients.clone(), message_type);
        let shared = ClusterShared::get_or_init(grpc_clients.clone(), message_type);
        Ok(Self { runtime, register, cfg, grpc_clients, shared, router })
    }
}

#[async_trait]
impl Plugin for ClusterPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        self.register
            .add(Type::GrpcMessageReceived, Box::new(HookHandler::new(self.shared, self.router)))
            .await;
        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name());
        self.register.start().await;
        *self.runtime.extends.shared_mut().await = Box::new(self.shared);
        *self.runtime.extends.router_mut().await = Box::new(self.router);
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::warn!("{} stop, once the cluster is started, it cannot be stopped", self.name());
        Ok(false)
    }

    #[inline]
    async fn attrs(&self) -> serde_json::Value {
        let mut nodes = HashMap::default();
        for (id, (addr, c)) in self.grpc_clients.iter() {
            let stats = json!({
                "channel_tasks": c.channel_tasks(),
                "active_tasks": c.active_tasks(),
            });
            nodes.insert(format!("{id}/{addr:?}"), stats);
        }
        json!({
            "grpc_clients": nodes,
        })
    }
}

#[inline]
pub(crate) async fn kick(
    grpc_clients: GrpcClients,
    msg_type: MessageType,
    msg: Message,
) -> Result<OfflineSession> {
    let reply =
        rmqtt::grpc::MessageBroadcaster::new(grpc_clients, msg_type, msg, Some(Duration::from_secs(15)))
            .select_ok(|reply: MessageReply| -> Result<MessageReply> {
                log::debug!("reply: {reply:?}");
                if let MessageReply::Kick(o) = reply {
                    Ok(MessageReply::Kick(o))
                } else {
                    Err(MqttError::None)
                }
            })
            .await?;
    if let MessageReply::Kick(kicked) = reply {
        Ok(kicked)
    } else {
        Err(MqttError::None)
    }
}

pub(crate) async fn hook_message_dropped(droppeds: Vec<(To, From, Publish, Reason)>) {
    for (to, from, publish, reason) in droppeds {
        //hook, message_dropped
        Runtime::instance().extends.hook_mgr().await.message_dropped(Some(to), from, publish, reason).await;
    }
}
