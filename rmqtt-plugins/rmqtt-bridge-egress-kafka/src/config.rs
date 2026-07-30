//! Configuration types for the Kafka egress bridge plugin.
//!
//! Defines the plugin configuration structure, bridge definitions with
//! local topic filters, remote Kafka topic mapping, and per-entry settings
//! such as partitioning, queue timeout, and topic skip levels.

use std::time::Duration;

use bytestring::ByteString;
use serde::{de::Deserializer, Deserialize, Serialize};

use rmqtt::{types::HashMap, utils::deserialize_duration};

use crate::bridge::BridgeName;

/// Top-level plugin configuration.
///
/// Contains task queue capacity/limit settings and a list of bridge definitions.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default = "PluginConfig::task_queue_capacity_default")]
    pub task_queue_capacity: usize,
    #[serde(default = "PluginConfig::task_concurrency_limit_default")]
    pub task_concurrency_limit: usize,
    #[serde(default)]
    pub bridges: Vec<Bridge>,
}

impl PluginConfig {
    fn task_queue_capacity_default() -> usize {
        300_000
    }
    fn task_concurrency_limit_default() -> usize {
        128
    }
}

/// A Kafka egress bridge definition.
///
/// Specifies connection parameters (servers, client ID prefix, Kafka properties),
/// concurrent client limits, and routing entries mapping local topic filters
/// to remote Kafka topics.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Bridge {
    #[serde(default)]
    pub enable: bool,
    #[serde(default)]
    pub name: BridgeName,
    pub servers: String,
    #[serde(default)]
    pub client_id_prefix: Option<String>,
    #[serde(default = "Bridge::concurrent_client_limit_default")]
    pub concurrent_client_limit: usize,

    #[serde(default)]
    pub properties: HashMap<String, String>,

    #[serde(default)]
    pub entries: Vec<Entry>,
}

impl Bridge {
    fn concurrent_client_limit_default() -> usize {
        1
    }
}

/// A single routing entry pairing a local topic filter with a remote Kafka topic.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Entry {
    #[serde(default)]
    pub local: Local,

    #[serde(default)]
    pub remote: Remote,
}

type HasPattern = bool; //${local.topic}

/// Remote Kafka topic configuration for a bridge entry.
///
/// Controls the target topic (with optional `${local.topic}` pattern),
/// queue timeout, partition selection, and topic level skipping.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Remote {
    #[serde(default, deserialize_with = "Remote::deserialize_topic")]
    pub topic: (String, HasPattern),
    #[serde(default = "Remote::queue_timeout_default", deserialize_with = "deserialize_duration")]
    pub queue_timeout: Duration,
    #[serde(default)]
    pub partition: Option<i32>,
    #[serde(default)]
    pub skip_levels: usize,
}

impl Remote {
    /// Returns the remote topic string (without pattern check).
    #[inline]
    pub fn topic(&self) -> &str {
        &self.topic.0
    }

    /// Returns whether the topic contains the `${local.topic}` pattern.
    #[inline]
    pub fn topic_has_pattern(&self) -> bool {
        self.topic.1
    }

    /// Builds the actual topic string, optionally replacing `${local.topic}`.
    #[inline]
    pub fn make_topic(&self, local_topic: &str) -> ByteString {
        if self.topic_has_pattern() {
            ByteString::from(self.topic().replace("${local.topic}", local_topic.replace('/', "-").as_str()))
        } else {
            ByteString::from(self.topic())
        }
    }

    fn queue_timeout_default() -> Duration {
        Duration::ZERO
    }

    /// Deserializes a topic string and detects whether it contains the `${local.topic}` pattern.
    ///
    /// Replaces `/` with `-` in the result.
    pub fn deserialize_topic<'de, D>(deserializer: D) -> std::result::Result<(String, HasPattern), D::Error>
    where
        D: Deserializer<'de>,
    {
        let topic = String::deserialize(deserializer)?;
        let has_pattern = topic.contains("${local.topic}");
        Ok((topic.replace('/', "-"), has_pattern))
    }
}

/// Local topic filter for a bridge entry.
///
/// Defines which MQTT topics from the local broker are matched
/// and forwarded to the remote Kafka topic.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Local {
    #[serde(default)]
    pub topic_filter: String,
}
