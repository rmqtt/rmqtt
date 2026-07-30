//! Configuration types for the Kafka ingress bridge plugin.
//!
//! Defines bridge connection parameters (servers, Kafka properties),
//! consumer group settings, partition and offset management, and local
//! MQTT topic mapping (including `${kafka.key}` pattern substitution).

use std::borrow::Cow;
use std::time::Duration;

use rdkafka::topic_partition_list::Offset;
use serde::de::{self, Deserializer};
use serde::{ser::Serializer, Deserialize, Serialize};

use rmqtt::{
    types::{HashMap, QoS, TopicName},
    utils::deserialize_duration,
};

use crate::bridge::BridgeName;

/// Sentinel value indicating no specific partition is assigned.
pub const PARTITION_UNASSIGNED: i32 = -1;

/// Header key used to store the Kafka message key in user properties.
pub const MESSAGE_KEY: &str = "_message_key";

/// Top-level plugin configuration containing a list of bridge definitions.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default)]
    pub bridges: Vec<Bridge>,
}

/// A Kafka ingress bridge definition.
///
/// Specifies connection parameters (servers, client ID prefix, Kafka properties),
/// routing entries, and a default expiry interval for ingested messages.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Bridge {
    #[serde(default)]
    pub enable: bool,
    #[serde(default)]
    pub name: BridgeName,
    pub servers: String,
    #[serde(default)]
    pub client_id_prefix: Option<String>,
    #[serde(default)]
    pub properties: HashMap<String, String>,

    #[serde(default)]
    pub entries: Vec<Entry>,

    #[serde(default = "Bridge::expiry_interval_default", deserialize_with = "deserialize_duration")]
    pub expiry_interval: Duration,
}

impl Bridge {
    #[inline]
    fn expiry_interval_default() -> Duration {
        Duration::from_secs(300)
    }
}

/// A routing entry pairing a remote Kafka topic/partition with a local MQTT topic.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Entry {
    #[serde(default)]
    pub remote: Remote,

    #[serde(default)]
    pub local: Local,
}

type HasPattern = bool; //${kafka.key}

/// Remote Kafka topic configuration for a bridge entry.
///
/// Controls the source topic, consumer group, partition range, and offset.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Remote {
    pub topic: String,
    pub group_id: String,
    #[serde(default = "Remote::start_partition_default")]
    pub start_partition: i32,
    #[serde(default = "Remote::stop_partition_default")]
    pub stop_partition: i32,
    #[serde(
        default,
        deserialize_with = "Remote::deserialize_offset",
        serialize_with = "Remote::serialize_offset"
    )]
    pub offset: Option<Offset>,
}

impl Remote {
    fn start_partition_default() -> i32 {
        PARTITION_UNASSIGNED
    }

    fn stop_partition_default() -> i32 {
        PARTITION_UNASSIGNED
    }

    /// Deserializes a Kafka offset from a string ("beginning", "end", "stored", "invalid", or a number).
    pub fn deserialize_offset<'de, D>(deserializer: D) -> std::result::Result<Option<Offset>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let topic = String::deserialize(deserializer)?;

        let offset = match topic.to_lowercase().as_str() {
            "beginning" => Offset::Beginning,
            "end" => Offset::End,
            "stored" => Offset::Stored,
            "invalid" => Offset::Invalid,
            n => {
                let n = n.parse::<i64>().map_err(de::Error::custom)?;
                if n >= 0 {
                    Offset::Offset(n)
                } else {
                    Offset::OffsetTail(-n)
                }
            }
        };
        Ok(Some(offset))
    }

    /// Serializes a Kafka offset to its string representation.
    #[inline]
    pub fn serialize_offset<S>(offset: &Option<Offset>, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match offset {
            Some(Offset::Beginning) => Some("beginning".into()),
            Some(Offset::End) => Some("end".into()),
            Some(Offset::Stored) => Some("stored".into()),
            Some(Offset::Invalid) => Some("invalid".into()),
            Some(Offset::Offset(n)) => Some((*n).to_string()), //Some(itoa::Buffer::new().format(*n)),
            Some(Offset::OffsetTail(n)) => Some((-*n).to_string()), //Some(itoa::Buffer::new().format(-*n)),
            None => None,
        }
        .serialize(serializer)
    }
}

/// Local MQTT topic configuration for an ingress entry.
///
/// Controls the target MQTT topic (with optional `${kafka.key}` pattern),
/// QoS, and retain flag.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Local {
    #[serde(default, deserialize_with = "Local::deserialize_qos")]
    pub qos: Option<QoS>,
    #[serde(default, deserialize_with = "Local::deserialize_topic")]
    pub topic: (String, HasPattern),
    #[serde(default)]
    pub retain: Option<bool>,
}

impl Local {
    /// Returns the local topic string (without pattern check).
    #[inline]
    pub fn topic(&self) -> &str {
        &self.topic.0
    }

    /// Returns whether the topic contains the `${kafka.key}` pattern.
    #[inline]
    pub fn topic_has_pattern(&self) -> bool {
        self.topic.1
    }

    /// Builds the actual MQTT topic, optionally substituting `${kafka.key}` with the Kafka message key.
    #[inline]
    pub fn make_topic(&self, kafka_key: Option<Cow<str>>) -> TopicName {
        if self.topic_has_pattern() {
            if let Some(kafka_key) = kafka_key {
                TopicName::from(self.topic().replace("${kafka.key}", kafka_key.as_ref()))
            } else {
                TopicName::from(self.topic().replace("${kafka.key}", ""))
            }
        } else {
            TopicName::from(self.topic())
        }
    }

    /// Determines the effective retain flag, preferring the configured value.
    #[inline]
    pub fn make_retain(&self, remote_retain: Option<bool>) -> bool {
        self.retain.unwrap_or(remote_retain.unwrap_or_default())
    }

    /// Determines the effective QoS, preferring the configured value (default: AtLeastOnce).
    #[inline]
    pub fn make_qos(&self, remote_qos: Option<QoS>) -> QoS {
        self.qos.unwrap_or(remote_qos.unwrap_or(QoS::AtLeastOnce))
    }

    /// Deserializes QoS from a u8 value (0, 1, or 2).
    #[inline]
    pub fn deserialize_qos<'de, D>(deserializer: D) -> std::result::Result<Option<QoS>, D::Error>
    where
        D: Deserializer<'de>,
    {
        match u8::deserialize(deserializer)? {
            0 => Ok(Some(QoS::AtMostOnce)),
            1 => Ok(Some(QoS::AtLeastOnce)),
            2 => Ok(Some(QoS::ExactlyOnce)),
            _ => Err(de::Error::custom("invalid value")),
        }
    }

    #[inline]
    pub fn deserialize_topic<'de, D>(deserializer: D) -> std::result::Result<(String, HasPattern), D::Error>
    where
        D: Deserializer<'de>,
    {
        let topic = String::deserialize(deserializer)?;
        let has_pattern = topic.contains("${kafka.key}");
        Ok((topic, has_pattern))
    }
}
