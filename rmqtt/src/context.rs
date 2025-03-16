use std::fmt;
use std::iter::Sum;
use std::ops::Deref;
use std::sync::Arc;

use rust_box::task_exec_queue::{Builder, TaskExecQueue};

use crate::conf::Settings;
use crate::delayed::DefaultDelayedSender;
use crate::executor::HandshakeExecutor;
use crate::logger::Logger;
use crate::metrics::Metrics;
use crate::node::Node;
use crate::router::DefaultRouter;
use crate::shared::DefaultShared;
use crate::stats::Stats;
use crate::{extend, plugin};

#[derive(Clone)]
pub struct ServerContext {
    inner: Arc<ServerContextInner>,
}

pub struct ServerContextInner {
    pub settings: Settings,
    pub logger: Logger,
    pub node: Node,
    pub extends: extend::Manager,
    pub plugins: plugin::Manager,
    pub metrics: Metrics,
    pub stats: Stats,
    pub handshake_exec: HandshakeExecutor,
    pub global_exec: TaskExecQueue,
    // pub sched: JobScheduler,
}

impl Deref for ServerContext {
    type Target = ServerContextInner;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

// impl Default for ServerContext {
//     fn default() -> Self {
//         Self::new(Node::default())
//     }
// }

impl ServerContext {
    pub fn new(settings: Settings, logger: Logger, node: Node) -> Self {
        let busy_limit = settings.node.busy.handshaking as isize;

        let (global_exec, task_runner) = Builder::default()
            .workers(settings.task.exec_workers)
            .queue_max(settings.task.exec_queue_max)
            .build();

        tokio::spawn(async move {
            task_runner.await;
        });

        ServerContext {
            inner: Arc::new(ServerContextInner {
                settings,
                logger,
                node,
                extends: extend::Manager::new(),
                plugins: plugin::Manager::new(),
                metrics: Metrics::new(),
                stats: Stats::new(),
                handshake_exec: HandshakeExecutor::new(busy_limit),
                global_exec,
            }),
        }
    }

    pub async fn config(self) -> Self {
        *self.extends.shared_mut().await = Box::new(DefaultShared::new(Some(self.clone())));
        *self.extends.router_mut().await = Box::new(DefaultRouter::new(Some(self.clone())));
        *self.extends.delayed_sender_mut().await = Box::new(DefaultDelayedSender::new(Some(self.clone())));

        self
    }

    #[inline]
    pub async fn is_busy(&self) -> bool {
        if self.settings.node.busy.check_enable {
            self.handshake_exec.is_busy(self).await || self.node.sys_is_busy()
        } else {
            false
        }
    }
}

impl fmt::Debug for ServerContext {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ServerContext ...")?;
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

    // #[inline]
    // pub fn from_local_exec() -> Self {
    //     get_local_stats()
    // }

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

// static LOCAL_ACTIVE_COUNTS: OnceCell<DashMap<ThreadId, TaskExecStats>> = OnceCell::new();
//
// #[inline]
// async fn set_local_stats(exec: &LocalTaskExecQueue) {
//     let active_counts = LOCAL_ACTIVE_COUNTS.get_or_init(DashMap::default);
//     let mut entry = active_counts.entry(std::thread::current().id()).or_default();
//     let stats = entry.value_mut();
//     stats.active_count = exec.active_count();
//     stats.completed_count = exec.completed_count().await;
//     stats.pending_wakers_count = exec.pending_wakers_count();
//     stats.waiting_wakers_count = exec.waiting_wakers_count();
//     stats.waiting_count = exec.waiting_count();
//     stats.rate = exec.rate().await;
// }
//
// #[inline]
// fn get_local_stats() -> TaskExecStats {
//     LOCAL_ACTIVE_COUNTS.get().map(|m| m.iter().map(|item| item.value().clone()).sum()).unwrap_or_default()
// }
