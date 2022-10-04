use std::thread::ThreadId;
use tokio::task::spawn_local;
use once_cell::sync::OnceCell;

use rust_box::task_executor::{LocalBuilder, LocalExecutor};

use crate::broker::types::*;
use crate::settings::listener::Listener;

pub type Port = u16;

std::thread_local! {
    pub static HANDSHAKE_EXECUTORS: DashMap<Port, LocalExecutor> = DashMap::default();
}

#[inline]
pub(crate) fn get_handshake_executor(name: Port, listen_cfg: Listener) -> LocalExecutor {
    let exec = HANDSHAKE_EXECUTORS.with(|m| {
        m.entry(name)
            .or_insert_with(|| {
                let (exec, task_runner) = LocalBuilder::default()
                    .workers(listen_cfg.max_handshaking_limit / listen_cfg.workers)
                    .queue_max(listen_cfg.max_connections / listen_cfg.workers)
                    .build();

                spawn_local(async move{
                    task_runner.await;
                });

                exec
            })
            .value()
            .clone()
    });

    set_active_count(name, exec.active_count());
    set_rate(name, exec.rate());
    exec
}

static ACTIVE_COUNTS: OnceCell<DashMap<(Port, ThreadId), (isize, Timestamp)>> = OnceCell::new();

fn set_active_count(name: Port, c: isize){
    let active_counts = ACTIVE_COUNTS.get_or_init(DashMap::default);
    let mut entry = active_counts.entry((name, std::thread::current().id())).or_default();
    let (count, t) = entry.value_mut();
    *count = c;
    *t = chrono::Local::now().timestamp();
}

pub fn get_active_count() -> isize {
    ACTIVE_COUNTS.get()
        .map(|m| m.iter()
            .filter_map(|entry|{
                let (c, t) = entry.value();
                if *t + 5 > chrono::Local::now().timestamp(){
                    Some(*c)
                }else{
                    None
                }
            }).sum()).unwrap_or_default()
}

static RATES: OnceCell<DashMap<(Port, ThreadId), f64>> = OnceCell::new();

fn set_rate(name: Port, rate: f64){
    let rates = RATES.get_or_init(DashMap::default);
    let mut entry = rates.entry((name, std::thread::current().id())).or_default();
    *entry.value_mut() = rate;
}

pub fn get_rate() -> f64 {
    RATES.get()
        .map(|m| m.iter()
            .map(|entry|{
                *entry.value()
            }).sum::<f64>()).unwrap_or_default()
}

