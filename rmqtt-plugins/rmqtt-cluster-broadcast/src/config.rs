//! Configuration for the cluster broadcast plugin.
//!
//! Defines [`PluginConfig`] with gRPC communication settings including
//! node addresses, timeouts, batch sizes, and concurrency limits.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use rmqtt::grpc::MessageType;
use rmqtt::utils::{deserialize_duration, NodeAddr};
use rmqtt::Result;

/// Configuration for the cluster broadcast plugin.
///
/// Specifies the gRPC message type, peer node addresses, client concurrency
/// limits, timeouts, and batch sizes for inter-node communication.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default = "PluginConfig::message_type_default")]
    pub message_type: MessageType,

    pub node_grpc_addrs: Vec<NodeAddr>,

    #[serde(default = "PluginConfig::grpc_client_concurrency_limit_default")]
    pub node_grpc_client_concurrency_limit: usize,

    #[serde(
        default = "PluginConfig::grpc_client_timeout_default",
        deserialize_with = "deserialize_duration"
    )]
    pub node_grpc_client_timeout: Duration,

    //#Maximum number of messages sent in batch
    #[serde(default = "PluginConfig::grpc_batch_size_default")]
    pub node_grpc_batch_size: usize,

    //#Task execution queue workers
    #[serde(default = "PluginConfig::task_exec_queue_workers_default")]
    pub task_exec_queue_workers: usize,

    //#Task execution queue max capacity
    #[serde(default = "PluginConfig::task_exec_queue_max_default")]
    pub task_exec_queue_max: usize,

    // ─── gRPC circuit breaker ──────────────────────────────────────────────
    /// Enable circuit breaker for gRPC client. When enabled and the
    /// circuit is OPEN, all gRPC requests fast-fail without sending.
    #[serde(default = "PluginConfig::node_grpc_circuit_breaker_enabled_default")]
    pub node_grpc_circuit_breaker_enabled: bool,

    /// Consecutive gRPC failures before tripping the circuit to OPEN.
    #[serde(default = "PluginConfig::node_grpc_circuit_failure_threshold_default")]
    pub node_grpc_circuit_failure_threshold: usize,

    /// Duration in OPEN state before transitioning to HALF_OPEN (probe).
    /// Example: "15s", "1m".
    #[serde(
        default = "PluginConfig::node_grpc_circuit_reset_timeout_default",
        deserialize_with = "deserialize_duration"
    )]
    pub node_grpc_circuit_reset_timeout: Duration,

    /// Consecutive probe successes in HALF_OPEN before closing the circuit.
    #[serde(default = "PluginConfig::node_grpc_circuit_half_open_success_threshold_default")]
    pub node_grpc_circuit_half_open_success_threshold: usize,
}

impl PluginConfig {
    fn message_type_default() -> MessageType {
        98
    }

    fn grpc_client_concurrency_limit_default() -> usize {
        128
    }
    fn grpc_client_timeout_default() -> Duration {
        Duration::from_secs(60)
    }

    fn grpc_batch_size_default() -> usize {
        128
    }

    fn task_exec_queue_workers_default() -> usize {
        500
    }

    fn task_exec_queue_max_default() -> usize {
        100_000
    }

    fn node_grpc_circuit_breaker_enabled_default() -> bool {
        true
    }

    fn node_grpc_circuit_failure_threshold_default() -> usize {
        10
    }

    fn node_grpc_circuit_reset_timeout_default() -> Duration {
        Duration::from_secs(15)
    }

    fn node_grpc_circuit_half_open_success_threshold_default() -> usize {
        3
    }

    /// Serializes the configuration to a JSON value.
    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
}
