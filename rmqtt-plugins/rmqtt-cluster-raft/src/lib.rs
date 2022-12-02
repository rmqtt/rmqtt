#![deny(unsafe_code)]
#[macro_use]
extern crate serde;

use std::convert::From as _f;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use rmqtt_raft::{Mailbox, Raft};

use config::PluginConfig;
use handler::HookHandler;
use retainer::ClusterRetainer;
use rmqtt::{
    ahash, anyhow,
    async_trait::async_trait,
    futures, log,
    serde_json::{self, json},
    tokio, RwLock,
};
use rmqtt::{
    broker::{
        error::MqttError,
        hook::{Register, Type},
        types::{From, Publish, Reason, To},
    },
    grpc::{client::NodeGrpcClient, GrpcClients, Message, MessageReply, MessageType},
    plugin::{DynPlugin, DynPluginResult, Plugin},
    tokio::time::sleep,
    Result, Runtime,
};
use rmqtt::{
    once_cell::sync::OnceCell,
    rust_box::task_exec_queue::{Builder, TaskExecQueue},
};
use router::ClusterRouter;
use shared::ClusterShared;

mod config;
mod handler;
mod message;
mod retainer;
mod router;
mod shared;

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;
// pub(crate) type GrpcClients = Arc<DashMap<NodeId, NodeGrpcClient>>;

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
                ClusterPlugin::new(runtime, name, descr).await.map(|p| -> DynPlugin { Box::new(p) })
            })
        })
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
    raft_mailbox: Option<Mailbox>,
}

impl ClusterPlugin {
    #[inline]
    async fn new<S: Into<String>>(runtime: &'static Runtime, name: S, descr: S) -> Result<Self> {
        let name = name.into();
        let mut cfg = runtime
            .settings
            .plugins
            .load_config::<PluginConfig>(&name)
            .map_err(|e| MqttError::from(e.to_string()))?;
        log::info!("{} ClusterPlugin cfg: {:?}", name, cfg);
        cfg.merge(&runtime.settings.opts);

        init_task_exec_queue(cfg.task_exec_queue_workers, cfg.task_exec_queue_max);

        let register = runtime.extends.hook_mgr().await.register();
        let mut grpc_clients = HashMap::default();

        let node_grpc_addrs = cfg.node_grpc_addrs.clone();
        log::info!("node_grpc_addrs: {:?}", node_grpc_addrs);
        for node_addr in &node_grpc_addrs {
            if node_addr.id != runtime.node.id() {
                grpc_clients.insert(
                    node_addr.id,
                    (node_addr.addr.clone(), runtime.node.new_grpc_client(&node_addr.addr).await?),
                );
            }
        }
        let grpc_clients = Arc::new(grpc_clients);
        let router = ClusterRouter::get_or_init(cfg.try_lock_timeout);
        let shared = ClusterShared::get_or_init(router, grpc_clients.clone(), cfg.message_type);
        let retainer = ClusterRetainer::get_or_init(grpc_clients.clone(), cfg.message_type);
        let raft_mailbox = None;
        let cfg = Arc::new(RwLock::new(cfg));
        Ok(Self {
            runtime,
            name,
            descr: descr.into(),
            register,
            cfg,
            grpc_clients,
            shared,
            retainer,
            router,
            raft_mailbox,
        })
    }

    //raft init ...
    async fn start_raft(cfg: Arc<RwLock<PluginConfig>>, router: &'static ClusterRouter) -> Mailbox {
        let raft_peer_addrs = cfg.read().raft_peer_addrs.clone();

        let id = Runtime::instance().node.id();
        let raft_laddr = raft_peer_addrs
            .iter()
            .find(|peer| peer.id == id)
            .map(|peer| peer.addr.to_string())
            .expect("raft listening address does not exist");
        let logger = Runtime::instance().logger.clone();
        log::info!("raft_laddr: {:?}", raft_laddr);
        let raft = Raft::new(raft_laddr, router, logger, cfg.read().raft.to_raft_config());
        let mailbox = raft.mailbox();

        let peer_addrs = raft_peer_addrs
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
                .worker_threads(8)
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
                raft_handle.await.unwrap().unwrap();
            };

            rt.block_on(runner);
            log::info!("exit cluster raft worker.");
        });

        mailbox
    }

    #[inline]
    async fn hook_register(&self, typ: Type) {
        self.register
            .add(typ, Box::new(HookHandler::new(self.shared, self.retainer, self.raft_mailbox())))
            .await;
    }

    fn raft_mailbox(&self) -> Mailbox {
        if let Some(raft_mailbox) = &self.raft_mailbox {
            raft_mailbox.clone()
        } else {
            unreachable!()
        }
    }
}

#[async_trait]
impl Plugin for ClusterPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name);

        let raft_mailbox = Self::start_raft(self.cfg.clone(), self.router).await;

        for _ in 0..20 {
            match raft_mailbox.status().await {
                Ok(status) => {
                    if status.is_started() {
                        break;
                    }
                    log::info!("{} Initializing cluster", self.name);
                }
                Err(e) => {
                    log::info!("{} init error, {:?}", self.name, e);
                }
            }
            sleep(Duration::from_millis(300)).await;
        }

        self.raft_mailbox.replace(raft_mailbox.clone());
        self.router.set_raft_mailbox(raft_mailbox).await;

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
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().to_json()
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name);
        let raft_mailbox = self.raft_mailbox();
        *self.runtime.extends.router_mut().await = Box::new(self.router);
        *self.runtime.extends.shared_mut().await = Box::new(self.shared);
        *self.runtime.extends.retain_mut().await = Box::new(self.retainer);
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
    async fn attrs(&self) -> serde_json::Value {
        let raft_mailbox = self.raft_mailbox();
        let raft_status = raft_mailbox.status().await.ok();

        let mut pears = HashMap::default();
        for (id, p) in raft_mailbox.pears() {
            let stats = json!({
                "active_tasks": p.active_tasks(),
                "grpc_fails": p.grpc_fails(),
            });
            pears.insert(id, stats);
        }

        let mut nodes = HashMap::default();
        for (node_id, (_, c)) in self.grpc_clients.iter() {
            let stats = json!({
                "channel_tasks": c.channel_tasks(),
                "active_tasks": c.active_tasks(),
            });
            nodes.insert(*node_id, stats);
        }

        let exec = task_exec_queue();
        json!({
            "grpc_clients": nodes,
            "raft_status": raft_status,
            "raft_pears": pears,
            "client_states": self.router.states_count(),
            "task_exec_queue": {
                "waiting_count": exec.waiting_count(),
                "active_count": exec.active_count(),
                "completed_count": exec.completed_count(),
            }
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
    #[inline]
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
        for (i, (_, (_, grpc_client))) in self.grpc_clients.iter().enumerate() {
            let grpc_client = grpc_client.clone();
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
}

#[inline]
pub(crate) async fn hook_message_dropped(droppeds: Vec<(To, From, Publish, Reason)>) {
    for (to, from, publish, reason) in droppeds {
        //hook, message_dropped
        Runtime::instance().extends.hook_mgr().await.message_dropped(Some(to), from, publish, reason).await;
    }
}

static TASK_EXEC_QUEUE: OnceCell<TaskExecQueue> = OnceCell::new();

#[inline]
fn init_task_exec_queue(workers: usize, queue_max: usize) {
    let (exec, task_runner) = Builder::default().workers(workers).queue_max(queue_max).build();

    tokio::spawn(async move {
        task_runner.await;
    });

    TASK_EXEC_QUEUE.set(exec).ok().expect("Failed to initialize task execution queue")
}

#[inline]
pub(crate) fn task_exec_queue() -> &'static TaskExecQueue {
    TASK_EXEC_QUEUE.get().expect("TaskExecQueue not initialized")
}
