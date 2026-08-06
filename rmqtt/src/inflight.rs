//! MQTT Inflight Message Management System
//!
//! Provides reliable message delivery tracking for QoS 1 and 2 with:
//! - Outbound message retransmission
//! - Inbound message deduplication
//! - Configurable window sizes
//! - Automatic expiry handling
//!
//! ## Core Functionality
//! 1. ​**​Outbound Tracking (OutInflight)​**​:
//!    - Manages unacknowledged publishes (QoS 1/2)
//!    - Handles retransmission timeouts
//!    - Maintains packet ID sequencing
//!    - Supports configurable capacity limits
//!
//! 2. ​**​Inbound Tracking (InInflight)​**​:
//!    - Detects duplicate messages (QoS 2)
//!    - Enforces maximum window size
//!    - Provides packet ID lifecycle management
//!
//! ## Key Features
//! - Dual interval timing (retry/expiry)
//! - Event hooks for push/pop operations
//! - Atomic packet ID generation
//! - Time-based message expiry
//! - Statistics integration
//!
//! ## Implementation Details
//! - DequeMap for O(1) front access
//! - BTreeSet for efficient deduplication
//! - Atomic counters for thread safety
//! - Zero-cost status tracking
//!
//! Configuration Parameters:
//! - `cap`: Maximum concurrent outbound messages
//! - `retry_interval`: Retransmission delay (ms)
//! - `expiry_interval`: Message expiry timeout (ms)
//! - `max_inflight`: Maximum inbound window size
//!
//! Usage Patterns:
//! 1. Assign packet IDs via `next_id()`
//! 2. Track outbound messages with `push_back()`
//! 3. Process acknowledgements with `remove()`
//! 4. Handle timeouts via `pop_front_timeout()`
//! 5. Manage inbound flow with `add()`/`remove()`
//!
//! Note: Implements MQTT spec requirements for:
//! - Packet ID uniqueness (2.2.1)
//! - QoS flow control (4.6)
//! - Message expiry (3.3.2.3.2)

use std::collections::BTreeSet;
use std::num::NonZeroU16;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use itertools::Itertools;
use rust_box::dequemap::DequeBTreeMap as DequeMap;
use serde::{Deserialize, Serialize};

use crate::context::ServerContext;
use crate::net::MqttError;
use crate::queue::OnEventFn;
use crate::types::{From, PacketId, Publish, TimestampMillis};
use crate::types::{QoS, Reason};
use crate::utils::timestamp_millis;
use crate::Result;

type OutQueues = DequeMap<PacketId, OutInflightMessage>;

/// Tracks the acknowledgment status of an inflight message.
///
/// Represents the current state in the QoS flow:
/// - `UnAck`: Published but not yet acknowledged by the subscriber.
/// - `UnReceived`: PUBREL sent but PUBREC not yet received (QoS 2 only).
/// - `UnComplete`: QoS 2 handshake initiated but not completed.
#[derive(Debug, Eq, PartialEq, Clone, Copy, Serialize, Deserialize)]
pub enum MomentStatus {
    /// Published to subscriber, awaiting acknowledgment.
    UnAck,
    /// PUBREL sent, awaiting PUBREC (QoS 2 only).
    UnReceived,
    /// QoS 2 handshake in progress, awaiting completion.
    UnComplete,
}

/// An outbound inflight message with delivery tracking metadata.
///
/// Tracks a publish message that has been sent but not yet
/// acknowledged by the receiver. Used for QoS 1 and QoS 2
/// retransmission and deduplication logic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutInflightMessage {
    /// The publish message content.
    pub publish: Publish,
    /// The source of the message (client, broker, or bridge).
    pub from: From,
    /// Current delivery acknowledgment status.
    pub status: MomentStatus,
    /// Last status update timestamp (milliseconds since epoch).
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

/// Manages outbound inflight messages for QoS 1 and QoS 2 delivery.
///
/// Provides packet ID generation, retransmission tracking via
/// dual-interval timing (retry/expiry), and capacity-based flow control.
///
/// # Packet ID Management
///
/// Packet IDs are generated atomically via `AtomicU16` starting from 1,
/// wrapping around when necessary. The system probes up to `u16::MAX`
/// attempts before reporting exhaustion.
///
/// # Timeout Behavior
///
/// The effective check interval is `min(retry_interval, expiry_interval)`.
/// When a timeout fires, [`pop_front_timeout`](OutInflight::pop_front_timeout)
/// returns the expired message for retransmission or cleanup.
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
    /// Create a new outbound inflight tracker.
    ///
    /// # Arguments
    ///
    /// * `cap` — Maximum concurrent outbound messages.
    /// * `retry_interval` — Delay before retransmitting unacknowledged messages (ms).
    /// * `expiry_interval` — Time after which messages expire (ms).
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

    /// Register a callback invoked when a message is pushed to the inflight queue.
    #[inline]
    pub fn on_push<F>(mut self, f: F) -> Self
    where
        F: OnEventFn,
    {
        self.on_push_fn = Some(Arc::new(f));
        self
    }

    /// Register a callback invoked when a message is popped from the inflight queue.
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

    /// Compute the timeout duration until the next inflight message expires.
    ///
    /// Returns `None` if no messages are queued or if intervals are zero.
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
            log::debug!("get timeout t: {t}");
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
            log::warn!("packet_id is None, inflight message: {m:?}");
            None
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

    /// Advance the packet-id allocator so that ids already reserved by
    /// transferred inflight messages (kept under their old ids) are never
    /// re-issued to concurrently delivered messages. `next_id` only checks
    /// the current queue, so without this isolation a stored message
    /// delivered during session resume can be assigned the same id as a
    /// transferred message and `push_back` would silently overwrite it.
    #[inline]
    pub fn advance_next_id(&self, max_reserved: u16) {
        self.next.fetch_max(max_reserved.saturating_add(1), Ordering::SeqCst);
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

/// Tracks inbound inflight messages for QoS 2 deduplication.
///
///
/// Maintains a set of packet IDs received from the client to detect
/// duplicate PUBLISH packets (a requirement of the MQTT QoS 2 protocol).
///
/// # Flow Control
///
/// Enforces a `max_inflight` window to prevent the client from
/// overwhelming the broker with concurrent QoS 2 publishes.
///
/// Size limit for concurrent inbound messages on a single connection.
/// TODO: Make this configurable per listener/client.
pub struct InInflight {
    cached: BTreeSet<NonZeroU16>,
    #[allow(dead_code)]
    scx: ServerContext,
    max_inflight: u16,
}

impl Drop for InInflight {
    fn drop(&mut self) {
        #[cfg(feature = "stats")]
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
            #[cfg(feature = "stats")]
            self.scx.stats.in_inflights.inc();
            Ok(true)
        } else if matches!(qos, QoS::ExactlyOnce) {
            Err(MqttError::PacketIdInUse(pid).into())
        } else {
            Ok(false)
        }
    }

    /// Check whether a packet id is already tracked in the inbound inflight set.
    ///
    /// Used by the QoS 2 duplicate detection path ([MQTT-4.3.3-10]): when a
    /// PUBLISH with the same Packet Identifier arrives before the exchange is
    /// complete, the broker must answer PUBREC without delivering again.
    #[inline]
    pub(crate) fn exist(&self, pid: &NonZeroU16) -> bool {
        self.cached.contains(pid)
    }

    #[inline]
    pub(crate) fn remove(&mut self, pid: &NonZeroU16) -> bool {
        #[allow(clippy::needless_bool)]
        if self.cached.remove(pid) {
            #[cfg(feature = "stats")]
            self.scx.stats.in_inflights.dec();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use bytestring::ByteString;

    use crate::types::{CodecPublish, Id};

    fn make_publish(topic: &str, packet_id: u16) -> Publish {
        Publish {
            inner: Box::new(CodecPublish {
                dup: false,
                retain: false,
                qos: QoS::ExactlyOnce,
                topic: ByteString::from(topic),
                packet_id: NonZeroU16::new(packet_id),
                properties: None,
                payload: Bytes::from_static(b"payload"),
            }),
            target_clientid: None,
            delay_interval: None,
            create_time: None,
        }
    }

    fn make_from(client: &str) -> From {
        From::from_custom(Id::new(1, 1, None, None, ByteString::from(client), None))
    }

    fn make_msg(topic: &str, pid: u16, status: MomentStatus) -> OutInflightMessage {
        OutInflightMessage::new(status, make_from(topic), make_publish(topic, pid))
    }

    /// Root cause 1: the packet-id allocator restarts at 1 and is not
    /// isolated from the id space of transferred inflight messages
    /// (which keep their old ids 1..N and are registered later by
    /// `send_rerelease`). `next_id` only checks the current queue, so a
    /// concurrently delivered stored message can be assigned the same id.
    #[test]
    fn next_id_restarts_at_one_and_collides_with_transfer_ids() {
        let inflight = OutInflight::new(100, 0, 0);
        // The transferred messages (old ids 1..=3) are still queued behind
        // `SendRerelease`; the new session's queue is empty, so the allocator
        // hands out id 1 again — the exact overlap that causes the collision.
        assert_eq!(inflight.next_id().unwrap(), 1, "allocator is not isolated from the transferred id space");
    }

    /// Root cause 2: `push_back` silently overwrites an existing entry with
    /// the same packet-id (`HashMap::insert`), returning the overwritten id.
    /// A second registration therefore destroys the first message's QoS 2 state.
    #[test]
    fn push_back_same_packet_id_silently_overwrites() {
        let mut inflight = OutInflight::new(100, 0, 0);
        inflight.push_back(make_msg("stored/a", 1, MomentStatus::UnReceived));

        // Second registration with the same id (send_rerelease path).
        let old = inflight.push_back(make_msg("inflight/b", 1, MomentStatus::UnComplete));
        assert_eq!(old.map(|id| id.get()), Some(1), "push_back reported the overwrite");

        let cur = inflight.get(1).expect("packet 1 is still tracked");
        assert_eq!(cur.publish.inner.topic, ByteString::from_static("inflight/b"));
        assert_eq!(cur.status, MomentStatus::UnComplete);
    }

    /// Fix verification: after `advance_next_id`, new allocations skip the
    /// transferred id range (1..=N), so a concurrently delivered stored
    /// message can never collide with a transferred message's old id.
    #[test]
    fn advance_next_id_isolates_transfer_id_space() {
        let inflight = OutInflight::new(100, 0, 0);
        inflight.advance_next_id(5); // transferred messages keep ids 1..=5
        assert_eq!(inflight.next_id().unwrap(), 6, "allocator must skip the transferred id range");
    }

    /// End-to-end mechanism reproduction: during session resume, a stored
    /// message is first delivered (id 1), then `send_rerelease` registers a
    /// transferred message under the same old id 1 and overwrites it. The
    /// stored message's QoS 2 state is permanently lost — the PUBCOMP that
    /// arrives later completes the *other* message and the stored one has no
    /// record at all (no resend, no ack hook).
    #[test]
    fn resume_collision_loses_stored_message() {
        let mut inflight = OutInflight::new(100, 0, 0);

        // (1) stored message M1 delivered by send_storaged_messages: next_id=1
        let pid = NonZeroU16::new(inflight.next_id().unwrap()).unwrap();
        assert_eq!(pid.get(), 1);
        inflight.push_back(make_msg("stored/M1", pid.get(), MomentStatus::UnReceived));

        // (2) transferred message M2 (old id 1) registered by send_rerelease → overwrites M1
        let old = inflight.push_back(make_msg("inflight/M2", pid.get(), MomentStatus::UnComplete));
        assert_eq!(
            old.map(|id| id.get()),
            Some(1),
            "send_rerelease overwrote the stored message (bug reproduced)"
        );

        // (3) M1's PUBREC/PUBCOMP removes M2 — M1 is left with no tracking at all
        let removed = inflight.remove(&pid.get()).expect("something is tracked under id 1");
        assert_eq!(removed.publish.inner.topic, ByteString::from_static("inflight/M2"));
        assert!(
            inflight.get(pid.get()).is_none(),
            "M1's QoS 2 state is gone: no resend, no ack hook, message effectively lost"
        );
    }
}
