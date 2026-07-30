//! Configuration types for the NATS egress bridge plugin.
//!
//! Defines bridge connection parameters (NATS server addresses, TLS,
//! authentication), per-entry routing with topic filters and NATS subject
//! mapping, and options for forwarding MQTT metadata.

use std::path::PathBuf;
use std::time::Duration;

use serde::de::Deserializer;
use serde::{Deserialize, Serialize};

use crate::bridge::BridgeName;

use rmqtt::utils::deserialize_duration_option;

/// Top-level plugin configuration containing a list of bridge definitions.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default)]
    pub bridges: Vec<Bridge>,
}

/// A NATS egress bridge definition.
///
/// Specifies connection parameters (server addresses, TLS, authentication),
/// producer naming, and routing entries.
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
    pub(crate) no_echo: Option<bool>,
    #[serde(default)]
    pub(crate) max_reconnects: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_duration_option")]
    pub(crate) connection_timeout: Option<Duration>,
    #[serde(default)]
    pub(crate) tls_required: Option<bool>,
    #[serde(default)]
    pub(crate) tls_first: Option<bool>,
    #[serde(default, deserialize_with = "Bridge::deserialize_pathbuf")]
    pub(crate) root_certificates: Option<PathBuf>,
    #[serde(default, deserialize_with = "Bridge::deserialize_pathbuf")]
    pub(crate) client_cert: Option<PathBuf>,
    #[serde(default, deserialize_with = "Bridge::deserialize_pathbuf")]
    pub(crate) client_key: Option<PathBuf>,
    #[serde(default, deserialize_with = "deserialize_duration_option")]
    pub(crate) ping_interval: Option<Duration>,
    #[serde(default)]
    pub(crate) sender_capacity: Option<usize>,
    #[serde(default, deserialize_with = "deserialize_duration_option")]
    pub(crate) request_timeout: Option<Duration>,
    #[serde(default)]
    pub(crate) retry_on_initial_connect: bool,
    #[serde(default)]
    pub(crate) ignore_discovered_servers: bool,
    #[serde(default)]
    pub(crate) retain_servers_order: bool,
    #[serde(default)]
    pub(crate) read_buffer_capacity: Option<u16>,
    #[serde(default)]
    pub(crate) auth: Auth,

    #[serde(default)]
    pub(crate) entries: Vec<Entry>,
}

impl Bridge {
    /// Deserializes a file path from a string, returning `None` for empty strings.
    #[inline]
    pub fn deserialize_pathbuf<'de, D>(deserializer: D) -> std::result::Result<Option<PathBuf>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let name = String::deserialize(deserializer)?;
        if name.is_empty() {
            Ok(None)
        } else {
            Ok(Some(PathBuf::from(name)))
        }
    }
}

/// NATS authentication configuration (JWT, NKey, user/password, token).
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Auth {
    pub(crate) jwt: Option<String>,
    pub(crate) jwt_seed: Option<String>,
    pub(crate) nkey: Option<String>,
    pub(crate) username: Option<String>,
    pub(crate) password: Option<String>,
    pub(crate) token: Option<String>,
}

/// A routing entry pairing a local topic filter with a remote NATS subject.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Entry {
    #[serde(default)]
    pub local: Local,

    #[serde(default)]
    pub remote: Remote,
}

/// Remote NATS subject configuration for a bridge entry.
///
/// Controls the target subject, metadata forwarding options, and
/// topic level skipping.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Remote {
    pub topic: String,
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
