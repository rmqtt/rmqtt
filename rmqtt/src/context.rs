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
use crate::router::DefaultRouter;
use crate::shared::DefaultShared;
#[cfg(feature = "stats")]
use crate::stats::Stats;
use crate::types::{DashMap, ListenerConfig, NodeId, Port};
use crate::utils::Counter;

pub struct ServerContextBuilder {
    args: CommandArgs,
    node: Node,

    task_exec_workers: usize,
    task_exec_queue_max: usize,

    busy_check_enable: bool,
    busy_handshaking_limit: isize,

    mqtt_delayed_publish_max: usize,
    mqtt_max_sessions: isize,
    mqtt_delayed_publish_immediate: bool,

    plugins_dir: String,
}

impl Default for ServerContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerContextBuilder {
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
            plugins_dir: "rmqtt-plugins/".into(),
        }
    }

    pub fn args(mut self, args: CommandArgs) -> Self {
        self.args = args;
        self
    }

    pub fn node_id(mut self, id: NodeId) -> Self {
        self.node.id = id;
        self
    }

    pub fn node(mut self, node: Node) -> Self {
        self.node = node;
        self
    }

    pub fn task_exec_workers(mut self, task_exec_workers: usize) -> Self {
        self.task_exec_workers = task_exec_workers;
        self
    }

    pub fn task_exec_queue_max(mut self, task_exec_queue_max: usize) -> Self {
        self.task_exec_queue_max = task_exec_queue_max;
        self
    }

    pub fn busy_check_enable(mut self, busy_check_enable: bool) -> Self {
        self.busy_check_enable = busy_check_enable;
        self
    }

    pub fn busy_handshaking_limit(mut self, busy_handshaking_limit: isize) -> Self {
        self.busy_handshaking_limit = busy_handshaking_limit;
        self
    }

    pub fn mqtt_delayed_publish_max(mut self, mqtt_delayed_publish_max: usize) -> Self {
        self.mqtt_delayed_publish_max = mqtt_delayed_publish_max;
        self
    }

    pub fn mqtt_max_sessions(mut self, mqtt_max_sessions: isize) -> Self {
        self.mqtt_max_sessions = mqtt_max_sessions;
        self
    }

    pub fn mqtt_delayed_publish_immediate(mut self, mqtt_delayed_publish_immediate: bool) -> Self {
        self.mqtt_delayed_publish_immediate = mqtt_delayed_publish_immediate;
        self
    }

    pub fn plugins_dir<N: Into<String>>(mut self, plugins_dir: N) -> Self {
        self.plugins_dir = plugins_dir.into();
        self
    }

    pub async fn build(self) -> ServerContext {
        let (global_exec, task_runner) =
            Builder::default().workers(self.task_exec_workers).queue_max(self.task_exec_queue_max).build();

        tokio::spawn(async move {
            task_runner.await;
        });

        ServerContext {
            inner: Arc::new(ServerContextInner {
                args: self.args,
                node: self.node,
                listen_cfgs: DashMap::default(),
                extends: extend::Manager::new(),
                #[cfg(feature = "plugin")]
                plugins: plugin::Manager::new(self.plugins_dir),
                #[cfg(feature = "metrics")]
                metrics: Metrics::new(),
                #[cfg(feature = "stats")]
                stats: Stats::new(),
                handshake_exec: HandshakeExecutor::new(self.busy_handshaking_limit),
                global_exec,

                busy_check_enable: self.busy_check_enable,
                mqtt_delayed_publish_max: self.mqtt_delayed_publish_max,
                mqtt_max_sessions: self.mqtt_max_sessions,
                mqtt_delayed_publish_immediate: self.mqtt_delayed_publish_immediate,

                handshakings: Counter::new(),
                connections: Counter::new(),
                sessions: Counter::new(),
            }),
        }
        .config()
        .await
    }
}

#[derive(Clone)]
pub struct ServerContext {
    inner: Arc<ServerContextInner>,
}

pub struct ServerContextInner {
    pub node: Node,
    pub listen_cfgs: DashMap<Port, ListenerConfig>,
    pub args: CommandArgs,
    pub extends: extend::Manager,
    #[cfg(feature = "plugin")]
    pub plugins: plugin::Manager,
    #[cfg(feature = "metrics")]
    pub metrics: Metrics,
    #[cfg(feature = "stats")]
    pub stats: Stats,
    pub handshake_exec: HandshakeExecutor,
    pub global_exec: TaskExecQueue,

    pub busy_check_enable: bool,
    pub mqtt_delayed_publish_max: usize,
    pub mqtt_max_sessions: isize,
    pub mqtt_delayed_publish_immediate: bool,

    pub handshakings: Counter,
    pub connections: Counter,
    pub sessions: Counter,
}

impl Deref for ServerContext {
    type Target = ServerContextInner;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

impl ServerContext {
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> ServerContextBuilder {
        ServerContextBuilder::new()
    }

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

    #[inline]
    pub async fn is_busy(&self) -> bool {
        if self.busy_check_enable {
            self.handshake_exec.is_busy(self).await || self.node.sys_is_busy()
        } else {
            false
        }
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

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct TaskExecStats {
    active_count: isize,
    completed_count: isize,
    pending_wakers_count: usize,
    waiting_wakers_count: usize,
    waiting_count: isize,
    rate: f64,
}

impl TaskExecStats {
    #[inline]
    pub async fn from_global_exec(global_exec: &TaskExecQueue) -> Self {
        Self {
            active_count: global_exec.active_count(),
            completed_count: global_exec.completed_count().await,
            pending_wakers_count: global_exec.pending_wakers_count(),
            waiting_wakers_count: global_exec.waiting_wakers_count(),
            waiting_count: global_exec.waiting_count(),
            rate: global_exec.rate().await,
        }
    }

    #[inline]
    fn add2(mut self, other: &Self) -> Self {
        self.add(other);
        self
    }

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
