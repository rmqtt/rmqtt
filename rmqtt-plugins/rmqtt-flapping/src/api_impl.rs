//! The plugin's answer to [`rmqtt::flapping::Flapping`].
//!
//! Installed into `scx.extends.flapping` when the plugin initialises, so that
//! the management API and the periodic stats snapshot can read the bans this
//! node enforces without knowing that the plugin exists. The class of problems
//! this is meant to solve is described in `designs/flapping-plugin.md` §3.8:
//! `rmqtt-http-api` has no way to let a plugin register routes, and the
//! extension slot is the one pattern the repository already uses for exactly
//! this (`retain`, `msgstore`, `delayed`, `auto-subscription`).

use std::sync::Arc;

use async_trait::async_trait;

use rmqtt::flapping::{BanInfo, BanQuery, Dimension, Flapping};

use crate::gate::flapping::FlappingGate;

/// Read-only view of a [`FlappingGate`], plus the one write an operator has:
/// lifting a ban by hand.
pub(crate) struct FlappingApi {
    gate: Arc<FlappingGate>,
}

impl FlappingApi {
    pub(crate) fn new(gate: Arc<FlappingGate>) -> Self {
        Self { gate }
    }
}

#[async_trait]
impl Flapping for FlappingApi {
    #[inline]
    fn available(&self) -> bool {
        self.gate.enabled()
    }

    #[inline]
    async fn banned(&self, query: BanQuery) -> Vec<BanInfo> {
        self.gate.banned(&query, crate::now_ms())
    }

    #[inline]
    async fn banned_count(&self) -> usize {
        self.gate.banned_count(crate::now_ms())
    }

    #[inline]
    async fn windows_count(&self) -> usize {
        self.gate.windows_count()
    }

    #[inline]
    async fn unban(&self, dimension: Dimension, key: &str) -> bool {
        self.gate.unban(dimension, key).await
    }
}
