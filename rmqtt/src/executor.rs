//! Handshake Connection Management System
//!
//! Provides controlled execution of MQTT handshake operations with:
//! - Port-specific worker pools
//! - Dynamic busy state detection
//! - Connection rate monitoring
//!
//! ## Core Functionality
//! 1. ​**​Executor Management​**​:
//!    - Creates dedicated TaskExecQueue per listener port
//!    - Auto-scales worker pools based on config limits
//!    - Maintains execution statistics
//!
//! 2. ​**​Busy State Detection​**​:
//!    - Dynamic threshold calculation (35% of max capacity)
//!    - Cross-port busy state aggregation
//!    - Integration with shared server state
//!
//! 3. ​**​Performance Monitoring​**​:
//!    - Per-port execution rate tracking
//!    - Active task count summation
//!    - Async-compatible metrics collection
//!
//! ## Implementation Details
//! - Uses DashMap for concurrent port mapping
//! - Lazy initialization of executor pools
//! - Tokio-based async task spawning
//! - Zero-cost deref to TaskExecQueue
//!
//! Operational Flow:
//! 1. Get executor for specific port (auto-creates if needed)
//! 2. Execute handshake tasks in isolated pool
//! 3. Monitor aggregate system state
//! 4. Trigger busy state when thresholds exceeded

use rust_box::task_exec_queue::{Builder, TaskExecQueue};
use std::ops::Deref;

use crate::context::ServerContext;
use crate::types::{DashMap, ListenerConfig, Port};

type BusyLimit = isize;

/// Per-port handshake task execution entry.
///
/// Wraps a [`TaskExecQueue`] with a busy limit threshold used
/// to determine whether the port is saturated with handshakes.
pub struct Entry {
    exec: TaskExecQueue,
    busy_limit: BusyLimit,
}

impl Deref for Entry {
    type Target = TaskExecQueue;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.exec
    }
}

/// Manages per-port handshake task execution and busy-state detection.
///
/// Creates dedicated `TaskExecQueue` instances for each listener port and
/// provides aggregated busy-state checking across all ports. The busy limit
/// is derived from the listener's handshake capacity or a configured default.
pub struct HandshakeExecutor {
    handshake_execs: DashMap<Port, Entry>,
    busy_limit: BusyLimit,
}

impl HandshakeExecutor {
    /// Creates a new `HandshakeExecutor` with the specified busy limit.
    ///
    /// If `busy_limit` is 0, the limit will be dynamically calculated as
    /// 35% of each listener's `max_handshaking_limit` configuration.
    pub fn new(busy_limit: isize) -> Self {
        Self { handshake_execs: DashMap::default(), busy_limit }
    }

    /// Retrieves (or lazily creates) the handshake executor for the given port.
    ///
    /// # Arguments
    /// * `name` - Listener port identifier
    /// * `listen_cfg` - Listener configuration used to initialize the executor pool
    #[inline]
    pub fn get(&self, name: Port, listen_cfg: &ListenerConfig) -> TaskExecQueue {
        self.handshake_execs
            .entry(name)
            .or_insert_with(|| {
                let (exec, task_runner) = Builder::default()
                    .workers(listen_cfg.max_handshaking_limit)
                    .queue_max(listen_cfg.max_connections)
                    .build();

                tokio::spawn(async move {
                    task_runner.await;
                });

                let busy_limit = if self.busy_limit == 0 {
                    (listen_cfg.max_handshaking_limit as f64 * 0.35) as isize
                } else {
                    self.busy_limit
                };

                Entry { exec, busy_limit }
            })
            .exec
            .clone()
    }

    /// Returns the total number of active handshake tasks across all ports.
    /// Returns the total number of active handshake tasks across all ports.
    #[inline]
    pub fn active_count(&self) -> isize {
        self.handshake_execs.iter().map(|exec| exec.active_count()).sum()
    }

    /// Returns the aggregate handshake processing rate across all ports.
    #[inline]
    pub async fn get_rate(&self) -> f64 {
        let mut rate = 0.0;
        for exec in self.handshake_execs.iter() {
            rate += exec.rate().await;
        }
        rate
    }

    /// Checks whether the server is in a busy state.
    ///
    /// Returns `true` if any port's active handshake count exceeds its busy
    /// limit, or if the shared extension indicates an overload condition.
    /// Checks whether the server is in a busy state.
    ///
    /// Returns `true` if any port's active handshake count exceeds its busy limit
    /// or if the shared subscription layer reports operation overload.
    #[inline]
    pub async fn is_busy(&self, scx: &ServerContext) -> bool {
        let _is_busy = self
            .handshake_execs
            .iter()
            .filter_map(|exec| if exec.active_count() > exec.busy_limit { Some(1) } else { None })
            .sum::<u32>()
            > 0;
        _is_busy || scx.extends.shared().await.operation_is_busy()
    }
}
