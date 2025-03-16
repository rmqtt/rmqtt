use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::{anyhow, Result};
use crossbeam::queue::SegQueue;
use futures::channel::mpsc;
use futures::SinkExt;
use futures::Stream;
use governor::{
    clock::DefaultClock,
    middleware::NoOpMiddleware,
    prelude::StreamRateLimitExt,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter, RatelimitedStream,
};

type DirectLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

pub type Receiver<'a, T> =
    RatelimitedStream<'a, ReceiverStream<T>, InMemoryState, DefaultClock, NoOpMiddleware>;

pub enum Policy {
    //Discard current value
    Current,
    //Discard earliest value
    Early,
}

pub trait PolicyFn<P>: 'static + Sync + Send + Fn(&P) -> Policy {}

impl<T, P> PolicyFn<P> for T where T: 'static + Sync + Send + Clone + Fn(&P) -> Policy {}

pub trait OnEventFn: 'static + Sync + Send + Fn() {}
impl<T> OnEventFn for T where T: 'static + Sync + Send + Clone + Fn() {}

// #[derive(Clone)]
pub struct Sender<T> {
    tx: mpsc::Sender<()>,
    queue: Arc<Queue<T>>,
    policy_fn: Arc<dyn PolicyFn<T>>,
}

impl<T> Sender<T> {
    #[inline]
    pub async fn close(&mut self) -> Result<()> {
        self.tx.close().await?;
        Ok(())
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[inline]
    pub fn policy<F>(mut self, f: F) -> Self
    where
        F: PolicyFn<T>, // + Clone,
    {
        self.policy_fn = Arc::new(f);
        self
    }

    ///If the queue is full, the data is discarded according to the policy
    #[inline]
    pub async fn send(&self, v: T) -> Result<(), T> {
        if let Err(v) = self.queue.push(v) {
            match (self.policy_fn)(&v) {
                Policy::Current => return Err(v),
                Policy::Early => {
                    let removed = self.queue.pop();
                    if let Err(v) = self.queue.push(v) {
                        log::warn!("queue is full, queue len is {}", self.queue.len());
                        return Err(v);
                    }
                    if let Some(removed) = removed {
                        return Err(removed);
                    } else {
                        return Ok(());
                    }
                }
            }
        } else if let Err(e) = self.tx.clone().try_send(()) {
            log::warn!("channel is full, {:?}", e);
        }
        Ok(())
    }

    #[inline]
    pub fn pop(&self) -> Option<T> {
        self.queue.pop()
    }
}

pub struct ReceiverStream<T> {
    rx: mpsc::Receiver<()>,
    queue: Arc<Queue<T>>,
}

impl<T> Stream for ReceiverStream<T> {
    type Item = Option<T>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let result: Option<_> = futures::ready!(Pin::new(&mut self.rx).poll_next(cx));
        Poll::Ready(match result {
            Some(_) => Some(self.queue.pop()),
            None => None,
        })
    }
}

pub struct Limiter {
    l: DirectLimiter,
}

impl Limiter {
    #[inline]
    pub fn new(burst: NonZeroU32, replenish_n_per: Duration) -> Result<Self> {
        if replenish_n_per.as_nanos() == 0 {
            return Err(anyhow!("illegal parameter, replenish_n_per is 0"));
        }
        let period = replenish_n_per.as_nanos() as u64 / burst.get() as u64;
        let period = if period > 0 { Duration::from_nanos(period) } else { Duration::from_nanos(1) };
        log::debug!("burst: {:?}, {:?}, {:?}", burst, replenish_n_per, period);
        let q = Quota::with_period(period).ok_or_else(|| anyhow!("period is 0"))?.allow_burst(burst);
        let l = RateLimiter::direct(q);
        Ok(Self { l })
    }

    #[inline]
    pub fn channel<T>(&self, queue: Arc<Queue<T>>) -> (Sender<T>, Receiver<'_, T>) {
        let (tx, rx) = mpsc::channel::<()>((queue.capacity() as f64 * 1.5) as usize);
        let s = ReceiverStream { rx, queue: queue.clone() }.ratelimit_stream(&self.l);
        (0..queue.len()).for_each(|_| {
            if let Err(e) = tx.clone().try_send(()) {
                //send offline message
                log::warn!("channel is full, {:?}", e);
            }
        });
        (Sender { tx, queue, policy_fn: Arc::new(|_v: &T| -> Policy { Policy::Current }) }, s)
    }
}

pub struct Queue<T> {
    cap: usize,
    inner: SegQueue<T>,
    on_push_fn: Option<Arc<dyn OnEventFn>>,
    on_pop_fn: Option<Arc<dyn OnEventFn>>,
}

impl<T> Drop for Queue<T> {
    #[inline]
    fn drop(&mut self) {
        log::debug!("Queue Drop ... len: {}", self.len());
    }
}

impl<T> Queue<T> {
    #[inline]
    pub fn new(cap: usize) -> Self {
        Self { cap, inner: SegQueue::new(), on_push_fn: None, on_pop_fn: None }
    }

    #[inline]
    pub fn on_push<F>(&mut self, f: F)
    where
        F: OnEventFn,
    {
        self.on_push_fn = Some(Arc::new(f));
    }

    #[inline]
    pub fn on_pop<F>(&mut self, f: F)
    where
        F: OnEventFn,
    {
        self.on_pop_fn = Some(Arc::new(f));
    }

    #[inline]
    pub fn push(&self, v: T) -> Result<(), T> {
        if self.inner.len() > self.cap {
            return Err(v);
        }
        if let Some(f) = self.on_push_fn.as_ref() {
            f();
        }
        self.inner.push(v);
        Ok(())
    }

    #[inline]
    pub fn pop(&self) -> Option<T> {
        if let Some(v) = self.inner.pop() {
            if let Some(f) = self.on_pop_fn.as_ref() {
                f();
            }
            Some(v)
        } else {
            None
        }
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.cap
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

mod test {
    #[tokio::main]
    #[test]
    async fn channel() {
        use super::Policy;
        use super::Queue;
        use chrono::Local;
        use futures::StreamExt;
        use std::num::NonZeroU32;
        use std::sync::Arc;
        use std::time::Duration;

        let limiter = super::Limiter::new(NonZeroU32::new(1).unwrap(), Duration::from_millis(100)).unwrap();
        let (tx, mut rx) = limiter.channel::<u64>(Arc::new(Queue::new(100)));

        let tx = tx.policy(|_v: &u64| -> Policy { Policy::Early });

        tokio::spawn(async move {
            for i in 0..20 {
                if let Err(e) = tx.send(i).await {
                    log::warn!("[send] {} => removed: {:?}  queue len: {}", i, e, tx.len());
                } else {
                    //log::info!("[send] {} => Ok   {}", i, tx.len());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });

        while let Some(v) = rx.next().await {
            println!("{} queue recv: {:?}", Local::now().format("%Y-%m-%d %H:%M:%S%.3f %z"), v);
        }
    }
}
