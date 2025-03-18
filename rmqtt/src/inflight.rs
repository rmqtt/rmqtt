use std::collections::BTreeSet;
use std::num::NonZeroU16;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use itertools::Itertools;
use rmqtt_net::MqttError;
use rust_box::dequemap::DequeBTreeMap as DequeMap;

use crate::context::ServerContext;
use crate::queue::OnEventFn;
use crate::types::{From, PacketId, Publish, TimestampMillis};
use crate::utils::timestamp_millis;
use crate::{QoS, Reason, Result};

type OutQueues = DequeMap<PacketId, OutInflightMessage>;

#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
pub enum MomentStatus {
    UnAck,
    UnReceived,
    UnComplete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutInflightMessage {
    pub publish: Publish,
    pub from: From,
    pub status: MomentStatus,
    pub update_time: TimestampMillis,
}

impl OutInflightMessage {
    #[inline]
    pub fn new(status: MomentStatus, from: From, publish: Publish) -> Self {
        Self { publish, from, status, update_time: timestamp_millis() }
    }

    #[inline]
    fn update_status(&mut self, status: MomentStatus) {
        self.update_time = timestamp_millis();
        self.status = status;
    }

    #[inline]
    pub fn timeout(&self, interval_millis: TimestampMillis) -> bool {
        log::debug!("interval_millis:{} {}", interval_millis, timestamp_millis() - self.update_time);
        interval_millis > 0 && ((timestamp_millis() - self.update_time) >= interval_millis)
    }
}

#[derive(Clone)]
pub struct OutInflight {
    cap: usize,
    interval: TimestampMillis,
    next: Arc<AtomicU16>,
    queues: OutQueues,
    on_push_fn: Option<Arc<dyn OnEventFn>>,
    on_pop_fn: Option<Arc<dyn OnEventFn>>,
}

impl OutInflight {
    #[inline]
    pub fn new(cap: usize, retry_interval: TimestampMillis, expiry_interval: TimestampMillis) -> Self {
        let interval = Self::interval(retry_interval, expiry_interval);
        Self {
            cap,
            interval,
            next: Arc::new(AtomicU16::new(1)),
            queues: OutQueues::default(),
            on_push_fn: None,
            on_pop_fn: None,
        }
    }

    #[inline]
    pub fn on_push<F>(mut self, f: F) -> Self
    where
        F: OnEventFn,
    {
        self.on_push_fn = Some(Arc::new(f));
        self
    }

    #[inline]
    pub fn on_pop<F>(mut self, f: F) -> Self
    where
        F: OnEventFn,
    {
        self.on_pop_fn = Some(Arc::new(f));
        self
    }

    #[inline]
    fn interval(retry_interval: TimestampMillis, expiry_interval: TimestampMillis) -> TimestampMillis {
        match (retry_interval, expiry_interval) {
            (0, 0) => 0,
            (0, expiry_interval) => expiry_interval,
            (retry_interval, 0) => retry_interval,
            (retry_interval, expiry_interval) => retry_interval.min(expiry_interval),
        }
    }

    #[inline]
    pub fn get_timeout(&self) -> Option<Duration> {
        if self.interval == 0 {
            return None;
        }
        if let Some((_, m)) = self.queues.front() {
            let mut t = self.interval - (timestamp_millis() - m.update_time);
            if t < 1 {
                t = 1;
            }
            log::debug!("get timeout t: {}", t);
            return Some(Duration::from_millis(t as u64));
        }
        None
    }

    #[inline]
    fn front_timeout(&self) -> bool {
        if self.interval == 0 {
            return false;
        }
        if let Some((_, m)) = self.queues.front() {
            if m.timeout(self.interval) {
                return true;
            }
        }
        false
    }

    #[inline]
    pub fn get(&self, packet_id: PacketId) -> Option<&OutInflightMessage> {
        self.queues.get(&packet_id)
    }

    #[inline]
    pub fn front(&self) -> Option<(&PacketId, &OutInflightMessage)> {
        self.queues.front()
    }

    #[inline]
    pub fn pop_front(&mut self) -> Option<OutInflightMessage> {
        if let Some(msg) = self.queues.pop_front().map(|(_, m)| m) {
            if let Some(f) = self.on_pop_fn.as_ref() {
                f();
            }
            Some(msg)
        } else {
            None
        }
    }

    #[inline]
    pub fn pop_front_timeout(&mut self) -> Option<OutInflightMessage> {
        if self.front_timeout() {
            self.pop_front()
        } else {
            None
        }
    }

    #[inline]
    pub fn push_back(&mut self, m: OutInflightMessage) -> Option<NonZeroU16> {
        if let Some(packet_id) = m.publish.packet_id {
            if let Some(f) = self.on_push_fn.as_ref() {
                f();
            }
            let old = self.queues.insert(packet_id.get(), m);
            if old.is_some() {
                if let Some(f) = self.on_pop_fn.as_ref() {
                    f();
                }
            }
            old.and_then(|old| old.publish.packet_id)
        } else {
            log::warn!("packet_id is None, inflight message: {:?}", m);
            return None;
        }
    }

    #[inline]
    pub fn remove(&mut self, packet_id: &PacketId) -> Option<OutInflightMessage> {
        if let Some(msg) = self.queues.remove(packet_id) {
            if let Some(f) = self.on_pop_fn.as_ref() {
                f();
            }
            Some(msg)
        } else {
            None
        }
    }

    #[inline]
    pub fn update_status(&mut self, packet_id: &PacketId, s: MomentStatus) {
        if let Some(m) = self.queues.get_mut(packet_id) {
            m.update_status(s);
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.queues.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.queues.is_empty()
    }

    #[inline]
    pub fn exist(&self, packet_id: &PacketId) -> bool {
        self.queues.contains_key(packet_id)
    }

    #[inline]
    pub fn has_credit(&self) -> bool {
        (self.cap - self.queues.len()) > 0
    }

    #[inline]
    pub fn next_id(&self) -> Result<PacketId> {
        for _ in 0..u16::MAX {
            let packet_id = self.next.fetch_add(1, Ordering::SeqCst);
            if packet_id == 0 {
                continue;
            }
            if !self.queues.contains_key(&packet_id) {
                return Ok(packet_id);
            }
        }
        Err(anyhow!("no packet_id available, should unreachable!()"))
    }

    #[inline]
    pub fn to_inflight_messages(&mut self) -> Vec<OutInflightMessage> {
        let mut inflight_messages = Vec::new();
        while let Some(msg) = self.pop_front() {
            //@TODO ..., check message expired
            inflight_messages.push(msg);
        }
        inflight_messages
    }

    #[inline]
    pub fn clone_inflight_messages(&mut self) -> Vec<OutInflightMessage> {
        self.queues.iter().map(|(_, msg)| msg.clone()).collect_vec()
    }
}

//@TODO 大小限制，即同一个连接上接收消息的并发限制
pub struct InInflight {
    cached: BTreeSet<NonZeroU16>,
    scx: ServerContext,
    max_inflight: u16,
}

impl Drop for InInflight {
    fn drop(&mut self) {
        self.scx.stats.in_inflights.decs(self.cached.len() as isize);
    }
}

impl InInflight {
    pub(crate) fn new(scx: ServerContext, max_inflight: u16) -> Self {
        Self { cached: BTreeSet::default(), scx, max_inflight }
    }

    #[inline]
    pub(crate) fn add(&mut self, pid: NonZeroU16, qos: QoS) -> std::result::Result<bool, Reason> {
        if self.cached.len() >= self.max_inflight as usize {
            return Err(Reason::InflightWindowFull);
        }
        if self.cached.insert(pid) {
            self.scx.stats.in_inflights.inc();
            Ok(true)
        } else {
            if matches!(qos, QoS::ExactlyOnce) {
                Err(MqttError::PacketIdInUse(pid).into())
            } else {
                Ok(false)
            }
        }
    }

    #[inline]
    pub(crate) fn remove(&mut self, pid: &NonZeroU16) -> bool {
        if self.cached.remove(pid) {
            self.scx.stats.in_inflights.dec();
            true
        } else {
            false
        }
    }
}
