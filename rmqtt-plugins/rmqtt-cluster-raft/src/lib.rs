#![allow(clippy::result_large_err)]
#![deny(unsafe_code)]
#[macro_use]
extern crate serde;

#[macro_use]
extern crate rmqtt_macros;

use rmqtt_raft::{Mailbox, Raft};
use std::convert::From as _f;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use config::PluginConfig;
use handler::HookHandler;

use rmqtt::anyhow::anyhow;
use rmqtt::{
    ahash, anyhow,
    async_trait::async_trait,
    log, rand,
    serde_json::{self, json},
    tokio, NodeId,
};
use rmqtt::{
    broker::{
        error::MqttError,
        hook::{Register, Type},
        types::{From, Publish, Reason, To},
    },
    grpc::{client::NodeGrpcClient, GrpcClients},
    plugin::{PackageInfo, Plugin},
    register,
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
mod router;
mod shared;

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

register!(ClusterPlugin::new);

#[derive(Plugin)]
struct ClusterPlugin {
    runtime: &'static Runtime,
    register: Box<dyn Register>,
    cfg: Arc<PluginConfig>,
    grpc_clients: GrpcClients,
    shared: &'static ClusterShared,

    router: &'static ClusterRouter,
    raft_mailbox: Option<Mailbox>,
}

impl ClusterPlugin {
    #[inline]
    async fn new<S: Into<String>>(runtime: &'static Runtime, name: S) -> Result<Self> {
        let name = name.into();
        let env_list_keys = ["node_grpc_addrs", "raft_peer_addrs"];
        let mut cfg = runtime.settings.plugins.load_config_with::<PluginConfig>(&name, &env_list_keys)?;
        cfg.merge(&runtime.settings.opts);
        log::info!("{name} ClusterPlugin cfg: {cfg:?}");

        init_task_exec_queue(cfg.task_exec_queue_workers, cfg.task_exec_queue_max);

        let register = runtime.extends.hook_mgr().await.register();
        let mut grpc_clients = HashMap::default();
        let mut node_names = HashMap::default();

        let node_grpc_addrs = cfg.node_grpc_addrs.clone();
        log::info!("node_grpc_addrs: {node_grpc_addrs:?}");
        for node_addr in &node_grpc_addrs {
            if node_addr.id != runtime.node.id() {
                grpc_clients.insert(
                    node_addr.id,
                    (node_addr.addr.clone(), runtime.node.new_grpc_client(&node_addr.addr).await?),
                );
            }
            node_names.insert(node_addr.id, format!("{}@{}", node_addr.id, node_addr.addr));
        }
        let grpc_clients = Arc::new(grpc_clients);
        let router = ClusterRouter::get_or_init(cfg.try_lock_timeout, cfg.compression);
        let shared = ClusterShared::get_or_init(
            router,
            grpc_clients.clone(),
            node_names,
            cfg.message_type,
            cfg.task_exec_queue_max,
            cfg.task_exec_queue_workers,
        );
        let raft_mailbox = None;
        let cfg = Arc::new(cfg);
        Ok(Self { runtime, register, cfg, grpc_clients, shared, router, raft_mailbox })
    }

    //raft init ...
    async fn start_raft(cfg: Arc<PluginConfig>, router: &'static ClusterRouter) -> Result<Mailbox> {
        let logger = Runtime::instance().logger.clone();
        let raft_peer_addrs = cfg.raft_peer_addrs.clone();

        let id = Runtime::instance().node.id();

        let raft_node_addr = raft_peer_addrs
            .iter()
            .find(|peer| peer.id == id)
            .map(|peer| peer.addr.to_string())
            .ok_or_else(|| MqttError::from("raft listening address does not exist"))?;

        let raft_laddr =
            if let Some(laddr) = cfg.laddr.as_ref() { laddr.to_string() } else { raft_node_addr.clone() };

        log::info!("raft_laddr: {raft_laddr:?}, raft_node_addr: {raft_node_addr:?}");

        //verify the listening address
        if cfg.verify_addr {
            parse_addr(&raft_laddr).await?;
        }

        let raft = Raft::new(raft_laddr, router, logger, cfg.raft.to_raft_config())
            .map_err(|e| MqttError::StdError(Box::new(e)))?;
        let mailbox = raft.mailbox();

        let mut peer_addrs = Vec::new();
        for peer in raft_peer_addrs.iter() {
            if peer.id != id {
                if cfg.verify_addr {
                    peer_addrs.push(parse_addr(&peer.addr).await?.to_string());
                } else {
                    peer_addrs.push(peer.addr.to_string());
                }
            }
        }
        log::info!("peer_addrs: {peer_addrs:?}");

        let leader_info = match cfg.leader()? {
            Some(leader_info) => {
                log::info!("Specify a leader: {leader_info:?}");
                if id == leader_info.id {
                    //First, check if the Leader exists.
                    let actual_leader_info = find_actual_leader(&raft, peer_addrs, 3).await?;
                    if actual_leader_info.is_some() {
                        log::info!("Leader already exists, {actual_leader_info:?}");
                    }
                    actual_leader_info
                } else {
                    //The other nodes are leader.
                    let actual_leader_info = find_actual_leader(&raft, peer_addrs, 60).await?;
                    let (actual_leader_id, actual_leader_addr) =
                        actual_leader_info.ok_or_else(|| MqttError::from("Leader does not exist"))?;
                    if actual_leader_id != leader_info.id {
                        return Err(MqttError::from(format!(
                            "Not the expected Leader, the expected one is {leader_info:?}"
                        )));
                    }
                    Some((actual_leader_id, actual_leader_addr))
                }
            }
            None => {
                log::info!("Search for the existing leader ... ");
                let leader_info =
                    raft.find_leader_info(peer_addrs).await.map_err(|e| MqttError::StdError(Box::new(e)))?;
                log::info!("The information about the located leader: {leader_info:?}");
                leader_info
            }
        };

        //let (status_tx, status_rx) = futures::channel::oneshot::channel::<Result<Status>>();
        let _child = std::thread::Builder::new().name("cluster-raft".to_string()).spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(cfg.worker_threads)
                .thread_name("cluster-raft-worker")
                .thread_stack_size(4 * 1024 * 1024)
                .build()
                .expect("tokio runtime build failed");

            let runner = async move {
                log::info!("leader_info: {leader_info:?}");
                let raft_handle = match leader_info {
                    Some((leader_id, leader_addr)) => {
                        log::info!(
                            "running in follower mode, leader_id: {leader_id}, leader_addr: {leader_addr}"
                        );
                        tokio::spawn(raft.join(id, raft_node_addr, Some(leader_id), leader_addr)).await
                    }
                    None => {
                        log::info!("running in leader mode");
                        tokio::spawn(raft.lead(id)).await
                    }
                };

                if let Err(_) | Ok(Err(_)) = raft_handle {
                    log::error!("Raft service startup failed, {raft_handle:?}");
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    std::process::exit(-1);
                }
            };

            rt.block_on(runner);
        })?;
        Ok(mailbox)
    }

    #[inline]
    async fn hook_register(&self, typ: Type) {
        self.register.add(typ, Box::new(HookHandler::new(self.shared, self.raft_mailbox()))).await;
    }

    #[inline]
    fn raft_mailbox(&self) -> Mailbox {
        if let Some(raft_mailbox) = &self.raft_mailbox {
            raft_mailbox.clone()
        } else {
            unreachable!()
        }
    }

    fn start_check_health(&self) {
        let exit_on_node_unavailable = self.cfg.health.exit_on_node_unavailable;
        let exit_code = self.cfg.health.exit_code;
        let unavailable_check_interval = self.cfg.health.unavailable_check_interval;
        let max_continuous_unavailable_count = self.cfg.health.max_continuous_unavailable_count;

        let mut continuous_unavailable_count = 0;
        let raft_mailbox = self.raft_mailbox();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(unavailable_check_interval).await;
                match raft_mailbox.status().await {
                    Err(e) => {
                        log::error!("Error retrieving cluster status, {e}");
                    }
                    Ok(s) => {
                        if s.available() {
                            if continuous_unavailable_count > 0 {
                                continuous_unavailable_count = 0;
                            }
                        } else {
                            continuous_unavailable_count += 1;
                            log::error!(
                                "cluster node unavailable({continuous_unavailable_count}), node status: {s:?}"
                            );
                            if exit_on_node_unavailable
                                && continuous_unavailable_count >= max_continuous_unavailable_count
                            {
                                std::thread::sleep(Duration::from_secs(2));
                                std::process::exit(exit_code);
                            }
                        }
                    }
                }
            }
        });
    }
}

#[async_trait]
impl Plugin for ClusterPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());

        let raft_mailbox = Self::start_raft(self.cfg.clone(), self.router).await?;

        for i in 0..60 {
            match raft_mailbox.status().await {
                Ok(status) => {
                    if status.is_started() {
                        break;
                    }
                    log::info!("{} Initializing cluster, raft status({}): {:?}", self.name(), i, status);
                }
                Err(e) => {
                    log::info!("{} init error, {:?}", self.name(), e);
                }
            }
            sleep(Duration::from_millis(500)).await;
        }

        self.raft_mailbox.replace(raft_mailbox.clone());
        self.router.set_raft_mailbox(raft_mailbox).await;

        self.hook_register(Type::ClientDisconnected).await;
        self.hook_register(Type::SessionTerminated).await;
        self.hook_register(Type::GrpcMessageReceived).await;

        self.start_check_health();

        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.to_json()
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name());
        let raft_mailbox = self.raft_mailbox();
        *self.runtime.extends.router_mut().await = Box::new(self.router);
        *self.runtime.extends.shared_mut().await = Box::new(self.shared);
        self.register.start().await;
        let status = raft_mailbox.status().await.map_err(anyhow::Error::new)?;
        log::info!("raft status: {:?}", status);
        if !status.is_started() {
            return Err(MqttError::from("Raft cluster status is abnormal"));
        }

        let ping = message::Message::Ping.encode()?;
        for _ in 0..100 {
            match raft_mailbox.send_proposal(ping.clone()).await {
                Ok(reply) => match message::MessageReply::decode(&reply)? {
                    message::MessageReply::Ping => {
                        log::info!("ping ok");
                        return Ok(());
                    }
                    message::MessageReply::Error(e) => {
                        log::warn!("ping error, {e:?}");
                    }
                    _ => {
                        unreachable!()
                    }
                },
                Err(e) => {
                    log::warn!("ping error, {e:?}");
                }
            }
            sleep(Duration::from_millis(500)).await;
        }

        Err(MqttError::from("Raft cluster status is unavailable"))
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::warn!("{} stop, once the cluster is started, it cannot be stopped", self.name());
        Ok(false)
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
                "completed_count": exec.completed_count().await,
            }
        })
    }
}

async fn parse_addr(addr: &str) -> Result<SocketAddr> {
    for i in 0..10 {
        match addr.to_socket_addrs() {
            Ok(mut to_socket_addrs) => {
                if let Some(a) = to_socket_addrs.next() {
                    log::info!("Round: {i}, parse_addr({addr:?}), addr is {a:?}");
                    return Ok(a);
                } else {
                    log::warn!("Round: {i}, parse_addr({addr:?}), next is None");
                }
            }
            Err(e) => {
                log::warn!("Round: {i}, {e:?}");
            }
        }
        tokio::time::sleep(Duration::from_millis((rand::random::<u64>() % 300) + 500)).await;
    }
    Err(MqttError::from(format!("Parsing address{addr:?} error")))
}

async fn find_actual_leader(
    raft: &Raft<&ClusterRouter>,
    peer_addrs: Vec<String>,
    rounds: usize,
) -> Result<Option<(NodeId, String)>> {
    let mut actual_leader_info = None;
    for i in 0..rounds {
        actual_leader_info = raft.find_leader_info(peer_addrs.clone()).await.map_err(|e| anyhow!(e))?;
        if actual_leader_info.is_some() {
            break;
        }
        log::info!("Leader not found, rounds: {i}");
        sleep(Duration::from_millis(500)).await;
    }
    Ok(actual_leader_info)
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
