use once_cell::sync::OnceCell;
use rust_box::task_exec_queue::{Builder, TaskExecQueue};
use std::fmt;
use tokio::spawn;
use tokio_cron_scheduler::JobScheduler;

use crate::logger::{config_logger, Logger};
use crate::{
    broker::{metrics::Metrics, stats::Stats},
    extend,
    node::Node,
    plugin,
    settings::Settings,
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
        let (exec, task_runner) = Builder::default().workers(100).queue_max(100_000).build();
        spawn(async move {
            task_runner.await;
        });

        let sched = JobScheduler::new().await.unwrap();
        sched.start().await.unwrap();

        let settings = Settings::instance();
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
}

impl fmt::Debug for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Runtime ...")?;
        Ok(())
    }
}
