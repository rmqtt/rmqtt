//! Configuration for the session storage plugin.
//!
//! Defines [`PluginConfig`] wrapping an `rmqtt_storage::Config` for
//! persisting session state.

use serde::{Deserialize, Serialize};

use rmqtt_storage::Config;

/// Top-level configuration for the session storage plugin.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    pub storage: Config,
}

impl PluginConfig {
    /// Serializes the configuration to a JSON value.
    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!(self)
    }
}
