//! Configuration types for the Shared Subscription plugin.
//!
//! Defines [`PluginConfig`] used to select the subscriber selection
//! strategy for `$share/{group}/{topic}` subscriptions.

use serde::{Deserialize, Serialize};
use std::num::NonZeroUsize;

/// Top-level configuration for the Shared Subscription plugin.
///
/// Controls which strategy is used when choosing a subscriber from
/// a shared subscription group.
///
/// Supports the following strategies:
/// - `random` — Random subscriber selection (basic load balancing)
/// - `round_robin` — Sequential round-robin (default)
/// - `round_robin_per_group` — Per-node independent round-robin (large cluster)
/// - `sticky` — Fixed subscriber per publisher (stateful processing)
/// - `local` — Prefer publisher's node (reduce cross-node traffic)
/// - `hash_clientid` — Hash by publisher ClientId (per-device ordering)
/// - `hash_topic` — Hash by topic (topic sharding)
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    /// Shared subscription selection strategy.
    ///
    /// Valid values: `random`, `round_robin`, `round_robin_per_group`,
    /// `sticky`, `local`, `hash_clientid`, `hash_topic`.
    #[serde(default = "PluginConfig::strategy_default")]
    pub strategy: Strategy,
    /// Maximum number of sticky bindings in the LRU cache.
    ///
    /// When the cache reaches this limit, the least recently used binding
    /// is evicted to make room for a new one.
    #[serde(default = "PluginConfig::sticky_cache_size_default")]
    pub sticky_cache_size: NonZeroUsize,
}

impl PluginConfig {
    /// Default strategy: round_robin
    fn strategy_default() -> Strategy {
        Strategy::RoundRobin
    }

    /// Default sticky cache size: 100,000 entries
    fn sticky_cache_size_default() -> NonZeroUsize {
        NonZeroUsize::new(100_000).unwrap()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Strategy {
    Random,
    RoundRobin,
    RoundRobinPerGroup,
    Sticky,
    Local,
    #[serde(alias = "hash_clientid")]
    HashClientId,
    HashTopic,
}
