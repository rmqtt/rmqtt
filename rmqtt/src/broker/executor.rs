use itertools::Itertools;
use std::ops::Deref;
use std::thread::ThreadId;
use std::time::Duration;
use std::time::Instant;

use counter_rater::Counter;
use ntex_mqtt::handshakings;
use once_cell::sync::{Lazy, OnceCell};
use rust_box::task_exec_queue::{LocalBuilder, LocalTaskExecQueue};
use tokio::sync::RwLock;
use tokio::task::spawn_local;

use crate::broker::types::*;
use crate::settings::listener::Listener;
use crate::Runtime;

pub type Port = u16;

std::thread_local! {
    pub static HANDSHAKE_EXECUTORS: DashMap<Port, LocalTaskExecQueue> = DashMap::default();
}

#[inline]
pub(crate) fn get_handshake_exec(name: Port, listen_cfg: Listener) -> LocalTaskExecQueue {
    HANDSHAKE_EXECUTORS.with(|m| {
        m.entry(name)
            .or_insert_with(|| {
                let (exec, task_runner) = LocalBuilder::default()
                    .workers(listen_cfg.max_handshaking_limit / listen_cfg.workers)
                    .queue_max(listen_cfg.max_connections / listen_cfg.workers)
                    .build();

                let busy_limit = if Runtime::instance().settings.node.busy.handshaking == 0 {
                    (listen_cfg.max_handshaking_limit as f64 * 0.35) as usize
                } else {
                    Runtime::instance().settings.node.busy.handshaking
                };

                set_active_count(name, exec.active_count(), Some(busy_limit));
                let exec1 = exec.clone();
                spawn_local(async move {
                    futures::future::join(task_runner, async move {
                        loop {
                            set_active_count(name, exec1.active_count(), None);
                            set_rate(name, exec1.rate().await);
                            tokio::time::sleep(Duration::from_secs(3)).await;
                        }
                    })
                    .await;
                });

                exec
            })
            .value()
            .clone()
    })
}

static ACTIVE_COUNTS: OnceCell<DashMap<(Port, ThreadId), (isize, isize)>> = OnceCell::new();

#[inline]
fn set_active_count(name: Port, c: isize, handshaking_busy_limit: Option<usize>) {
    let active_counts = ACTIVE_COUNTS.get_or_init(DashMap::default);
    let mut entry = active_counts.entry((name, std::thread::current().id())).or_default();
    let (count, busy_limit) = entry.value_mut();
    *count = c;
    if let Some(handshaking_busy_limit) = handshaking_busy_limit {
        *busy_limit = handshaking_busy_limit as isize;
    }
}

#[inline]
pub async fn is_busy() -> bool {
    #[inline]
    fn _is_busy() -> bool {
        let busies = ACTIVE_COUNTS
            .get()
            .map(|m| {
                m.iter()
                    .chunk_by(|item| (item.key().0, item.value().1))
                    .into_iter()
                    .map(|(k, g)| {
                        (
                            k,
                            g.map(|item| {
                                let (c, _) = item.value();
                                *c
                            })
                            .sum::<isize>(),
                        )
                    })
                    .filter_map(|((_, busy_limit), c)| if c > busy_limit { Some(1) } else { None })
                    .sum::<u32>()
            })
            .unwrap_or_default();
        busies > 0
    }

    use rust_box::std_ext::RwLock;
    static CACHED: Lazy<RwLock<(bool, Instant)>> = Lazy::new(|| RwLock::new((false, Instant::now())));
    {
        let cached = CACHED.read();
        let (busy, inst) = cached.deref();
        if inst.elapsed() < Runtime::instance().settings.node.busy.update_interval {
            return *busy;
        }
    }
    let busy = _is_busy() || Runtime::instance().extends.shared().await.operation_is_busy();
    *CACHED.write() = (busy, Instant::now());
    busy
}

#[inline]
pub fn get_active_count() -> isize {
    ACTIVE_COUNTS
        .get()
        .map(|m| {
            m.iter()
                .map(|item| {
                    let (c, _) = item.value();
                    *c
                })
                .sum()
        })
        .unwrap_or_default()
}

static RATES: OnceCell<DashMap<(Port, ThreadId), f64>> = OnceCell::new();

#[inline]
fn set_rate(name: Port, rate: f64) {
    let rates = RATES.get_or_init(DashMap::default);
    let mut entry = rates.entry((name, std::thread::current().id())).or_default();
    *entry.value_mut() = rate;
}

#[inline]
pub fn get_rate() -> f64 {
    RATES.get().map(|m| m.iter().map(|entry| *entry.value()).sum::<f64>()).unwrap_or_default()
}

#[inline]
pub(crate) fn unavailable_stats() -> &'static Counter {
    static UNAVAILABLE_STATS: OnceCell<Counter> = OnceCell::new();
    UNAVAILABLE_STATS.get_or_init(|| {
        let period = Duration::from_secs(5);
        let c = Counter::new(period);
        c.close_auto_update();
        let c1 = c.clone();
        tokio::spawn(async move {
            let mut delay = None;
            loop {
                tokio::time::sleep(period).await;
                c1.rate_update();
                let fail_rate = c1.rate();
                let is_too_many_unavailable = is_too_many_unavailable().await;
                if fail_rate > 0.0 && !is_too_many_unavailable {
                    log::warn!(
                        "Connection handshake with too many unavailable, switching to TooManyUnavailable::Yes, fail rate: {}",
                        fail_rate
                    );
                    too_many_unavailable_set(TooManyUnavailable::Yes).await;
                    delay = Some(Instant::now());
                } else if (delay.map(|d| d.elapsed().as_secs() > 120).unwrap_or(false))
                    || (is_too_many_unavailable && !is_busy().await && handshakings() < 10)
                {
                    log::info!(
                        "Connection handshake restored to TooManyUnavailable::No, delay: {:?}",
                        delay.map(|d| d.elapsed())
                    );
                    too_many_unavailable_set(TooManyUnavailable::No).await;
                    delay = None;
                }
            }
        });
        c
    })
}

enum TooManyUnavailable {
    Yes,
    No,
}

#[inline]
fn too_many_unavailable() -> &'static RwLock<TooManyUnavailable> {
    static TOO_MANY_UNAVAILABLE: OnceCell<RwLock<TooManyUnavailable>> = OnceCell::new();
    TOO_MANY_UNAVAILABLE.get_or_init(|| RwLock::new(TooManyUnavailable::No))
}

#[inline]
pub(crate) async fn is_too_many_unavailable() -> bool {
    matches!(*too_many_unavailable().read().await, TooManyUnavailable::Yes)
}

#[inline]
async fn too_many_unavailable_set(v: TooManyUnavailable) {
    *too_many_unavailable().write().await = v;
}
