//! Configuration for the delayed publish plugin.
//!
//! Defines [`PluginConfig`] for the `rmqtt-delayed.toml` keys
//! `publish_max` and `publish_immediate`.

use serde::{Deserialize, Serialize};

use rmqtt::Result;

/// Top-level configuration for the delayed publish plugin (loaded from
/// `rmqtt-delayed.toml` in the plugin config directory, hot-reloadable on
/// plugin reload).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    /// Maximum number of pending delayed messages per node.
    #[serde(default = "PluginConfig::publish_max_default")]
    pub publish_max: usize,
    /// Behavior when the limit is reached: `true` - forward immediately as a
    /// regular message; `false` - drop the message (fires the
    /// `message_dropped` hook with `Reason::DelayedPublishRefused`).
    #[serde(default = "PluginConfig::publish_immediate_default")]
    pub publish_immediate: bool,
}

impl PluginConfig {
    fn publish_max_default() -> usize {
        100_000
    }

    fn publish_immediate_default() -> bool {
        true
    }

    /// Serializes the configuration to a JSON value.
    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            publish_max: Self::publish_max_default(),
            publish_immediate: Self::publish_immediate_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PluginConfig;

    #[test]
    fn config_defaults_match_global_history() {
        let cfg: PluginConfig = toml::from_str("").expect("empty config falls back to defaults");
        assert_eq!(cfg.publish_max, 100_000);
        assert!(cfg.publish_immediate);
    }

    #[test]
    fn config_parses_plugin_keys() {
        let cfg: PluginConfig =
            toml::from_str("publish_max = 10\npublish_immediate = false").expect("parse ok");
        assert_eq!(cfg.publish_max, 10);
        assert!(!cfg.publish_immediate);
    }

    #[test]
    fn config_ignores_unknown_keys() {
        // Legacy global keys (mqtt.delayed_publish_*) must not break loading.
        let cfg: PluginConfig = toml::from_str("delayed_publish_max = 1\nunknown = true").expect("parse ok");
        assert_eq!(cfg.publish_max, 100_000);
        assert!(cfg.publish_immediate);
    }
}
