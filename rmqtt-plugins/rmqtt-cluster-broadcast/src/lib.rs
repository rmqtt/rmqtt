//! Broadcast cluster plugin for RMQTT.
//!
//! Implements cluster communication via a simpler broadcast-based
//! approach (without Raft consensus). Messages are forwarded to
//! all peer nodes via gRPC.
//!
//! # Architecture
//!
//! - Each node maintains gRPC connections to all peers.
//! - Incoming MQTT events are broadcast to every peer node.
//! - No distributed consensus — eventual consistency model.
//! - Lighter weight than the Raft-based cluster plugin.
//!
#![deny(unsafe_code)]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{self, json};
use tokio::sync::RwLock;

use rmqtt::{
    grpc::{GrpcClients, Message, MessageReply, MessageType},
    hook::{Register, Type},
    macros::Plugin,
    plugin::{PackageInfo, Plugin},
    register,
    types::{From, OfflineSession, Publish, Reason, To},
    Result,
};

use config::PluginConfig;
use handler::HookHandler;
use rmqtt::context::ServerContext;
use rmqtt::net::MqttError;
use router::ClusterRouter;
use shared::ClusterShared;

mod config;
mod handler;
mod message;
mod router;
mod shared;

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

register!(ClusterPlugin::new);

#[derive(Plugin)]
struct ClusterPlugin {
    scx: ServerContext,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
    grpc_clients: GrpcClients,
    shared: ClusterShared,
    router: ClusterRouter,
}

impl ClusterPlugin {
    #[inline]
    async fn new<S: Into<String>>(scx: ServerContext, name: S) -> Result<Self> {
        let name = name.into();
        let cfg = scx.plugins.read_config_with::<PluginConfig>(&name, &["node_grpc_addrs"])?;
        log::debug!("{name} ClusterPlugin cfg: {cfg:?}");

        let register = scx.extends.hook_mgr().register();
        let mut grpc_clients = HashMap::default();
        let mut node_names = HashMap::default();
        let node_grpc_addrs = cfg.node_grpc_addrs.clone();

        // Read circuit-breaker config from ServerContext.
        let cb_config = &scx.circuit_breaker_config;

        for node_addr in &node_grpc_addrs {
            if node_addr.id != scx.node.id() {
                let batch_size = cfg.node_grpc_batch_size;
                let client_concurrency_limit = cfg.node_grpc_client_concurrency_limit;
                let client_timeout = cfg.node_grpc_client_timeout;
                grpc_clients.insert(
                    node_addr.id,
                    (
                        node_addr.addr.clone(),
                        scx.node
                            .new_grpc_client(
                                &node_addr.addr,
                                client_timeout,
                                client_concurrency_limit,
                                batch_size,
                                cb_config,
                            )
                            .await?,
                    ),
                );
            }
            node_names.insert(node_addr.id, format!("{}@{}", node_addr.id, node_addr.addr));
        }
        let grpc_clients = Arc::new(grpc_clients);
        let message_type = cfg.message_type;
        let exec = scx.get_exec(("BC_EXEC", cfg.task_exec_queue_workers, cfg.task_exec_queue_max));
        let router =
            ClusterRouter::new(scx.clone(), grpc_clients.clone(), message_type, cfg.node_grpc_client_timeout);
        let shared = ClusterShared::new(
            scx.clone(),
            exec,
            grpc_clients.clone(),
            message_type,
            node_names,
            cfg.task_exec_queue_max,
            cfg.task_exec_queue_workers,
            cfg.node_grpc_client_timeout,
        );
        let cfg = Arc::new(RwLock::new(cfg));
        Ok(Self { scx, register, cfg, grpc_clients, shared, router })
    }
}

#[async_trait]
impl Plugin for ClusterPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        self.register
            .add(
                Type::GrpcMessageReceived,
                Box::new(HookHandler::new(self.scx.clone(), self.shared.clone(), self.router.clone())),
            )
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
        *self.scx.extends.shared_mut().await = Box::new(self.shared.clone());
        *self.scx.extends.router_mut().await = Box::new(self.router.clone());

        // Async retain sync from peers (fire-and-forget)
        let shared = self.shared.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            shared.sync_retains_from_peers().await;
        });

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
                "transfer_queue_len": c.transfer_queue_len(),
                "active_tasks_count": c.active_tasks().count(),
                "active_tasks_max": c.active_tasks().max(),
                "circuit_breaker": c.circuit_breaker_json().await,
            });
            nodes.insert(format!("{id}-{addr}"), stats);
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
                    Err(MqttError::None.into())
                }
            })
            .await?;
    if let MessageReply::Kick(kicked) = reply {
        Ok(kicked)
    } else {
        Err(MqttError::None.into())
    }
}

pub(crate) async fn hook_message_dropped(scx: &ServerContext, droppeds: Vec<(To, From, Publish, Reason)>) {
    for (to, from, publish, reason) in droppeds {
        //hook, message_dropped
        scx.extends.hook_mgr().message_dropped(Some(to), from, publish, reason).await;
    }
}
