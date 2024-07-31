use std::borrow::Cow;
use std::time::Duration;

use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};

use rdkafka::topic_partition_list::Offset;

use rmqtt::{settings::deserialize_duration, HashMap, QoS, Result, TopicName};

use crate::bridge::BridgeName;

pub const PARTITION_UNASSIGNED: i32 = -1;
pub const MESSAGE_KEY: &str = "_message_key";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default)]
    pub bridges: Vec<Bridge>,
}

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

    #[serde(default = "Bridge::retain_available_default")]
    pub retain_available: bool,

    #[serde(default = "Bridge::storage_available_default")]
    pub storage_available: bool,

    #[serde(default = "Bridge::expiry_interval_default", deserialize_with = "deserialize_duration")]
    pub expiry_interval: Duration,
}

impl Bridge {
    #[inline]
    fn retain_available_default() -> bool {
        false
    }

    #[inline]
    fn storage_available_default() -> bool {
        false
    }

    #[inline]
    fn expiry_interval_default() -> Duration {
        Duration::from_secs(300)
    }
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Entry {
    #[serde(default)]
    pub remote: Remote,

    #[serde(default)]
    pub local: Local,
}

type HasPattern = bool; //${kafka.key}

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

    pub fn deserialize_offset<'de, D>(deserializer: D) -> Result<Option<Offset>, D::Error>
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

    #[inline]
    pub fn serialize_offset<S>(offset: &Option<Offset>, serializer: S) -> Result<S::Ok, S::Error>
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
    #[inline]
    pub fn topic(&self) -> &str {
        &self.topic.0
    }

    #[inline]
    pub fn topic_has_pattern(&self) -> bool {
        self.topic.1
    }

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

    #[inline]
    pub fn make_retain(&self, remote_retain: Option<bool>) -> bool {
        self.retain.unwrap_or(remote_retain.unwrap_or_default())
    }

    #[inline]
    pub fn make_qos(&self, remote_qos: Option<QoS>) -> QoS {
        self.qos.unwrap_or(remote_qos.unwrap_or(QoS::AtLeastOnce))
    }

    #[inline]
    pub fn deserialize_qos<'de, D>(deserializer: D) -> Result<Option<QoS>, D::Error>
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
    pub fn deserialize_topic<'de, D>(deserializer: D) -> Result<(String, HasPattern), D::Error>
    where
        D: Deserializer<'de>,
    {
        let topic = String::deserialize(deserializer)?;
        let has_pattern = topic.contains("${kafka.key}");
        Ok((topic, has_pattern))
    }
}
