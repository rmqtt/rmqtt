use rust_box::task_exec_queue::{Builder, TaskExecQueue};
use std::ops::Deref;

use crate::context::ServerContext;
use crate::types::Port;
use crate::{DashMap, ListenerConfig};

type BusyLimit = isize;

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

pub struct HandshakeExecutor {
    handshake_execs: DashMap<Port, Entry>,
    busy_limit: BusyLimit,
}

impl HandshakeExecutor {
    pub fn new(busy_limit: isize) -> Self {
        Self { handshake_execs: DashMap::default(), busy_limit }
    }

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

    #[inline]
    pub fn active_count(&self) -> isize {
        self.handshake_execs.iter().map(|exec| exec.active_count()).sum()
    }

    #[inline]
    pub async fn get_rate(&self) -> f64 {
        let mut rate = 0.0;
        for exec in self.handshake_execs.iter() {
            rate += exec.rate().await;
        }
        rate
    }

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

// #[inline]
// pub(crate) fn unavailable_stats() -> &'static Counter {
//     static UNAVAILABLE_STATS: OnceCell<Counter> = OnceCell::new();
//     UNAVAILABLE_STATS.get_or_init(|| {
//         let period = Duration::from_secs(5);
//         let c = Counter::new(period);
//         c.close_auto_update();
//         let c1 = c.clone();
//         tokio::spawn(async move {
//             let mut delay = None;
//             loop {
//                 tokio::time::sleep(period).await;
//                 c1.rate_update();
//                 let fail_rate = c1.rate();
//                 let is_too_many_unavailable = is_too_many_unavailable().await;
//                 if fail_rate > 0.0 && !is_too_many_unavailable {
//                     log::warn!(
//                         "Connection handshake with too many unavailable, switching to TooManyUnavailable::Yes, fail rate: {}",
//                         fail_rate
//                     );
//                     too_many_unavailable_set(TooManyUnavailable::Yes).await;
//                     delay = Some(Instant::now());
//                 } else if (delay.map(|d| d.elapsed().as_secs() > 120).unwrap_or(false))
//                     || (is_too_many_unavailable && !is_busy().await && handshakings() < 10)
//                 {
//                     log::info!(
//                         "Connection handshake restored to TooManyUnavailable::No, delay: {:?}",
//                         delay.map(|d| d.elapsed())
//                     );
//                     too_many_unavailable_set(TooManyUnavailable::No).await;
//                     delay = None;
//                 }
//             }
//         });
//         c
//     })
// }
//
// enum TooManyUnavailable {
//     Yes,
//     No,
// }
//
// #[inline]
// fn too_many_unavailable() -> &'static RwLock<TooManyUnavailable> {
//     static TOO_MANY_UNAVAILABLE: OnceCell<RwLock<TooManyUnavailable>> = OnceCell::new();
//     TOO_MANY_UNAVAILABLE.get_or_init(|| RwLock::new(TooManyUnavailable::No))
// }
//
// #[inline]
// pub(crate) async fn is_too_many_unavailable() -> bool {
//     matches!(*too_many_unavailable().read().await, TooManyUnavailable::Yes)
// }
//
// #[inline]
// async fn too_many_unavailable_set(v: TooManyUnavailable) {
//     *too_many_unavailable().write().await = v;
// }
