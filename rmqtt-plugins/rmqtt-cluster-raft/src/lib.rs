#[macro_use]
extern crate serde;
#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate async_trait;

mod config;
mod handler;
mod message;
mod retainer;
mod router;
mod shared;

use futures::FutureExt;
use parking_lot::RwLock;
use std::convert::From as _f;
use std::sync::Arc;
use std::time::Duration;

use rmqtt_raft::{Mailbox, Raft};

use rmqtt::{
    broker::{
        error::MqttError,
        hook::{Register, Type},
        types::{DashMap, From, NodeId, Publish, Reason, To},
    },
    grpc::{client::NodeGrpcClient, Message, MessageReply, MessageType},
    plugin::Plugin,
    Result, Runtime,
};

use config::PluginConfig;
use handler::HookHandler;
use retainer::ClusterRetainer;
use router::ClusterRouter;
use shared::ClusterShared;

pub(crate) type GrpcClients = Arc<DashMap<NodeId, NodeGrpcClient>>;

#[inline]
pub async fn init<N: Into<String>, D: Into<String>>(
    runtime: &'static Runtime,
    name: N,
    descr: D,
    default_startup: bool,
) -> Result<()> {
    runtime
        .plugins
        .register(Box::new(ClusterPlugin::new(runtime, name.into(), descr.into()).await?), default_startup)
        .await?;
    Ok(())
}

struct ClusterPlugin {
    runtime: &'static Runtime,
    name: String,
    descr: String,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
    grpc_clients: GrpcClients,
    shared: &'static ClusterShared,
    retainer: &'static ClusterRetainer,

    router: &'static ClusterRouter,
    raft_mailbox: Mailbox,
}

impl ClusterPlugin {
    #[inline]
    async fn new(runtime: &'static Runtime, name: String, descr: String) -> Result<Self> {
        let cfg = Arc::new(RwLock::new(
            runtime
                .settings
                .plugins
                .load_config::<PluginConfig>(&name)
                .map_err(|e| MqttError::from(e.to_string()))?,
        ));
        log::debug!("{} ClusterPlugin cfg: {:?}", name, cfg.read());

        let register = runtime.extends.hook_mgr().await.register();
        let grpc_clients = DashMap::default();
        for node_addr in cfg.read().node_grpc_addrs.iter() {
            if node_addr.id != runtime.node.id() {
                grpc_clients.insert(node_addr.id, runtime.node.new_grpc_client(&node_addr.addr).await?);
            }
        }
        let grpc_clients = Arc::new(grpc_clients);
        let message_type = cfg.read().message_type;
        let router = ClusterRouter::get_or_init();
        let shared = ClusterShared::get_or_init(router, grpc_clients.clone(), message_type);
        let retainer = ClusterRetainer::get_or_init(grpc_clients.clone(), message_type);
        let raft_mailbox = Self::start_raft(cfg.clone(), router).await;
        router.set_raft_mailbox(raft_mailbox.clone()).await;

        Ok(Self { runtime, name, descr, register, cfg, grpc_clients, shared, retainer, router, raft_mailbox })
    }

    //raft init ...
    async fn start_raft(cfg: Arc<RwLock<PluginConfig>>, router: &'static ClusterRouter) -> Mailbox {
        let id = Runtime::instance().node.id();
        let raft_addr = cfg
            .read()
            .raft_peer_addrs
            .iter()
            .find(|peer| peer.id == id)
            .map(|peer| peer.addr.to_string())
            .expect("raft listening address does not exist");
        let logger = Runtime::instance().logger.clone();
        let raft = Raft::new(raft_addr, router, logger);
        let mailbox = raft.mailbox();

        let peer_addrs = cfg
            .read()
            .raft_peer_addrs
            .iter()
            .filter_map(|peer| if peer.id != id { Some(peer.addr.to_string()) } else { None })
            .collect();

        log::info!("peer_addrs: {:?}", peer_addrs);

        let leader_info =
            raft.find_leader_info(peer_addrs).await.map_err(|e| MqttError::Error(Box::new(e))).unwrap();
        log::info!("leader_info: {:?}", leader_info);

        let _child = std::thread::Builder::new().name("cluster-raft".to_string()).spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .worker_threads(3)
                .thread_name("cluster-raft-worker")
                .thread_stack_size(4 * 1024 * 1024)
                .build()
                .unwrap();

            let runner = async move {
                let id = Runtime::instance().node.id();
                let raft_handle = match leader_info {
                    Some((leader_id, leader_addr)) => {
                        log::info!(
                            "running in follower mode, leader_id: {}, leader_addr: {}",
                            leader_id,
                            leader_addr
                        );
                        tokio::spawn(raft.join(id, Some(leader_id), leader_addr))
                    }
                    None => {
                        log::info!("running in leader mode");
                        tokio::spawn(raft.lead(id))
                    }
                };
                let _ = raft_handle.await.unwrap().unwrap();
            };

            rt.block_on(runner);
            log::info!("exit cluster raft worker.");
        });

        mailbox
    }

    #[inline]
    async fn hook_register(&self, typ: Type) {
        self.register
            .add(typ, Box::new(HookHandler::new(self.shared, self.retainer, self.raft_mailbox.clone())))
            .await;
    }
}

#[async_trait]
impl Plugin for ClusterPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name);

        self.hook_register(Type::ClientConnected).await;
        self.hook_register(Type::ClientDisconnected).await;
        self.hook_register(Type::SessionTerminated).await;
        self.hook_register(Type::GrpcMessageReceived).await;

        Ok(())
    }

    #[inline]
    fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name);
        let raft_mailbox = self.raft_mailbox.clone();
        *self.runtime.extends.router_mut().await = Box::new(self.router);
        *self.runtime.extends.shared_mut().await = Box::new(self.shared);
        *self.runtime.extends.retain_mut().await = Box::new(self.retainer);
        // *self.runtime.extends.shared_subscriber_mut().await = Box::new(self.shared_subscriber);
        self.register.start().await;

        for i in 0..10 {
            let status = raft_mailbox.status().await.map_err(anyhow::Error::new)?;
            log::info!("raft status({}): {:?}", i, status);
            if status.is_started() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::warn!("{} stop, once the cluster is started, it cannot be stopped", self.name);
        Ok(false)
    }

    #[inline]
    fn version(&self) -> &str {
        "0.1.1"
    }

    #[inline]
    fn descr(&self) -> &str {
        &self.descr
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().to_json()
    }

    #[inline]
    async fn attrs(&self) -> serde_json::Value {
        let raft_status = self.raft_mailbox.status().await.ok();

        let mut nodes = std::collections::HashMap::new();
        for entry in self.grpc_clients.iter() {
            let (node_id, c) = entry.pair();
            let stats = json!({
                "channel_tasks": c.channel_tasks(),
                "active_tasks": c.active_tasks(),
            });
            nodes.insert(format!("{}", node_id), stats);
        }
        json!({
            "grpc_clients": nodes,
            "raft_status": raft_status,
        })
    }
}

pub(crate) struct MessageSender {
    client: NodeGrpcClient,
    msg_type: MessageType,
    msg: Message,
    max_retries: usize,
    retry_interval: Duration,
}

impl MessageSender {
    async fn send(&mut self) -> Result<MessageReply> {
        let mut current_retry = 0usize;
        loop {
            match self.client.send_message(self.msg_type, self.msg.clone()).await {
                Ok(reply) => {
                    return Ok(reply);
                }
                Err(e) => {
                    if current_retry < self.max_retries {
                        current_retry += 1;
                        tokio::time::sleep(self.retry_interval).await;
                    } else {
                        log::error!("error sending message after {} retries, {:?}", self.max_retries, e);
                        return Err(e);
                    }
                }
            }
        }
    }
}

pub struct MessageBroadcaster {
    grpc_clients: GrpcClients,
    msg_type: MessageType,
    msg: Option<Message>,
}

impl MessageBroadcaster {
    pub fn new(grpc_clients: GrpcClients, msg_type: MessageType, msg: Message) -> Self {
        Self { grpc_clients, msg_type, msg: Some(msg) }
    }

    #[inline]
    pub async fn join_all(&mut self) -> Vec<Result<MessageReply>> {
        let msg_type = self.msg_type;
        let msg = self.msg.take().unwrap();
        let mut senders = Vec::new();
        let max_idx = self.grpc_clients.len() - 1;
        for (i, grpc_client) in self.grpc_clients.iter().enumerate() {
            let grpc_client = grpc_client.value().clone();
            if i == max_idx {
                let fut = async move { grpc_client.send_message(msg_type, msg).await };
                senders.push(fut.boxed());
                break;
            } else {
                let msg = msg.clone();
                let fut = async move { grpc_client.send_message(msg_type, msg).await };
                senders.push(fut.boxed());
            }
        }
        futures::future::join_all(senders).await
    }

    //     #[inline]
    //     pub async fn kick(&mut self) -> Result<SessionOfflineInfo> {
    //         let reply = self
    //             .select_ok(&|reply: MessageReply| -> Result<MessageReply> {
    //                 log::debug!("reply: {:?}", reply);
    //                 if let MessageReply::Kick(Some(o)) = reply {
    //                     Ok(MessageReply::Kick(Some(o)))
    //                 } else {
    //                     Err(MqttError::None)
    //                 }
    //             })
    //             .await?;
    //         if let MessageReply::Kick(Some(kicked)) = reply {
    //             Ok(kicked)
    //         } else {
    //             Err(MqttError::None)
    //         }
    //     }
    //
    //     #[inline]
    //     pub async fn select_ok<F: Fn(MessageReply) -> Result<MessageReply> + Send + Sync>(
    //         &mut self,
    //         check_fn: &F,
    //     ) -> Result<MessageReply> {
    //         let msg = self.msg.take().unwrap();
    //         let mut senders = Vec::new();
    //         let max_idx = self.grpc_clients.len() - 1;
    //         for (i, (_, grpc_client)) in self.grpc_clients.iter().enumerate() {
    //             if i == max_idx {
    //                 senders.push(Self::send(grpc_client, self.msg_type, msg, check_fn).boxed());
    //                 break;
    //             } else {
    //                 senders.push(Self::send(grpc_client, self.msg_type, msg.clone(), check_fn).boxed());
    //             }
    //         }
    //         let (reply, _) = futures::future::select_ok(senders).await?;
    //         Ok(reply)
    //     }
    //
    //     async fn send<F: Fn(MessageReply) -> Result<MessageReply> + Send + Sync>(
    //         grpc_client: &NodeGrpcClient,
    //         typ: MessageType,
    //         msg: Message,
    //         check_fn: &F,
    //     ) -> Result<MessageReply> {
    //         match grpc_client.send_message(typ, msg).await {
    //             Ok(r) => {
    //                 log::debug!("OK reply: {:?}", r);
    //                 check_fn(r)
    //             }
    //             Err(e) => {
    //                 log::debug!("ERROR reply: {:?}", e);
    //                 Err(e)
    //             }
    //         }
    //     }
}

pub(crate) async fn hook_message_dropped(droppeds: Vec<(To, From, Publish, Reason)>) {
    for (to, from, publish, reason) in droppeds {
        //hook, message_dropped
        Runtime::instance().extends.hook_mgr().await.message_dropped(Some(to), from, publish, reason).await;
    }
}
