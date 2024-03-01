use std::time::Duration;

use serde::de::{self, Deserialize, Deserializer};

use rmqtt::serde_json;
use rmqtt::settings::{deserialize_duration, Bytesize};
use rmqtt::Result;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default)]
    #[serde(deserialize_with = "PluginConfig::deserialize_storage")]
    pub storage: Config,

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
    fn deserialize_storage<'de, D>(deserializer: D) -> std::result::Result<Config, D::Error>
    where
        D: Deserializer<'de>,
    {
        let storage = serde_json::Value::deserialize(deserializer)?;
        let typ = storage.as_object().and_then(|obj| obj.get("type").and_then(|typ| typ.as_str()));
        match typ {
            Some("ram") => {
                match storage
                    .as_object()
                    .and_then(|obj| {
                        obj.get("ram").map(|ram| serde_json::from_value::<RamConfig>(ram.clone()))
                    })
                    .unwrap_or_else(|| Ok(RamConfig::default()))
                {
                    Err(e) => Err(de::Error::custom(e.to_string())),
                    Ok(_) => Ok(Config::Ram),
                }
            }
            _ => match serde_json::from_value::<rmqtt_storage::Config>(storage) {
                Err(e) => Err(de::Error::custom(e.to_string())),
                Ok(s_cfg) => Ok(Config::Storage(s_cfg)),
            },
        }
    }

    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum Config {
    Ram,
    Storage(rmqtt_storage::Config),
}

impl Default for Config {
    #[inline]
    fn default() -> Self {
        Config::Ram
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RamConfig {}
