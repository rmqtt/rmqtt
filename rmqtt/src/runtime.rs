use std::fmt;
use std::iter::Sum;
use std::rc::Rc;
use std::thread::ThreadId;
use std::time::Duration;

use once_cell::sync::OnceCell;
use rust_box::stream_ext::LimiterExt;
use rust_box::task_exec_queue::{Builder, LocalBuilder, LocalSender, LocalTaskExecQueue, TaskExecQueue};
use tokio::spawn;
use tokio::task::spawn_local;
use tokio_cron_scheduler::JobScheduler;

use crate::logger::{config_logger, Logger};
use crate::{
    broker::{executor::is_busy as handshake_is_busy, metrics::Metrics, stats::Stats, types::DashMap},
    extend,
    node::Node,
    plugin,
    settings::Settings,
    Result,
};

pub struct Runtime {
    pub logger: Logger,
    pub settings: Settings,
    pub extends: extend::Manager,
    pub plugins: plugin::Manager,
    pub node: Node,
    pub metrics: &'static Metrics,
    pub stats: &'static Stats,
    pub exec: TaskExecQueue,
    pub sched: JobScheduler,
}

static INSTANCE: OnceCell<Runtime> = OnceCell::new();

impl Runtime {
    #[inline]
    pub async fn init() -> &'static Self {
        let settings = Settings::instance();

        let (exec, task_runner) = Builder::default()
            .workers(settings.task.exec_workers)
            .queue_max(settings.task.exec_queue_max)
            .build();

        spawn(async move {
            task_runner.await;
        });

        let sched = JobScheduler::new().await.unwrap();
        sched.start().await.unwrap();

        let r = Self {
            logger: config_logger(settings.log.filename(), settings.log.to, settings.log.level),
            settings: settings.clone(),
            extends: extend::Manager::new(),
            plugins: plugin::Manager::new(),
            node: Node::new(),
            metrics: Metrics::instance(),
            stats: Stats::instance(),
            exec,
            sched,
        };
        INSTANCE.set(r).unwrap();
        return INSTANCE.get().unwrap();
    }

    #[inline]
    pub fn instance() -> &'static Self {
        INSTANCE.get().unwrap()
    }

    #[inline]
    pub fn local_exec() -> Rc<LocalTaskExecQueue> {
        get_local_exec()
    }

    #[inline]
    pub fn local_exec_stats() -> TaskExecStats {
        get_local_stats()
    }

    #[inline]
    pub fn is_busy(&self) -> bool {
        if self.settings.node.busy.check_enable {
            handshake_is_busy() || self.node.sys_is_busy()
        } else {
            false
        }
    }
}

impl fmt::Debug for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Runtime ...")?;
        Ok(())
    }
}

pub async fn scheduler_init() -> Result<()> {
    //Execute every 5 seconds
    if Runtime::instance().settings.node.busy.check_enable {
        let async_job_5 = tokio_cron_scheduler::Job::new_async("*/3 * * * * *", move |_uuid, _l| {
            Box::pin(async move {
                Runtime::instance().node.update_cpuload().await;
            })
        })
        .map_err(anyhow::Error::new)?;
        Runtime::instance().sched.add(async_job_5).await.map_err(anyhow::Error::new)?;
    }

    Ok(())
}

#[inline]
fn get_local_exec() -> Rc<LocalTaskExecQueue> {
    std::thread_local! {
        pub static LOCAL_EXECUTORS: Rc<LocalTaskExecQueue> = {
            let exec_workers = Runtime::instance().settings.task.local_exec_workers;
            let exec_queue_max = Runtime::instance().settings.task.local_exec_queue_max;
            let (tokens, period) = Runtime::instance().settings.task.local_exec_rate_limit;
            let tokens = tokens.get() as usize;

            let rate_limiter = leaky_bucket::RateLimiter::builder()
                .initial(tokens)
                .refill(tokens / 10)
                .interval(period / 10)
                .max(tokens)
                .fair(true)
                .build();

            let (tx, rx) = futures::channel::mpsc::channel(exec_queue_max);

            let (exec, task_runner) = LocalBuilder::default()
                .workers(exec_workers)
                .queue_max(exec_queue_max)
                .with_channel(LocalSender::new(tx), rx.leaky_bucket_limiter(rate_limiter))
                .build();

            let exec1 = exec.clone();

            spawn_local(async move {
                futures::future::join(task_runner, async move {
                    loop {
                        set_local_stats(&exec1).await;
                        tokio::time::sleep(Duration::from_secs(3)).await;
                    }
                })
                .await;
            });
            Rc::new(exec)
        };
    }

    LOCAL_EXECUTORS.with(|exec| exec.clone())
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
    pub async fn from_exec() -> Self {
        let exec = &Runtime::instance().exec;
        Self {
            active_count: exec.active_count(),
            completed_count: exec.completed_count().await,
            pending_wakers_count: exec.pending_wakers_count(),
            waiting_wakers_count: exec.waiting_wakers_count(),
            waiting_count: exec.waiting_count(),
            rate: exec.rate().await,
        }
    }

    #[inline]
    pub fn from_local_exec() -> Self {
        get_local_stats()
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

static LOCAL_ACTIVE_COUNTS: OnceCell<DashMap<ThreadId, TaskExecStats>> = OnceCell::new();

#[inline]
async fn set_local_stats(exec: &LocalTaskExecQueue) {
    let active_counts = LOCAL_ACTIVE_COUNTS.get_or_init(DashMap::default);
    let mut entry = active_counts.entry(std::thread::current().id()).or_default();
    let stats = entry.value_mut();
    stats.active_count = exec.active_count();
    stats.completed_count = exec.completed_count().await;
    stats.pending_wakers_count = exec.pending_wakers_count();
    stats.waiting_wakers_count = exec.waiting_wakers_count();
    stats.waiting_count = exec.waiting_count();
    stats.rate = exec.rate().await;
}

#[inline]
fn get_local_stats() -> TaskExecStats {
    LOCAL_ACTIVE_COUNTS.get().map(|m| m.iter().map(|item| item.value().clone()).sum()).unwrap_or_default()
}
