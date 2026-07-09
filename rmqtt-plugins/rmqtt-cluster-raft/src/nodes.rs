//! Runtime cluster node registry for business gRPC peers.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use dashmap::DashMap;
use serde_json::json;

use rmqtt::{
    context::ServerContext,
    grpc::{GrpcClient, GrpcClients},
    types::{Addr, NodeId, NodeName},
    utils::NodeAddr,
};

use super::config::PluginConfig;
use super::message::ClusterNode;
use super::HashMap;

#[derive(Clone)]
pub(crate) struct ClusterNodeManager {
    scx: ServerContext,
    nodes: Arc<DashMap<NodeId, ClusterNode, ahash::RandomState>>,
    clients: Arc<DashMap<NodeId, (Addr, GrpcClient), ahash::RandomState>>,
    options: Arc<ClientOptions>,
}

#[derive(Clone)]
struct ClientOptions {
    batch_size: usize,
    concurrency_limit: usize,
    timeout: Duration,
}

impl ClientOptions {
    fn from_config(cfg: &PluginConfig) -> Self {
        Self {
            batch_size: cfg.node_grpc_batch_size,
            concurrency_limit: cfg.node_grpc_client_concurrency_limit,
            timeout: cfg.node_grpc_client_timeout,
        }
    }
}

impl ClusterNodeManager {
    pub(crate) fn new(scx: ServerContext, cfg: &PluginConfig) -> Self {
        Self {
            scx,
            nodes: Arc::new(DashMap::default()),
            clients: Arc::new(DashMap::default()),
            options: Arc::new(ClientOptions::from_config(cfg)),
        }
    }

    pub(crate) async fn seed_from_config(
        &self,
        node_grpc_addrs: &[NodeAddr],
        raft_peer_addrs: &[NodeAddr],
    ) -> Result<()> {
        for grpc_addr in node_grpc_addrs {
            let raft_addr = raft_peer_addrs
                .iter()
                .find(|addr| addr.id == grpc_addr.id)
                .map(|addr| addr.addr.clone())
                .unwrap_or_default();
            self.upsert(ClusterNode { id: grpc_addr.id, grpc_addr: grpc_addr.addr.clone(), raft_addr })
                .await?;
        }
        Ok(())
    }

    pub(crate) fn local_node_from_config(
        &self,
        node_grpc_addrs: &[NodeAddr],
        raft_peer_addrs: &[NodeAddr],
    ) -> Result<ClusterNode> {
        let id = self.scx.node.id();
        let grpc_addr = node_grpc_addrs
            .iter()
            .find(|addr| addr.id == id)
            .map(|addr| addr.addr.clone())
            .ok_or_else(|| anyhow::anyhow!("node_grpc_addrs does not contain local node id {}", id))?;
        let raft_addr = raft_peer_addrs
            .iter()
            .find(|addr| addr.id == id)
            .map(|addr| addr.addr.clone())
            .ok_or_else(|| anyhow::anyhow!("raft_peer_addrs does not contain local node id {}", id))?;
        Ok(ClusterNode { id, grpc_addr, raft_addr })
    }

    pub(crate) async fn upsert(&self, node: ClusterNode) -> Result<()> {
        let id = node.id;
        let grpc_addr = node.grpc_addr.clone();
        self.nodes.insert(id, node);

        if id == self.scx.node.id() {
            self.clients.remove(&id);
            return Ok(());
        }

        let client_current = self.clients.get(&id).map(|entry| entry.value().0.clone());
        if client_current.as_ref() == Some(&grpc_addr) {
            return Ok(());
        }

        let client = self
            .scx
            .node
            .new_grpc_client(
                &grpc_addr,
                self.options.timeout,
                self.options.concurrency_limit,
                self.options.batch_size,
                &self.scx.circuit_breaker_config,
            )
            .await?;
        self.clients.insert(id, (grpc_addr, client));
        Ok(())
    }

    pub(crate) fn remove(&self, node_id: NodeId) {
        self.nodes.remove(&node_id);
        self.clients.remove(&node_id);
    }

    pub(crate) fn grpc_client(&self, node_id: NodeId) -> Option<GrpcClient> {
        self.clients.get(&node_id).map(|entry| entry.value().1.clone())
    }

    pub(crate) fn grpc_clients(&self) -> GrpcClients {
        let mut clients = HashMap::default();
        for entry in self.clients.iter() {
            clients.insert(*entry.key(), entry.value().clone());
        }
        Arc::new(clients)
    }

    pub(crate) fn node_name(&self, id: NodeId) -> NodeName {
        self.nodes.get(&id).map(|node| format!("{}@{}", node.id, node.grpc_addr)).unwrap_or_default()
    }

    pub(crate) fn snapshot_nodes(&self) -> Vec<ClusterNode> {
        self.nodes.iter().map(|entry| entry.value().clone()).collect()
    }

    pub(crate) async fn restore_nodes(&self, nodes: Vec<ClusterNode>) -> Result<()> {
        self.nodes.clear();
        self.clients.clear();
        for node in nodes {
            self.upsert(node).await?;
        }
        Ok(())
    }

    pub(crate) fn to_json(&self) -> serde_json::Value {
        let nodes = self
            .nodes
            .iter()
            .map(|entry| {
                let node = entry.value();
                json!({
                    "id": node.id,
                    "grpc_addr": node.grpc_addr,
                    "raft_addr": node.raft_addr,
                    "has_grpc_client": self.clients.contains_key(&node.id),
                })
            })
            .collect::<Vec<_>>();
        json!(nodes)
    }
}
