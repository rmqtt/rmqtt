//! MQTT Server Runtime Context Management
//!
//! Provides core infrastructure for building scalable MQTT brokers with:
//! - Resource-managed task execution pipelines
//! - Dynamic configuration of server capabilities
//! - System health monitoring and overload protection
//!
//! ## Architectural Components
//! 1. ​**​ServerContextBuilder​**​: Fluent interface for configuring:
//!    - Cluster node properties
//!    - Task execution parameters (workers/queue sizes)
//!    - Connection/session limits
//!    - Plugin system integration
//!
//! 2. ​**​Runtime Features​**​:
//!    - Asynchronous task execution with backpressure control
//!    - Busy state detection (handshake/connection thresholds)
//!    - Metrics collection and statistical tracking
//!    - Delayed message publishing with configurable policies
//!
//! 3. ​**​Resource Management​**​:
//!    - Atomic counters for connection/session tracking
//!    - DashMap-based listener configuration storage
//!    - Extension points for router/shared state customization
//!
//! ## Implementation Highlights
//! - Zero-cost abstractions through Arc-based sharing
//! - Tokio-powered async task scheduling
//! - Feature-gated components (metrics/plugins/stats)
//! - Thread-safe counters with lock-free operations
//!
//! Typical usage flow:
//! 1. Configure via ServerContextBuilder
//! 2. Initialize shared components (router/delayed sender)
//! 3. Monitor system state via is_busy() checks
//! 4. Access execution metrics through TaskExecStats
//!
//! ```rust
//! use std::sync::Arc;
//! use rmqtt::context::{ServerContext, TaskExecStats};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create server context with custom configuration
//! let ctx = ServerContext::new()
//!     .node_id(1)
//!     .task_exec_workers(2500)
//!     .mqtt_max_sessions(5000)
//!     .build()
//!     .await;
//!
//! Ok(())
//! }
//! ```

use std::fmt;
use std::iter::Sum;
use std::ops::Deref;
use std::sync::Arc;

use rust_box::task_exec_queue::{Builder, TaskExecQueue};
use serde::{Deserialize, Serialize};

use crate::args::CommandArgs;
#[cfg(feature = "delayed")]
use crate::delayed::DefaultDelayedSender;
use crate::executor::HandshakeExecutor;
use crate::extend;
#[cfg(feature = "metrics")]
use crate::metrics::Metrics;
use crate::node::Node;
#[cfg(feature = "plugin")]
use crate::plugin;
#[cfg(feature = "plugin")]
use crate::plugin::PluginManagerConfig;
use crate::router::DefaultRouter;
use crate::shared::DefaultShared;
#[cfg(feature = "stats")]
use crate::stats::Stats;
use crate::types::{DashMap, HashMap, ListenerConfig, ListenerId, NodeId};
use crate::utils::Counter;

/// Builder for constructing ServerContext with configurable parameters
/// # Example
/// ```
/// use rmqtt::context::ServerContextBuilder;
/// let builder = ServerContextBuilder::new()
///     .mqtt_delayed_publish_max(50_000)
///     .busy_handshaking_limit(100);
/// ```
pub struct ServerContextBuilder {
    /// Command line arguments configuration
    pub args: CommandArgs,
    /// Cluster node information
    pub node: Node,

    /// Number of worker threads for task execution
    pub task_exec_workers: usize,
    /// Maximum capacity for task execution queue
    pub task_exec_queue_max: usize,

    /// Enable/disable busy state checking
    pub busy_check_enable: bool,
    /// Maximum allowed concurrent handshakes before busy state
    pub busy_handshaking_limit: isize,

    /// Maximum delayed publish messages allowed
    pub mqtt_delayed_publish_max: usize,
    /// Maximum concurrent MQTT sessions (0 = unlimited)
    pub mqtt_max_sessions: isize,
    /// Immediate execution flag for delayed publishes
    pub mqtt_delayed_publish_immediate: bool,

    /// plugins config, path or configMap<plugin_name, toml_string>
    #[cfg(feature = "plugin")]
    pub plugins_config: PluginManagerConfig,
}

impl Default for ServerContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerContextBuilder {
    /// Creates a new ServerContextBuilder with default values
    pub fn new() -> ServerContextBuilder {
        Self {
            args: CommandArgs::default(),
            node: Node::default(),
            task_exec_workers: 2000,
            task_exec_queue_max: 300_000,
            busy_check_enable: true,
            busy_handshaking_limit: 0,
            mqtt_delayed_publish_max: 100_000,
            mqtt_max_sessions: 0,
            mqtt_delayed_publish_immediate: true,
            #[cfg(feature = "plugin")]
            plugins_config: PluginManagerConfig::default().path("rmqtt-plugins/".into()),
        }
    }

    /// Sets command line arguments configuration
    pub fn args(mut self, args: CommandArgs) -> Self {
        self.args = args;
        self
    }

    /// Configures cluster node identifier
    pub fn node_id(mut self, id: NodeId) -> Self {
        self.node.id = id;
        self
    }

    /// Sets complete Node configuration
    pub fn node(mut self, node: Node) -> Self {
        self.node = node;
        self
    }

    /// Configures task executor worker thread count
    pub fn task_exec_workers(mut self, task_exec_workers: usize) -> Self {
        self.task_exec_workers = task_exec_workers;
        self
    }

    /// Sets maximum capacity for task execution queue
    pub fn task_exec_queue_max(mut self, task_exec_queue_max: usize) -> Self {
        self.task_exec_queue_max = task_exec_queue_max;
        self
    }

    /// Enables/disables busy state checking
    pub fn busy_check_enable(mut self, busy_check_enable: bool) -> Self {
        self.busy_check_enable = busy_check_enable;
        self
    }

    /// Sets maximum concurrent handshakes threshold
    pub fn busy_handshaking_limit(mut self, busy_handshaking_limit: isize) -> Self {
        self.busy_handshaking_limit = busy_handshaking_limit;
        self
    }

    /// Configures maximum delayed publish messages
    pub fn mqtt_delayed_publish_max(mut self, mqtt_delayed_publish_max: usize) -> Self {
        self.mqtt_delayed_publish_max = mqtt_delayed_publish_max;
        self
    }

    /// Sets maximum allowed MQTT sessions
    pub fn mqtt_max_sessions(mut self, mqtt_max_sessions: isize) -> Self {
        self.mqtt_max_sessions = mqtt_max_sessions;
        self
    }

    /// Configures immediate execution for delayed publishes
    pub fn mqtt_delayed_publish_immediate(mut self, mqtt_delayed_publish_immediate: bool) -> Self {
        self.mqtt_delayed_publish_immediate = mqtt_delayed_publish_immediate;
        self
    }

    /// Sets directory path for plugin loading
    #[cfg(feature = "plugin")]
    pub fn plugins_config_dir<N: Into<String>>(mut self, plugins_dir: N) -> Self {
        self.plugins_config = self.plugins_config.path(plugins_dir.into());
        self
    }

    #[cfg(feature = "plugin")]
    pub fn plugins_config_map(mut self, plugins_config_map: HashMap<String, String>) -> Self {
        self.plugins_config = self.plugins_config.map(plugins_config_map);
        self
    }

    #[cfg(feature = "plugin")]
    pub fn plugins_config_map_add<N: Into<String>, C: Into<String>>(mut self, name: N, cfg: C) -> Self {
        self.plugins_config = self.plugins_config.add(name.into(), cfg.into());
        self
    }

    #[cfg(feature = "plugin")]
    pub fn plugins_config(mut self, plugins_config: PluginManagerConfig) -> Self {
        if let Some(path) = plugins_config.path {
            self.plugins_config = self.plugins_config.path(path);
        }
        self.plugins_config = self.plugins_config.map(plugins_config.map);
        self
    }

    /// Constructs the ServerContext with configured parameters
    pub async fn build(self) -> ServerContext {
        ServerContext {
            inner: Arc::new(ServerContextInner {
                args: self.args,
                node: self.node,
                listen_cfgs: DashMap::default(),
                extends: extend::Manager::new(),
                #[cfg(feature = "plugin")]
                plugins: plugin::Manager::new(self.plugins_config),
                #[cfg(feature = "metrics")]
                metrics: Metrics::new(),
                #[cfg(feature = "stats")]
                stats: Stats::new(),
                handshake_exec: HandshakeExecutor::new(self.busy_handshaking_limit),
                execs: DashMap::default(),

                busy_check_enable: self.busy_check_enable,
                mqtt_delayed_publish_max: self.mqtt_delayed_publish_max,
                mqtt_max_sessions: self.mqtt_max_sessions,
                mqtt_delayed_publish_immediate: self.mqtt_delayed_publish_immediate,

                handshakings: Counter::new(),
                connections: Counter::new(),
                sessions: Counter::new(),

                task_exec_workers: self.task_exec_workers,
                task_exec_queue_max: self.task_exec_queue_max,
            }),
        }
        .config()
        .await
        .start_cpuload_monitoring()
    }
}

/// Main server runtime context container
#[derive(Clone)]
pub struct ServerContext {
    inner: Arc<ServerContextInner>,
}

/// Inner container for server context components
pub struct ServerContextInner {
    /// Cluster node information
    pub node: Node,
    /// Port-to-listener configuration mappings
    pub listen_cfgs: DashMap<ListenerId, ListenerConfig>,
    /// Command line arguments
    pub args: CommandArgs,
    /// Extension point manager
    pub extends: extend::Manager,
    #[cfg(feature = "plugin")]
    /// Plugin management system
    pub plugins: plugin::Manager,
    #[cfg(feature = "metrics")]
    /// Metrics collection system
    pub metrics: Metrics,
    #[cfg(feature = "stats")]
    /// Statistical data tracking
    pub stats: Stats,
    /// Handshake process executor
    pub handshake_exec: HandshakeExecutor,
    /// Task execution queues
    execs: DashMap<&'static str, TaskExecQueue>,

    /// Busy state check flag
    pub busy_check_enable: bool,
    /// Delayed publish message limit
    pub mqtt_delayed_publish_max: usize,
    /// Maximum allowed MQTT sessions
    pub mqtt_max_sessions: isize,
    /// Immediate delayed publish flag
    pub mqtt_delayed_publish_immediate: bool,

    /// Active handshake counter
    pub handshakings: Counter,
    /// Established connection counter
    pub connections: Counter,
    /// Active session counter
    pub sessions: Counter,

    task_exec_workers: usize,
    task_exec_queue_max: usize,
}

impl Deref for ServerContext {
    type Target = ServerContextInner;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

impl ServerContext {
    /// Creates new ServerContextBuilder instance
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> ServerContextBuilder {
        ServerContextBuilder::new()
    }

    /// Configures core system components
    async fn config(self) -> Self {
        *self.extends.shared_mut().await = Box::new(DefaultShared::new(Some(self.clone())));
        *self.extends.router_mut().await = Box::new(DefaultRouter::new(Some(self.clone())));
        #[cfg(feature = "delayed")]
        {
            *self.extends.delayed_sender_mut().await =
                Box::new(DefaultDelayedSender::new(Some(self.clone())));
        }
        self
    }

    /// Starts a background asynchronous task that periodically updates the node's CPU load.
    ///
    /// If `busy_check_enable` is set to `true`, this function will:
    /// - Clone the current `ServerContext` instance.
    /// - Spawn a Tokio task that runs in an infinite loop.
    /// - In each loop iteration:
    ///   1. Sleep for `node.busy_update_interval`.
    ///   2. Call `node.update_cpuload()` to refresh the CPU load metrics.
    ///
    /// This method is typically used for monitoring server load to support
    /// busy-state detection and performance management.
    ///
    /// Returns the original `ServerContext` so the call can be chained.
    fn start_cpuload_monitoring(self) -> ServerContext {
        if self.busy_check_enable {
            let scx = self.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(scx.node.busy_update_interval).await;
                    scx.node.update_cpuload().await;
                }
            });
        }
        self
    }

    /// Checks if server is in busy state
    /// # Returns
    /// true if server is overloaded or at capacity
    #[inline]
    pub async fn is_busy(&self) -> bool {
        if self.busy_check_enable {
            self.handshake_exec.is_busy(self).await || self.node.sys_is_busy()
        } else {
            false
        }
    }

    #[inline]
    pub fn get_exec<K: ExecKey>(&self, key: K) -> TaskExecQueue {
        self.execs
            .entry(key.get())
            .or_insert_with(|| {
                let (exec, task_runner) = Builder::default()
                    .workers(key.workers().unwrap_or(self.task_exec_workers))
                    .queue_max(key.queue_max().unwrap_or(self.task_exec_queue_max))
                    .build();

                tokio::spawn(async move {
                    task_runner.await;
                });
                exec
            })
            .value()
            .clone()
    }

    #[inline]
    pub fn execs(&self) -> HashMap<String, TaskExecQueue> {
        self.execs.iter().map(|entry| (entry.key().to_string(), entry.value().clone())).collect()
    }
}

impl fmt::Debug for ServerContext {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "ServerContext node: {:?}, \
            handshake_exec.active_count: {}, \
            busy_check_enable: {}, mqtt_delayed_publish_max: {}, \
            mqtt_delayed_publish_immediate: {}, mqtt_max_sessions: {}",
            self.node,
            self.handshake_exec.active_count(),
            self.busy_check_enable,
            self.mqtt_delayed_publish_max,
            self.mqtt_delayed_publish_immediate,
            self.mqtt_max_sessions
        )?;
        Ok(())
    }
}

pub type ExecWorkers = usize;
pub type ExecQueueMax = usize;

pub trait ExecKey {
    fn get(&self) -> &'static str;
    fn workers(&self) -> Option<ExecWorkers>;
    fn queue_max(&self) -> Option<ExecQueueMax>;
}

impl ExecKey for &'static str {
    fn get(&self) -> &'static str {
        self
    }

    fn workers(&self) -> Option<ExecWorkers> {
        None
    }

    fn queue_max(&self) -> Option<ExecQueueMax> {
        None
    }
}

impl ExecKey for (&'static str, ExecWorkers, ExecQueueMax) {
    fn get(&self) -> &'static str {
        self.0
    }

    fn workers(&self) -> Option<ExecWorkers> {
        Some(self.1)
    }

    fn queue_max(&self) -> Option<ExecQueueMax> {
        Some(self.2)
    }
}

/// Execution statistics for task queues
#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct TaskExecStats {
    /// Currently active tasks
    pub active_count: isize,
    /// Total completed tasks
    pub completed_count: isize,
    /// Pending wake-up notifications
    pub pending_wakers_count: usize,
    /// Waiting wake-up notifications
    pub waiting_wakers_count: usize,
    /// Tasks waiting in queue
    pub waiting_count: isize,
    /// Tasks processed per second
    pub rate: f64,
}

impl TaskExecStats {
    /// Collects statistics from task execution queue
    /// # Arguments
    /// * `exec` - Task queue to analyze
    #[inline]
    pub async fn from_exec(exec: &TaskExecQueue) -> Self {
        Self {
            active_count: exec.active_count(),
            completed_count: exec.completed_count().await,
            pending_wakers_count: exec.pending_wakers_count(),
            waiting_wakers_count: exec.waiting_wakers_count(),
            waiting_count: exec.waiting_count(),
            rate: exec.rate().await,
        }
    }

    /// Combines two statistics instances (consuming self)
    #[inline]
    fn add2(mut self, other: &Self) -> Self {
        self.add(other);
        self
    }

    /// Aggregates statistics from another instance
    #[inline]
    pub fn add(&mut self, other: &Self) {
        self.active_count += other.active_count;
        self.completed_count += other.completed_count;
        self.pending_wakers_count += other.pending_wakers_count;
        self.waiting_wakers_count += other.waiting_wakers_count;
        self.waiting_count += other.waiting_count;
        self.rate += other.rate;
    }
}

impl Sum for TaskExecStats {
    fn sum<I: Iterator<Item = TaskExecStats>>(iter: I) -> Self {
        iter.fold(TaskExecStats::default(), |acc, x| acc.add2(&x))
    }
}

impl Sum<&'static TaskExecStats> for TaskExecStats {
    fn sum<I: Iterator<Item = &'static TaskExecStats>>(iter: I) -> Self {
        iter.fold(TaskExecStats::default(), |acc, x| acc.add2(x))
    }
}
