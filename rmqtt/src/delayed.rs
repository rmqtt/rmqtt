//! Delayed Message Publishing System
//!
//! Defines the [`DelayedSender`] extension trait and the placeholder
//! [`DefaultDelayedSender`] used before the `rmqtt-delayed` plugin starts.
//!
//! ## Core Functionality
//! 1. ​**​Topic Parsing​**​:
//!    - Recognizes `$delayed/<interval>/<topic>` format
//!    - Extracts delay intervals from topic strings
//!    - Validates delay parameter formatting
//!
//! 2. ​**​Message Scheduling​**​ (provided by the `rmqtt-delayed` plugin):
//!    - Time-ordered priority queue (BinaryHeap)
//!    - Periodic expiration checks
//!    - Automatic forwarding of expired messages
//!
//! ## Implementation Details
//! - The placeholder implementation is a no-op: the feature reports disabled,
//!   `parse` returns the publish unchanged and `delay_publish` passes the
//!   message through, so `$delayed/...` topics are treated as literal topics.
//! - Metadata types ([`DelayedPublishInfo`], [`DelayedPublishDetail`]) are
//!   shared with the HTTP API and stay in `crate::types`.

use std::time::Duration;

use async_trait::async_trait;

use crate::types::{DelayedPublishDetail, DelayedPublishInfo, From, Publish, TimestampMillis};
use crate::Result;

/// Trait for delayed message publishing using the `$delayed/<interval>/<topic>` topic scheme.
///
/// Implementors parse delay parameters from topic strings, schedule messages
/// for future delivery, and provide access to pending messages (metadata and,
/// through [`Self::find`], the full payload).
///
/// The real implementation (`MemDelayedSender`) lives in the `rmqtt-delayed`
/// plugin and is injected through `extends.delayed_sender_mut()` on plugin
/// start. Before that, [`DefaultDelayedSender`] is in place: the feature
/// reports disabled and all operations are no-ops.
#[async_trait]
pub trait DelayedSender: Sync + Send {
    /// Whether delayed message publishing is enabled. Defaults to `false`
    /// (the default implementation is a no-op). For `MemDelayedSender` this
    /// is `true` once the `rmqtt-delayed` plugin has been started.
    #[inline]
    fn enable(&self) -> bool {
        false
    }

    ///Parse the topic and extract the delayed sending parameters.
    fn parse(&self, publish: Publish) -> Result<Publish>;

    ///Schedule the message for delayed delivery.
    ///
    /// Returns `Ok(None)` when the sender handled the message: either it was
    /// scheduled for future delivery, or it was refused (e.g. over capacity)
    /// and dropped by the sender — in the latter case the sender fires the
    /// `message_dropped` hook itself (with `Reason::DelayedPublishRefused`).
    /// Returns `Ok(Some((from, publish)))` when the message was not scheduled
    /// and the caller should forward it immediately as a regular message.
    async fn delay_publish(
        &self,
        from: From,
        publish: Publish,
        message_storage_available: bool,
        message_expiry_interval: Option<Duration>,
    ) -> Result<Option<(From, Publish)>>;

    ///Delayed message count
    async fn len(&self) -> usize;

    /// List pending delayed messages matching an optional MQTT topic filter
    /// (`#`/`+` wildcards; matched against the target topic with the
    /// `$delayed/<interval>/` prefix already stripped). Returns at most `max`
    /// entries sorted by trigger time (oldest first). Payload content is never
    /// included. The default impl returns an empty vec (feature disabled).
    #[inline]
    async fn list(&self, topic_filter: Option<&str>, max: usize) -> Vec<DelayedPublishInfo> {
        let _ = (topic_filter, max);
        Vec::new()
    }

    /// Find one pending delayed publish by its composite key
    /// `(target topic, trigger timestamp, optional publisher client id)` and
    /// return it with the full payload content. Returns the first match when
    /// several entries share the same key (no stable id in the heap).
    /// The default impl returns `None` (feature disabled).
    #[inline]
    async fn find(
        &self,
        _topic: &str,
        _expired_time: TimestampMillis,
        _client_id: Option<&str>,
    ) -> Option<DelayedPublishDetail> {
        None
    }

    #[inline]
    async fn is_empty(&self) -> bool {
        self.len().await == 0
    }
}

/// Placeholder implementation used before the `rmqtt-delayed` plugin starts.
///
/// All operations are no-ops: the feature reports disabled (`enable()` is
/// `false`, so the session skips `$delayed` parsing entirely) and pending
/// message queries return empty results. On plugin start the plugin swaps this
/// placeholder for its own `MemDelayedSender` through
/// `extends.delayed_sender_mut()`.
#[derive(Clone, Default)]
pub struct DefaultDelayedSender;

impl DefaultDelayedSender {
    /// Creates the placeholder sender.
    #[inline]
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl DelayedSender for DefaultDelayedSender {
    #[inline]
    fn parse(&self, publish: Publish) -> Result<Publish> {
        Ok(publish)
    }

    #[inline]
    async fn delay_publish(
        &self,
        from: From,
        publish: Publish,
        _message_storage_available: bool,
        _message_expiry_interval: Option<Duration>,
    ) -> Result<Option<(From, Publish)>> {
        // Unreachable through the session path (the feature reports disabled),
        // pass the message through unchanged for safety.
        Ok(Some((from, publish)))
    }

    #[inline]
    async fn len(&self) -> usize {
        0
    }
}
