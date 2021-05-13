use linked_hash_map::LinkedHashMap;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::broker::types::{From, Packet, PacketId, PacketV3, Publish, TimestampMillis};
use crate::{MqttError, Result};

#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum MomentStatus {
    UnAck,
    UnReceived,
    UnComplete,
}

#[derive(Debug, Clone)]
pub struct InflightMessage {
    pub publish: Publish,
    pub from: From,
    pub status: MomentStatus,
    pub update_time: TimestampMillis,
}

impl InflightMessage {
    #[inline]
    pub fn new(status: MomentStatus, from: From, publish: Publish) -> Self {
        Self { publish, from, status, update_time: chrono::Local::now().timestamp_millis() }
    }

    #[inline]
    fn update_status(&mut self, status: MomentStatus) {
        self.update_time = chrono::Local::now().timestamp_millis();
        self.status = status;
    }

    #[inline]
    pub fn timeout(&self, interval_millis: TimestampMillis) -> bool {
        log::debug!(
            "interval_millis:{} {}",
            interval_millis,
            chrono::Local::now().timestamp_millis() - self.update_time
        );
        interval_millis > 0
            && ((chrono::Local::now().timestamp_millis() - self.update_time) >= interval_millis)
    }

    #[inline]
    pub fn release_packet(&self) -> Option<Packet> {
        match &self.publish {
            Publish::V3(p) => {
                if let Some(packet_id) = p.packet.packet_id {
                    return Some(Packet::V3(PacketV3::PublishRelease { packet_id }));
                }
            }
            Publish::V5(_p) => {
                log::warn!("[MQTT 5] Not implemented");
            }
        }
        None
    }
}

pub struct Inflight {
    cap: usize,
    interval: TimestampMillis,
    next: Arc<AtomicU16>,
    queues: LinkedHashMap<PacketId, InflightMessage, ahash::RandomState>,
}

impl Inflight {
    #[inline]
    pub fn new(cap: usize, retry_interval: TimestampMillis, expiry_interval: TimestampMillis) -> Self {
        let interval = Self::interval(retry_interval, expiry_interval);
        Self { cap, interval, next: Arc::new(AtomicU16::new(1)), queues: LinkedHashMap::default() }
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
            let mut t = self.interval - (chrono::Local::now().timestamp_millis() - m.update_time);
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
    pub fn get(&self, packet_id: PacketId) -> Option<&InflightMessage> {
        self.queues.get(&packet_id)
    }

    #[inline]
    pub fn front(&self) -> Option<(&PacketId, &InflightMessage)> {
        self.queues.front().map(|(packet_id, m)| (packet_id, m))
    }

    #[inline]
    pub fn pop_front(&mut self) -> Option<InflightMessage> {
        self.queues.pop_front().map(|(_, m)| m)
    }

    #[inline]
    pub fn pop_front_timeout(&mut self) -> Option<InflightMessage> {
        if self.front_timeout() {
            self.pop_front()
        } else {
            None
        }
    }

    #[inline]
    pub fn push_back(&mut self, m: InflightMessage) {
        if let Some(packet_id) = m.publish.packet_id() {
            self.queues.insert(packet_id, m);
        } else {
            log::warn!("packet_id is None, inflight message: {:?}", m);
        }
    }

    #[inline]
    pub fn remove(&mut self, packet_id: &PacketId) -> Option<InflightMessage> {
        self.queues.remove(packet_id)
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
        for _ in 0..u16::max_value() {
            let packet_id = self.next.fetch_add(1, Ordering::SeqCst);
            if packet_id == 0 {
                continue;
            }
            if !self.queues.contains_key(&packet_id) {
                return Ok(packet_id);
            }
        }
        Err(MqttError::Msg("no packet_id available, should unreachable!()".into()))
        //unreachable!()
    }
}
