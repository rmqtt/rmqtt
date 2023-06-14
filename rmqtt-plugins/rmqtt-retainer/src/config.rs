use std::time::Duration;

use serde::de::{Deserialize, Deserializer};

use rmqtt::grpc::MessageType;
use rmqtt::serde_json;
use rmqtt::settings::{deserialize_duration, Bytesize};
use rmqtt::Result;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default = "PluginConfig::message_type_default")]
    pub message_type: MessageType,

    // ram: only stored in memory;
    #[serde(default = "PluginConfig::storage_type_default")]
    pub storage_type: StorageType, // = "ram",

    // The maximum number of retained messages, where 0 indicates no limit. After the number of reserved messages exceeds
    // the maximum limit, existing reserved messages can be replaced, but reserved messages cannot be stored for new topics.
    #[serde(default = "PluginConfig::max_retained_messages_default")]
    pub max_retained_messages: isize, // = 0

    // The maximum Payload value for retaining messages. After the Payload size exceeds the maximum value, the RMQTT
    // message server will process the received reserved message as a regular message.
    #[serde(default = "PluginConfig::max_payload_size_default")]
    pub max_payload_size: Bytesize, // = "1MB"

    // The expiration time of the retention message, where 0 means it will never expire. If the message expiration interval is set in
    // the PUBLISH message, the message expiration interval in the PUBLISH message shall prevail.
    #[serde(default = "PluginConfig::expiry_interval_default", deserialize_with = "deserialize_duration")]
    pub expiry_interval: Duration, // = "10m"
}

impl PluginConfig {
    fn message_type_default() -> MessageType {
        69
    }

    fn storage_type_default() -> StorageType {
        StorageType::Ram
    }

    fn max_retained_messages_default() -> isize {
        0
    }

    fn max_payload_size_default() -> Bytesize {
        Bytesize::from(1024 * 1024)
    }

    fn expiry_interval_default() -> Duration {
        Duration::ZERO
    }

    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum StorageType {
    //ram: only stored in memory;
    Ram,
    //disc: stored in memory and hard drive;
    Disc,
    //disc_only: Only stored on the hard drive.
    DiscOnly,
}

impl<'de> Deserialize<'de> for StorageType {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let t = match (String::deserialize(deserializer)?).to_ascii_lowercase().as_str() {
            "ram" => StorageType::Ram,
            "disc" => StorageType::Disc,
            "disc_only" => StorageType::DiscOnly,
            _ => StorageType::Ram,
        };
        Ok(t)
    }
}
