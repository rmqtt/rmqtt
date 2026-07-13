//! Raft-based cluster plugin for RMQTT.
//!
//! Implements distributed consensus and state machine replication
//! using the Raft algorithm for cluster-wide coordination.
//!
//! # Architecture
//!
//! - **`ClusterPlugin`**: Main plugin implementing the [`Plugin`] trait.
//! - **`ClusterRouter`**: Distributed topic routing via Raft consensus.
//! - **`ClusterShared`**: Shared cluster state (sessions, subscriptions).
//! - **`HookHandler`**: MQTT event hooks for cluster-aware processing.
//!
//! # Startup Flow
//!
//! 1. [`Plugin::init`] — Starts the Raft node (leader or follower),
//!    registers hooks (`ClientDisconnected`, `SessionTerminated`,
//!    `GrpcMessageReceived`), and begins health monitoring.
//! 2. [`Plugin::start`] — Replaces the local router/shared state with
//!    clustered versions, enables hooks, and pings the Raft cluster
//!    to verify readiness.
//! 3. [`Plugin::stop`] — Always returns `false`; a running cluster
//!    cannot be cleanly stopped once started.
//!
//! # Raft Lifecycle
//!
//! - Runs in a dedicated OS thread with its own Tokio runtime.
//! - Leader election and log replication are handled by `rmqtt-raft`.
//! - Inter-node communication uses gRPC for message forwarding.
//!
#![deny(unsafe_code)]
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use backoff::{ExponentialBackoff, ExponentialBackoffBuilder};
use config::PluginConfig;
use handler::HookHandler;
use rust_box::task_exec_queue::TaskExecQueue;
use serde_json::{self, json};
use tokio::time::sleep;

use nodes::ClusterNodeManager;
use rmqtt::{
    context::ServerContext,
    hook::{Register, Type},
    macros::Plugin,
    plugin::{PackageInfo, Plugin},
    register,
    types::{From, NodeId, Publish, Reason, To},
    Result,
};
use rmqtt_raft::{Mailbox, Raft};
use router::ClusterRouter;
use shared::ClusterShared;

mod config;
mod handler;
mod message;
mod nodes;
mod raft_admin;
mod router;
mod shared;

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

register!(ClusterPlugin::new);

/// Raft-based cluster consensus plugin.
///
/// Manages cluster membership, distributed routing, and state
/// replication across RMQTT broker nodes.
#[derive(Plugin)]
struct ClusterPlugin {
    scx: ServerContext,
    register: Box<dyn Register>,
    cfg: Arc<PluginConfig>,
    nodes: ClusterNodeManager,
    shared: ClusterShared,
    router: ClusterRouter,
    raft_mailbox: Option<Mailbox>,
    draining: Arc<AtomicBool>,
    exec: TaskExecQueue,
    backoff_strategy: ExponentialBackoff,
}

impl ClusterPlugin {
    #[inline]
    async fn new<S: Into<String>>(scx: ServerContext, name: S) -> Result<Self> {
        let name = name.into();
        let env_list_keys = ["node_grpc_addrs", "raft_peer_addrs"];
        let mut cfg = scx.plugins.read_config_with::<PluginConfig>(&name, &env_list_keys)?;
        cfg.merge(&scx.args);
        log::info!("{name} ClusterPlugin cfg: {cfg:?}");

        let exec = scx.get_exec(("RAFT_EXEC", cfg.task_exec_queue_workers, cfg.task_exec_queue_max));

        let register = scx.extends.hook_mgr().register();
        let nodes = ClusterNodeManager::new(scx.clone(), &cfg);
        nodes.seed_from_config(&cfg.node_grpc_addrs, &cfg.raft_peer_addrs).await?;
        let backoff_strategy = ExponentialBackoffBuilder::new()
            .with_max_elapsed_time(Some(Duration::from_secs(60)))
            .with_multiplier(2.5)
            .build();
        let router = ClusterRouter::new(
            scx.clone(),
            backoff_strategy.clone(),
            nodes.clone(),
            cfg.try_lock_timeout,
            cfg.compression,
        );
        let shared = ClusterShared::new(
            scx.clone(),
            exec.clone(),
            router.clone(),
            nodes.clone(),
            cfg.message_type,
            cfg.task_exec_queue_max,
            cfg.task_exec_queue_workers,
            cfg.node_grpc_client_timeout,
        );
        let raft_mailbox = None;
        let draining = Arc::new(AtomicBool::new(false));
        let cfg = Arc::new(cfg);
        Ok(Self { scx, register, cfg, nodes, shared, router, raft_mailbox, draining, exec, backoff_strategy })
    }

    //raft init ...
    /// Initialize the Raft node and attempt to join or lead the cluster.
    ///
    /// # Flow
    ///
    /// 1. Resolve this node's Raft listen address.
    /// 2. Optionally verify it resolves.
    /// 3. Create the Raft instance on a dedicated thread with its own
    ///    Tokio runtime.
    /// 4. Based on configuration:
    ///    - If a leader is specified, verify the actual leader matches
    ///      and join as a follower.
    ///    - Otherwise, search for an existing leader. If none found,
    ///      become the leader.
    async fn start_raft(
        scx: &ServerContext,
        cfg: Arc<PluginConfig>,
        router: ClusterRouter,
    ) -> Result<Mailbox> {
        // let logger = Runtime::instance().logger.clone();
        let raft_peer_addrs = cfg.raft_peer_addrs.clone();

        let id = scx.node.id();

        let raft_node_addr = raft_peer_addrs
            .iter()
            .find(|peer| peer.id == id)
            .map(|peer| peer.addr.to_string())
            .ok_or_else(|| anyhow!("raft listening address does not exist"))?;

        let raft_laddr =
            if let Some(laddr) = cfg.laddr.as_ref() { laddr.to_string() } else { raft_node_addr.clone() };

        log::info!("raft_laddr: {raft_laddr:?}, raft_node_addr: {raft_node_addr:?}");

        //verify the listening address
        if cfg.verify_addr {
            parse_addr(&raft_laddr).await?;
        }
        use slog::Drain;
        let logger = slog::Logger::root(slog_stdlog::StdLog.fuse(), slog::o!());

        let raft =
            Raft::new(raft_laddr, router, logger, cfg.raft.to_raft_config()).map_err(|e| anyhow!(e))?;
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
                    Some(actual_leader_info.ok_or_else(|| anyhow!("Leader does not exist"))?)
                }
            }
            None => {
                log::info!("Search for the existing leader ... ");
                let leader_info = raft.find_leader_info(peer_addrs).await.map_err(|e| anyhow!(e))?;
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
                        tokio::spawn(raft.lead(id, raft_node_addr)).await
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
        self.register
            .add(
                typ,
                Box::new(HookHandler::new(
                    self.scx.clone(),
                    self.cfg.clone(),
                    self.backoff_strategy.clone(),
                    self.shared.clone(),
                    self.raft_mailbox(),
                )),
            )
            .await;
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

        let raft_mailbox = Self::start_raft(&self.scx, self.cfg.clone(), self.router.clone()).await?;

        let mut raft_status = None;
        for i in 0..60 {
            match raft_mailbox.status().await {
                Ok(status) => {
                    if status.is_started() {
                        raft_status = Some(status);
                        break;
                    }
                    log::info!("{} Initializing cluster, raft status({}): {:?}", self.name(), i, status);
                }
                Err(e) => {
                    log::info!("{} init failed, {:?}", self.name(), e);
                }
            }
            sleep(Duration::from_millis(500)).await;
        }
        if raft_status.is_none() {
            return Err(anyhow!("init failed"));
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
        *self.scx.extends.router_mut().await = Box::new(self.router.clone());
        *self.scx.extends.shared_mut().await = Box::new(self.shared.clone());
        self.register.start().await;

        // Async retain sync from peers (fire-and-forget)
        let shared = self.shared.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            shared.sync_retains_from_peers().await;
        });

        let status = raft_mailbox.status().await.map_err(anyhow::Error::new)?;
        log::info!("raft status: {status:?}");
        if !status.is_started() {
            return Err(anyhow!("Raft cluster status is abnormal"));
        }

        let ping = message::Message::Ping.encode()?;
        for _ in 0..100 {
            match raft_mailbox.send_proposal(ping.clone()).await {
                Ok(reply) => match message::MessageReply::decode(&reply)? {
                    message::MessageReply::Ping => {
                        log::info!("ping ok");
                        let local_node = self
                            .nodes
                            .local_node_from_config(&self.cfg.node_grpc_addrs, &self.cfg.raft_peer_addrs)?;
                        let node_up = message::Message::NodeUp { node: local_node }.encode()?;
                        raft_mailbox.send_proposal(node_up).await.map_err(anyhow::Error::new)?;
                        return Ok(());
                    }
                    message::MessageReply::Error(e) => {
                        log::warn!("ping failed, {e}");
                    }
                    _ => {
                        log::error!("unreachable!()");
                    }
                },
                Err(e) => {
                    log::warn!("ping failed, {e}");
                }
            }
            sleep(Duration::from_millis(500)).await;
        }

        Err(anyhow!("Raft cluster status is unavailable"))
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
        let grpc_clients = self.nodes.grpc_clients();
        for (node_id, (_, c)) in grpc_clients.iter() {
            let stats = json!({
                "transfer_queue_len": c.transfer_queue_len(),
                "active_tasks_count": c.active_tasks().count(),
                "active_tasks_max": c.active_tasks().max(),
                "circuit_breaker": c.circuit_breaker_json().await,
            });
            nodes.insert(*node_id, stats);
        }

        json!({
            "grpc_clients": nodes,
            "cluster_nodes": self.nodes.to_json(),
            "raft_status": raft_status,
            "raft_pears": pears,
            "client_states": self.router.states_count(),
            "task_exec_queue": {
                "waiting_count": self.exec.waiting_count(),
                "active_count": self.exec.active_count(),
                "completed_count": self.exec.completed_count().await,
            }
        })
    }

    #[inline]
    async fn send(&self, msg: serde_json::Value) -> Result<serde_json::Value> {
        let action = msg.get("action").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("missing action"))?;
        let raft_mailbox = self.raft_mailbox();
        match action {
            "node_up" => {
                let node: message::ClusterNode =
                    serde_json::from_value(msg.get("node").cloned().unwrap_or_else(|| {
                        json!({
                            "id": msg.get("id").cloned().unwrap_or_default(),
                            "grpc_addr": msg.get("grpc_addr").cloned().unwrap_or_default(),
                            "raft_addr": msg.get("raft_addr").cloned().unwrap_or_default(),
                        })
                    }))?;
                let proposal = message::Message::NodeUp { node }.encode()?;
                raft_mailbox.send_proposal(proposal).await.map_err(anyhow::Error::new)?;
                Ok(json!({"ok": true}))
            }
            "node_down" => {
                let id = msg.get("id").and_then(|v| v.as_u64()).ok_or_else(|| anyhow!("missing id"))?;
                let proposal = message::Message::NodeDown { id }.encode()?;
                raft_mailbox.send_proposal(proposal).await.map_err(anyhow::Error::new)?;
                Ok(json!({"ok": true, "id": id, "raft_membership_changed": false}))
            }
            "node_remove" => {
                let id = msg.get("id").and_then(|v| v.as_u64()).unwrap_or_else(|| self.scx.node.id());
                if id != self.scx.node.id() {
                    return Err(anyhow!(
                        "node_remove must be sent to target node {id}; \
                         use action=force_node_remove for business-only cleanup of an unreachable node"
                    ));
                }
                self.leave_cluster(&raft_mailbox).await
            }
            "force_node_remove" => {
                let id = msg.get("id").and_then(|v| v.as_u64()).ok_or_else(|| anyhow!("missing id"))?;
                if id == self.scx.node.id() {
                    return Err(anyhow!("force_node_remove cannot remove the local node; use action=leave"));
                }
                let proposal = message::Message::NodeRemove { id }.encode()?;
                raft_mailbox.send_proposal(proposal).await.map_err(anyhow::Error::new)?;
                Ok(json!({"ok": true, "id": id, "raft_membership_changed": false}))
            }
            "drain" | "node_drain" => self.drain_node(&raft_mailbox).await,
            "undrain" | "resume" | "node_up_local" => self.undrain_node(&raft_mailbox).await,
            "drain_status" => Ok(self.drain_status(&raft_mailbox).await),
            "leave" | "node_leave" => self.leave_cluster(&raft_mailbox).await,
            "nodes" => Ok(self.nodes.to_json()),
            other => Err(anyhow!("unknown action: {other}")),
        }
    }
}

impl ClusterPlugin {
    async fn drain_node(&self, raft_mailbox: &Mailbox) -> Result<serde_json::Value> {
        let id = self.scx.node.id();
        let proposal = message::Message::NodeDown { id }.encode()?;
        raft_mailbox.send_proposal(proposal).await.map_err(anyhow::Error::new)?;
        self.draining.store(true, Ordering::SeqCst);
        Ok(json!({
            "ok": true,
            "id": id,
            "draining": true,
            "raft_membership_changed": false,
            "note": "node is removed from the business gRPC peer table; operator should remove external traffic and wait for drain_status.safe_to_leave"
        }))
    }

    async fn undrain_node(&self, raft_mailbox: &Mailbox) -> Result<serde_json::Value> {
        let node = self.nodes.local_node_from_config(&self.cfg.node_grpc_addrs, &self.cfg.raft_peer_addrs)?;
        let id = node.id;
        let proposal = message::Message::NodeUp { node }.encode()?;
        raft_mailbox.send_proposal(proposal).await.map_err(anyhow::Error::new)?;
        self.draining.store(false, Ordering::SeqCst);
        Ok(json!({
            "ok": true,
            "id": id,
            "draining": false,
            "raft_membership_changed": false,
        }))
    }

    async fn drain_status(&self, raft_mailbox: &Mailbox) -> serde_json::Value {
        let stats = &self.scx.stats;
        let connections = stats.connections.count();
        let sessions = stats.sessions.count();
        let handshakings_active = stats.handshakings_active.count();
        let message_queues = stats.message_queues.count();
        let out_inflights = stats.out_inflights.count();
        let in_inflights = stats.in_inflights.count();
        let forwards = stats.forwards.count();
        let message_storages = stats.message_storages.count();
        let delayed_publishs = stats.delayed_publishs.count();
        let safe_to_leave = connections == 0
            && sessions == 0
            && handshakings_active == 0
            && message_queues == 0
            && out_inflights == 0
            && in_inflights == 0
            && forwards == 0;
        let raft_status = raft_mailbox.status().await.ok();

        json!({
            "id": self.scx.node.id(),
            "draining": self.draining.load(Ordering::SeqCst),
            "safe_to_leave": safe_to_leave,
            "raft_membership_present": raft_status
                .as_ref()
                .map(|status| status.peers.contains_key(&self.scx.node.id()))
                .unwrap_or(false),
            "raft_leader_id": raft_status.as_ref().map(|status| status.leader_id),
            "counts": {
                "connections": connections,
                "sessions": sessions,
                "handshakings_active": handshakings_active,
                "message_queues": message_queues,
                "out_inflights": out_inflights,
                "in_inflights": in_inflights,
                "forwards": forwards,
                "message_storages": message_storages,
                "delayed_publishs": delayed_publishs,
            },
            "operator_next_step": if safe_to_leave { "leave" } else { "wait" },
        })
    }

    async fn leave_cluster(&self, raft_mailbox: &Mailbox) -> Result<serde_json::Value> {
        let id = self.scx.node.id();
        let local_node =
            self.nodes.local_node_from_config(&self.cfg.node_grpc_addrs, &self.cfg.raft_peer_addrs)?;

        let proposal = message::Message::NodeRemove { id }.encode()?;
        raft_mailbox.send_proposal(proposal).await.map_err(anyhow::Error::new)?;

        if let Err(e) = self.remove_self_from_raft(raft_mailbox).await {
            let restore = message::Message::NodeUp { node: local_node }.encode()?;
            if let Err(restore_err) = raft_mailbox.send_proposal(restore).await {
                log::error!(
                    "raft leave failed and failed to restore business node metadata, id: {id}, \
                     leave_error: {e}, restore_error: {restore_err}"
                );
            }
            return Err(e);
        }

        Ok(json!({"ok": true, "id": id, "raft_membership_changed": true}))
    }

    async fn remove_self_from_raft(&self, raft_mailbox: &Mailbox) -> Result<()> {
        let status = raft_mailbox.status().await.map_err(anyhow::Error::new)?;
        if status.is_leader() {
            return raft_mailbox.leave().await.map_err(anyhow::Error::new);
        }

        let leader_id = status.leader_id;
        let leader_addr = status
            .peers
            .get(&leader_id)
            .and_then(|peer| peer.as_ref().map(|peer| peer.addr.to_string()))
            .or_else(|| {
                self.cfg
                    .raft_peer_addrs
                    .iter()
                    .find(|peer| peer.id == leader_id)
                    .map(|peer| peer.addr.to_string())
            })
            .ok_or_else(|| anyhow!("raft leader address not found, leader_id: {leader_id}"))?;

        log::info!(
            "request raft leader to remove local node, node_id: {}, leader_id: {}, leader_addr: {}",
            self.scx.node.id(),
            leader_id,
            leader_addr
        );
        raft_admin::remove_node(
            &leader_addr,
            self.scx.node.id(),
            self.cfg.raft.grpc_timeout.unwrap_or(Duration::from_secs(6)),
            self.cfg.raft.grpc_concurrency_limit.unwrap_or(200),
            self.cfg.raft.grpc_message_size.unwrap_or(50 * 1024 * 1024),
        )
        .await
    }
}

/// Parse a string address into a [`SocketAddr`] with retry logic.
///
/// Retries up to 10 times with randomized backoff (500-800ms)
/// to handle DNS resolution delays during startup.
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
                log::warn!("Round: {i}, {e}");
            }
        }
        tokio::time::sleep(Duration::from_millis((rand::random::<u64>() % 300) + 500)).await;
    }
    Err(anyhow!(format!("Parsing address{:?} error", addr)))
}

/// Poll peer nodes to find the current Raft leader.
///
/// Attempts up to `rounds` iterations, sleeping 500ms between attempts.
/// Returns `None` if no leader is found after all rounds.
async fn find_actual_leader(
    raft: &Raft<ClusterRouter>,
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
pub(crate) async fn hook_message_dropped(scx: &ServerContext, droppeds: Vec<(To, From, Publish, Reason)>) {
    for (to, from, publish, reason) in droppeds {
        //hook, message_dropped
        scx.extends.hook_mgr().message_dropped(Some(to), from, publish, reason).await;
    }
}
