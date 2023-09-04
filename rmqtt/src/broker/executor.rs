use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::task::{Context, Poll};
use std::thread::ThreadId;
use std::time::Duration;

use crossbeam::queue::SegQueue;
use futures::task::AtomicWaker;
use once_cell::sync::OnceCell;
//use parking_lot::RwLock;
use tokio::sync::RwLock;
use update_rate::{DiscreteRateCounter, RateCounter};

use crate::broker::types::*;
use crate::settings::listener::Listener;
use crate::{MqttError, Result, Runtime};

pub type Port = u16;

std::thread_local! {
    pub static LOCAL_HANDSHAKE_EXECUTORS: DashMap<Port, Executor> = DashMap::default();
}

#[inline]
pub(crate) async fn get_handshake_exec(name: Port, listen_cfg: Listener) -> Executor {
    let exec = LOCAL_HANDSHAKE_EXECUTORS.with(|m| {
        m.entry(name)
            .or_insert_with(|| {
                Executor::new(
                    (listen_cfg.max_handshaking_limit / listen_cfg.workers) as isize,
                    listen_cfg.handshake_timeout,
                )
            })
            .value()
            .clone()
    });

    set_active_count(name, exec.active_count());
    set_rate(name, exec.rate().await);
    exec
}

static ACTIVE_COUNTS: OnceCell<DashMap<(Port, ThreadId), (isize, Timestamp)>> = OnceCell::new();

fn set_active_count(name: Port, c: isize) {
    let active_counts = ACTIVE_COUNTS.get_or_init(DashMap::default);
    let mut entry = active_counts.entry((name, std::thread::current().id())).or_default();
    let (count, t) = entry.value_mut();
    *count = c;
    *t = chrono::Local::now().timestamp();
}

pub fn get_active_count() -> isize {
    ACTIVE_COUNTS
        .get()
        .map(|m| {
            m.iter()
                .filter_map(|entry| {
                    let (c, t) = entry.value();
                    if *t + 5 > chrono::Local::now().timestamp() {
                        Some(*c)
                    } else {
                        None
                    }
                })
                .sum()
        })
        .unwrap_or_default()
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

#[derive(Clone)]
pub struct Executor {
    inner: Rc<ExecutorInner>,
}

struct ExecutorInner {
    max_handshake_limit: isize,
    handshake_timeout: Duration,
    pending_wakers: SegQueue<Rc<AtomicWaker>>,
    active_count: AtomicIsize,
    rate_counter: RwLock<DiscreteRateCounter>,
}

impl Executor {
    pub fn new(max_handshake_limit: isize, handshake_timeout: Duration) -> Self {
        Self {
            inner: Rc::new(ExecutorInner {
                max_handshake_limit,
                handshake_timeout,
                pending_wakers: SegQueue::new(),
                active_count: AtomicIsize::new(0),
                rate_counter: RwLock::new(DiscreteRateCounter::new(100)),
            }),
        }
    }

    #[inline]
    pub async fn spawn<T>(self, future: T) -> Result<T::Output>
    where
        T: Future + 'static,
        T::Output: 'static,
    {
        if self.inner.active_count.load(Ordering::SeqCst) >= self.inner.max_handshake_limit {
            let now = std::time::Instant::now();
            let w = Rc::new(AtomicWaker::new());
            self.inner.pending_wakers.push(w.clone());
            let delay = tokio::time::sleep(self.inner.handshake_timeout);
            tokio::pin!(delay);
            tokio::select! {
                _ = &mut delay => {
                    log::debug!("is timeout ... {:?} {:?}", now.elapsed(), self.inner.handshake_timeout);
                    Runtime::instance().metrics.client_handshaking_timeout_inc();
                    return Err(MqttError::from(format!(
                        "handshake timeout, acquire cost time: {:?}",
                        now.elapsed()
                    )));
                },
                _ = PendingOnce::new(w) => {
                    log::debug!("is waked ... {:?}", now.elapsed());
                }
            }
        }

        self.inner.active_count.fetch_add(1, Ordering::SeqCst);
        let output = future.await;
        self.inner.active_count.fetch_sub(1, Ordering::SeqCst);
        if let Some(w) = self.inner.pending_wakers.pop() {
            w.wake();
        }
        self.inner.rate_counter.write().await.update();
        Ok(output)
    }

    #[inline]
    pub fn active_count(&self) -> isize {
        self.inner.active_count.load(Ordering::SeqCst)
    }

    #[inline]
    pub async fn rate(&self) -> f64 {
        self.inner.rate_counter.read().await.rate()
    }
}

struct PendingOnce {
    w: Rc<AtomicWaker>,
    is_ready: bool,
}

impl PendingOnce {
    #[inline]
    fn new(w: Rc<AtomicWaker>) -> Self {
        Self { w, is_ready: false }
    }
}

impl Future for PendingOnce {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.is_ready {
            Poll::Ready(())
        } else {
            self.w.register(cx.waker());
            self.is_ready = true;
            Poll::Pending
        }
    }
}
