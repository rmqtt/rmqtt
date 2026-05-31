//! MQTT Message Storage and Retrieval System
//!
//! Provides persistent message storage capabilities with:
//! - Message deduplication tracking
//! - Expiry-based cleanup
//! - Distributed storage coordination
//! - Client-specific message retrieval
//!
//! ## Core Functionality
//! 1. ​**​Message Storage​**​:
//!    - Tracks message origin and metadata
//!    - Records subscriber delivery status
//!    - Enforces message expiry policies
//!
//! 2. ​**​Message Retrieval​**​:
//!    - Client-specific message queries
//!    - Shared subscription support
//!    - Optional distributed merge operations
//!
//! 3. ​**​System Monitoring​**​:
//!    - Storage capacity tracking
//!    - Message count statistics
//!    - Operational status reporting
//!
//! ## Key Features
//! - Pluggable storage backend (default is no-op)
//! - Message ID generation hook
//! - Subscriber state tracking
//! - Cluster-aware storage coordination
//! - Lightweight default implementation
//!
//! ## Implementation Notes
//! - All methods have no-op default implementations
//! - Designed for easy extension with concrete storage backends
//! - Async-compatible interface
//! - Zero-cost when disabled
//!
//! Typical Usage:
//! 1. Implement `store()` for message persistence
//! 2. Override `get()` for client-specific retrieval
//! 3. Implement `should_merge_on_get()` for cluster coordination
//! 4. Provide capacity monitoring via `count()`/`max()`
//!
//! Note: The default implementation performs no actual storage,
//! making it suitable for brokers that don't require message persistence.
//!
use std::time::Duration;

use async_trait::async_trait;

use crate::types::{ClientId, From, MsgID, Publish, SharedGroup, TopicFilter};
use crate::Result;

#[async_trait]
/// Defines the message storage and retrieval contract for the broker.
///
/// Provides operations for storing, retrieving, and managing MQTT messages
/// with support for message deduplication, expiry-based cleanup, and
/// cluster-aware storage coordination. All methods have no-op default
/// implementations, making the trait easy to implement.
pub trait MessageManager: Sync + Send {
    /// Generate the next message ID for deduplication tracking.
    #[inline]
    fn next_msg_id(&self) -> MsgID {
        0
    }

    /// Persist a message for potential redelivery to reconnecting clients.
    ///
    /// # Arguments
    /// * `msg_id` - Unique message identifier for deduplication.
    /// * `from` - Origin information identifying the publishing source.
    /// * `p` - The MQTT publish packet content.
    /// * `expiry_interval` - Duration after which the message expires.
    /// * `sub_client_ids` - Optional set of subscribers that have already received this message.
    #[inline]
    async fn store(
        &self,
        _msg_id: MsgID,
        _from: From,
        _p: Publish,
        _expiry_interval: Duration,
        _sub_client_ids: Option<Vec<(ClientId, Option<(TopicFilter, SharedGroup)>)>>,
    ) -> Result<()> {
        Ok(())
    }

    /// Retrieve stored messages for a specific client or shared subscription.
    ///
    /// # Arguments
    /// * `client_id` - The target client identifier.
    /// * `topic_filter` - Topic filter for matching messages.
    /// * `group` - Optional shared subscription group name.
    #[inline]
    async fn get(
        &self,
        _client_id: &str,
        _topic_filter: &str,
        _group: Option<&SharedGroup>,
    ) -> Result<Vec<(MsgID, From, Publish)>> {
        Ok(Vec::new())
    }

    /// Indicate whether merging data from various cluster nodes is needed during retrieval.
    #[inline]
    fn should_merge_on_get(&self) -> bool {
        false
    }

    /// Return the current number of stored messages, or `-1` if unknown.
    #[inline]
    async fn count(&self) -> isize {
        -1
    }

    /// Return the maximum storage capacity, or `-1` if unlimited.
    #[inline]
    async fn max(&self) -> isize {
        -1
    }

    /// Indicate whether message storage is enabled.
    #[inline]
    fn enable(&self) -> bool {
        false
    }
}

/// A no-op default implementation of [`MessageManager`].
///
/// This implementation performs no actual storage, making it suitable
/// for brokers that do not require message persistence.
pub struct DefaultMessageManager {}

impl Default for DefaultMessageManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultMessageManager {
    /// Create a new `DefaultMessageManager` instance.
    #[inline]
    pub fn new() -> DefaultMessageManager {
        Self {}
    }
}

impl MessageManager for DefaultMessageManager {}
