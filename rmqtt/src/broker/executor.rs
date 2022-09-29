use std::pin::Pin;
use std::sync::Arc;
use std::rc::Rc;
use std::task::{Context, Poll};
use std::ops::{DerefMut, Deref};
use std::thread::ThreadId;
use futures::{Sink, Stream};
use linked_hash_map::LinkedHashMap;
use parking_lot::RwLock;
use tokio::task::spawn_local;
use once_cell::sync::OnceCell;

use rust_box::queue_ext::{Action, QueueExt, Reply, SendError};
use rust_box::task_executor::{LocalBuilder, LocalExecutor, LocalTaskType};


use crate::broker::types::*;
use crate::settings::listener::Listener;
use crate::{ClientId, Runtime};

pub type Port = u16;

type DataType = (ClientId, LocalTaskType);
type ErrorType = SendError<DataType>;

type ExecutorType = LocalExecutor<TxSink, (), ClientId>;

pub trait SinkType: futures::Sink<DataType, Error = ErrorType> + Sync + Unpin + 'static {}
impl<T> SinkType for T
    where
        T: Sink<DataType, Error = ErrorType>  + Sync + Unpin + 'static,
{}

std::thread_local! {
    pub static HANDSHAKE_EXECUTORS: DashMap<Port, ExecutorType> = DashMap::default();
}

#[inline]
pub(crate) fn get_handshake_executor(name: Port, listen_cfg: Listener) -> ExecutorType {
    let exec = HANDSHAKE_EXECUTORS.with(|m| {
        m.entry(name)
            .or_insert_with(|| {
                let (tx, rx) = channel(listen_cfg.max_connections / listen_cfg.workers);
                let (exec, task_runner) = LocalBuilder::default()
                    .workers(listen_cfg.max_handshake_limit / listen_cfg.workers)
                    .queue_max(listen_cfg.max_connections / listen_cfg.workers)
                    .with_channel(tx, rx)
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
            }).sum::<f64>() / m.len() as f64).unwrap_or_default()
}

fn channel(cap: usize)-> (TxSink, impl Stream<Item=DataType>)
{
    let (tx, rx) = Arc::new(RwLock::new(LinkedHashMap::new())).queue_channel(
        move |s, act| match act {
            Action::Send((key, val)) => {
                let mut s = s.write();
                if s.contains_key(&key) {
                    Runtime::instance().metrics.client_handshaking_reentry_inc();
                    s.remove(&key);
                }
                s.insert(key, val);
                Reply::Send(())
            }
            Action::IsFull => Reply::IsFull(s.read().len() >= cap),
            Action::IsEmpty => Reply::IsEmpty(s.read().is_empty()),
        },
        |s, _| {
            let mut s = s.write();
            if s.is_empty() {
                Poll::Pending
            } else {
                match s.pop_front() {
                    Some(m) => Poll::Ready(Some(m)),
                    None => Poll::Pending,
                }
            }
        },
    );
    (TxSink(Rc::new(RwLock::new(tx))), rx)
}

#[derive(Clone)]
pub struct TxSink(Rc<RwLock<dyn SinkType>>);

impl Deref for TxSink {
    type Target = Rc<RwLock<dyn SinkType>>;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for TxSink {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

unsafe impl Sync for TxSink {}
impl Unpin for TxSink {}

impl futures::Sink<DataType> for TxSink{
    type Error = ErrorType;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(self.0.write()).as_mut().poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: DataType) -> Result<(), Self::Error> {
        Pin::new(self.0.write()).as_mut().start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(self.0.write()).as_mut().poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Pin::new(self.0.write()).as_mut().poll_close(cx)
    }
}
