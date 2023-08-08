use serde::de::{self, Deserialize, Deserializer};
use std::time::Duration;

use rmqtt::broker::types::QoS;
use rmqtt::serde_json;
use rmqtt::settings::to_duration;
use rmqtt::Result;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(
        default = "PluginConfig::publish_qos_default",
        deserialize_with = "PluginConfig::deserialize_publish_qos"
    )]
    pub publish_qos: QoS,

    #[serde(
        default = "PluginConfig::publish_interval_default",
        deserialize_with = "PluginConfig::deserialize_publish_interval"
    )]
    pub publish_interval: Duration,
}

impl PluginConfig {
    #[inline]
    fn publish_qos_default() -> QoS {
        QoS::AtLeastOnce
    }

    #[inline]
    fn publish_interval_default() -> Duration {
        Duration::from_secs(60)
    }

    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }

    #[inline]
    fn deserialize_publish_qos<'de, D>(deserializer: D) -> Result<QoS, D::Error>
    where
        D: Deserializer<'de>,
    {
        let qos = match u8::deserialize(deserializer)? {
            0 => QoS::AtMostOnce,
            1 => QoS::AtLeastOnce,
            2 => QoS::ExactlyOnce,
            _ => return Err(de::Error::custom("QoS configuration error, only values (0,1,2) are supported")),
        };
        Ok(qos)
    }

    #[inline]
    pub fn deserialize_publish_interval<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = String::deserialize(deserializer)?;
        let d = to_duration(&v);
        if d < Duration::from_secs(1) {
            Err(de::Error::custom("'publish_interval' must be greater than 1 second"))
        } else {
            Ok(d)
        }
    }
}
