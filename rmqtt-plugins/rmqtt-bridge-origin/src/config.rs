//! Configuration types for the bridge origin plugin.
//!
//! Defines markers used to identify ingress/egress bridge connections
//! in session metadata, and the attribute key for storing origin info.

use bytestring::ByteString;
use serde::{Deserialize, Serialize};

use rmqtt::Result;

/// Plugin configuration with ingress/egress markers and attribute key.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default = "default_ingress_marker")]
    pub ingress_marker: String,

    #[serde(default = "default_egress_marker")]
    pub egress_marker: String,

    /// Key used to store BridgeOrigin in session.extra_attrs.
    #[serde(default = "default_attr_key")]
    pub attr_key: ByteString,
}

impl PluginConfig {
    /// Creates a new `PluginConfig` with the given marker and key values.
    #[allow(dead_code)]
    pub fn new(ingress_marker: String, egress_marker: String, attr_key: ByteString) -> Self {
        Self { ingress_marker, egress_marker, attr_key }
    }

    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
}

fn default_ingress_marker() -> String {
    ":ingress:".into()
}

fn default_egress_marker() -> String {
    ":egress:".into()
}

fn default_attr_key() -> ByteString {
    "bridge_origin".into()
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            ingress_marker: default_ingress_marker(),
            egress_marker: default_egress_marker(),
            attr_key: default_attr_key(),
        }
    }
}
