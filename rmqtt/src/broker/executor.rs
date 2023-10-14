use std::thread::ThreadId;
use std::time::Duration;

use once_cell::sync::OnceCell;
use rust_box::task_exec_queue::{LocalBuilder, LocalTaskExecQueue};
use tokio::task::spawn_local;

use crate::broker::types::*;
use crate::settings::listener::Listener;

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

                let exec1 = exec.clone();
                spawn_local(async move {
                    futures::future::join(task_runner, async move {
                        loop {
                            set_active_count(name, exec1.active_count());
                            set_rate(name, exec1.rate().await);
                            tokio::time::sleep(Duration::from_secs(5)).await;
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

static ACTIVE_COUNTS: OnceCell<DashMap<(Port, ThreadId), isize>> = OnceCell::new();

fn set_active_count(name: Port, c: isize) {
    let active_counts = ACTIVE_COUNTS.get_or_init(DashMap::default);
    let mut entry = active_counts.entry((name, std::thread::current().id())).or_default();
    *entry.value_mut() = c;
}

pub fn get_active_count() -> isize {
    ACTIVE_COUNTS.get().map(|m| m.iter().map(|item| *item.value()).sum()).unwrap_or_default()
}

static RATES: OnceCell<DashMap<(Port, ThreadId), f64>> = OnceCell::new();

fn set_rate(name: Port, rate: f64) {
    let rates = RATES.get_or_init(DashMap::default);
    let mut entry = rates.entry((name, std::thread::current().id())).or_default();
    *entry.value_mut() = rate;
}

pub fn get_rate() -> f64 {
    RATES.get().map(|m| m.iter().map(|entry| *entry.value()).sum::<f64>()).unwrap_or_default()
}
