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
pub trait MessageManager: Sync + Send {
    #[inline]
    fn next_msg_id(&self) -> MsgID {
        0
    }

    ///Store messages
    ///
    ///_msg_id           - MsgID <br>
    ///_from             - From <br>
    ///_p                - Message <br>
    ///_expiry_interval  - Message expiration time <br>
    ///_sub_client_ids   - Indicate that certain subscribed clients have already forwarded the specified message.
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

    #[inline]
    async fn get(
        &self,
        _client_id: &str,
        _topic_filter: &str,
        _group: Option<&SharedGroup>,
    ) -> Result<Vec<(MsgID, From, Publish)>> {
        Ok(Vec::new())
    }

    ///Indicate whether merging data from various nodes is needed during the 'get' operation.
    #[inline]
    fn should_merge_on_get(&self) -> bool {
        false
    }

    #[inline]
    async fn count(&self) -> isize {
        -1
    }

    #[inline]
    async fn max(&self) -> isize {
        -1
    }

    #[inline]
    fn enable(&self) -> bool {
        false
    }
}

pub struct DefaultMessageManager {}

impl Default for DefaultMessageManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultMessageManager {
    #[inline]
    pub fn new() -> DefaultMessageManager {
        Self {}
    }
}

impl MessageManager for DefaultMessageManager {}
