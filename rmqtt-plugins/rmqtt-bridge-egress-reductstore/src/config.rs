//! Configuration types for the ReductStore egress bridge plugin.
//!
//! Defines bridge connection parameters (server, API token, TLS verification)
//! and routing entries with ReductStore bucket/entry mapping and metadata
//! forwarding options.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::bridge::BridgeName;

use rmqtt::utils::deserialize_duration_option;

/// Top-level plugin configuration containing a list of bridge definitions.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default)]
    pub bridges: Vec<Bridge>,
}

/// A ReductStore egress bridge definition.
///
/// Specifies connection parameters (server, API token, SSL verification) and entries.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Bridge {
    #[serde(default)]
    pub(crate) enable: bool,
    #[serde(default)]
    pub(crate) name: BridgeName,
    pub(crate) servers: String,
    #[serde(default)]
    pub(crate) producer_name_prefix: Option<String>,

    #[serde(default)]
    pub(crate) api_token: Option<String>,
    #[serde(default)]
    pub(crate) verify_ssl: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_duration_option")]
    pub(crate) timeout: Option<Duration>,

    #[serde(default)]
    pub(crate) entries: Vec<Entry>,
}

/// A routing entry pairing a local topic filter with a remote ReductStore bucket/entry.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Entry {
    #[serde(default)]
    pub local: Local,

    #[serde(default)]
    pub remote: Remote,
}

/// Remote ReductStore configuration for a bridge entry.
///
/// Controls the target bucket/entry, quota, metadata forwarding, and skip levels.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Remote {
    pub bucket: String,
    pub entry: String,
    pub quota_size: Option<u64>,
    pub exist_ok: Option<bool>,
    #[serde(default)]
    pub forward_all_from: bool,
    #[serde(default)]
    pub forward_all_publish: bool,
    #[serde(default)]
    pub skip_levels: usize,
}

/// Local topic filter for a bridge entry.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Local {
    #[serde(default)]
    pub topic_filter: String,
}
