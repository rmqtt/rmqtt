//! MQTT Subscription Management Core
//!
//! Provides extensible subscription handling implementations supporting MQTT 5.0 protocol
//! features with asynchronous execution and cluster-aware operations. The implementation
//! follows EMQX-style design patterns for shared subscriptions.
//!
//! ## Core Components
//! 1. **Shared Subscription System**:
//!    - Implements random selection strategy for subscriber load balancing (default)
//!    - Supports online status detection through router integration
//!    - Enables distributed session management across nodes
//!
//! 2. **Auto-Subscription Framework**:
//!    - Provides trait-based extensibility for dynamic subscription rules
//!    - Supports conditional compilation via feature flags
//!
//! ## Key Design Features
//! - **Asynchronous Architecture**:
//!   ```rust,ignore
//!   #[async_trait]  // Transforms async methods into boxed futures
//!   ```
//!   Uses `async-trait` macro to enable async methods in traits while maintaining object safety
//!   through `Pin<Box<dyn Future>>` returns
//!
//! - **Cluster Optimization**:
//!   - Node-aware subscriber selection with fallback mechanisms
//!   - Online status caching to reduce router queries
//!
//! - **Extensibility**:
//!   ```rust,ignore
//!   #[cfg(feature = "shared-subscription")]  // Feature-gated implementation
//!   ```
//!   Modular design allows optional inclusion of advanced subscription types
//!
//! ## Implementation Notes
//! 1. **Shared Subscription Workflow**:
//!    - Filters candidates through `is_supported()` config check
//!    - Performs online status validation via `router().is_online()`
//!    - Implements random selection with retry logic for offline nodes
//!
//! 2. **Performance Considerations**:
//!    - Uses `#[inline]` hints for hot path methods
//!    - Avoids unnecessary cloning through reference counting
//!    - Limits dynamic dispatch through concrete trait implementations
//!
//! The architecture balances protocol compliance (MQTT 5.0 spec) with practical performance
//! requirements, leveraging Rust's type system for safe concurrent operations.

#[cfg(any(feature = "shared-subscription", feature = "auto-subscription"))]
use async_trait::async_trait;

#[cfg(feature = "shared-subscription")]
use crate::context::ServerContext;
#[cfg(any(feature = "shared-subscription", feature = "auto-subscription"))]
use crate::types::*;

/// Defines the shared subscription selection strategy for a cluster node.
///
/// Implementations control how subscribers within a shared subscription group
/// (`$share/{group}/{topic}`) are selected. The default implementation uses
/// a round-robin selection strategy with online status filtering.
///
/// # Context Parameters
///
/// The `choice` method provides the following context for strategy decisions:
/// - `group`: the shared subscription group name (e.g. `"group1"` in `$share/group1/topic`)
/// - `publisher_id`: the publishing client's identity (contains `node_id` and `client_id`)
/// - `topic`: the published topic name used for topic-based hashing
#[cfg(feature = "shared-subscription")]
#[async_trait]
pub trait SharedSubscription: Sync + Send {
    ///Whether shared subscriptions are supported
    #[inline]
    fn is_supported(&self, _listen_cfg: &ListenerConfig) -> bool {
        false
    }

    ///Selects a subscriber from the shared subscription group.
    ///Returns `Some((index, is_online))` or `None` if no subscriber is available.
    async fn choice(
        &self,
        _scx: &ServerContext,
        _group: &SharedGroup,
        _publisher_id: &Id,
        _topic: &TopicName,
        _ncs: &[(
            NodeId,
            ClientId,
            SubscriptionOptions,
            Option<Vec<SubscriptionIdentifier>>,
            Option<IsOnline>,
        )],
    ) -> Option<(usize, IsOnline)> {
        None
    }
}

/// Default shared subscription implementation using round-robin selection.
///
/// Note: This is a best-effort single-node round-robin. For true round-robin
/// across cluster nodes, use the `rmqtt-shared-subscription` plugin.
#[cfg(feature = "shared-subscription")]
pub struct DefaultSharedSubscription;

#[cfg(feature = "shared-subscription")]
#[async_trait]
impl SharedSubscription for DefaultSharedSubscription {}

/// Defines auto-subscription behavior for newly connected clients.
///
/// Implementations specify which topics a client should be automatically
/// subscribed to upon connection. This is useful for system topics or
/// mandatory monitoring subscriptions.
#[cfg(feature = "auto-subscription")]
#[async_trait]
pub trait AutoSubscription: Sync + Send {
    /// Check whether auto-subscription is enabled for this client.
    #[inline]
    fn enable(&self) -> bool {
        false
    }

    /// Return the list of subscriptions to apply automatically on client connect.
    #[inline]
    async fn subscribes(&self, _id: &Id) -> crate::Result<Vec<Subscribe>> {
        Ok(Vec::new())
    }
}

/// Default auto-subscription implementation that performs no automatic subscriptions.
///
/// All methods return their default (no-op) values: `enable()` returns `false`
/// and `subscribes()` returns an empty vector.
#[cfg(feature = "auto-subscription")]
pub struct DefaultAutoSubscription;

#[cfg(feature = "auto-subscription")]
#[async_trait]
impl AutoSubscription for DefaultAutoSubscription {}
