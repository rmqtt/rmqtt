use std::num::NonZeroU32;
use std::time::Duration;

use anyhow::anyhow;
use base64::{engine::general_purpose, Engine as _};
use ntex::util::{ByteString, Bytes};
use ntex_mqtt::v3::codec::LastWill as LastWillV3;
use ntex_mqtt::v5::codec::LastWill as LastWillV5;
use ntex_mqtt::v5::codec::UserProperties;
use ntex_mqtt::QoS;
use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use serde_json::json;
use serde_json::{self, Map, Value};

use rmqtt::{
    codec::types::{Protocol, MQTT_LEVEL_31, MQTT_LEVEL_311, MQTT_LEVEL_5},
    utils::{deserialize_duration, to_duration, Bytesize},
    Result,
};

use crate::bridge::BridgeName;

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
    #[serde(default)]
    pub client_id_prefix: String,
    #[serde(default, deserialize_with = "Bridge::deserialize_server")]
    pub server: ServerAddr,
    pub root_cert: Option<String>,
    // #Client Certificate File
    pub client_cert: Option<String>,
    // #Client key file
    pub client_key: Option<String>,

    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,

    #[serde(default = "Bridge::concurrent_client_limit_default")]
    pub concurrent_client_limit: usize,
    #[serde(default = "Bridge::connect_timeout_default", deserialize_with = "deserialize_duration")]
    pub connect_timeout: Duration,
    #[serde(default = "Bridge::keepalive_default", deserialize_with = "deserialize_duration")]
    pub keepalive: Duration,
    #[serde(default = "Bridge::reconnect_interval_default", deserialize_with = "deserialize_duration")]
    pub reconnect_interval: Duration,
    #[serde(default = "Bridge::message_channel_capacity_default")]
    pub message_channel_capacity: usize,
    #[serde(default = "Bridge::mqtt_ver_default", deserialize_with = "Bridge::deserialize_mqtt_ver")]
    pub mqtt_ver: Protocol,
    #[serde(default)]
    pub v4: MoreV3,
    #[serde(default)]
    pub v5: MoreV5,

    #[serde(default)]
    pub entries: Vec<Entry>,
}

impl Bridge {
    fn concurrent_client_limit_default() -> usize {
        1
    }

    fn connect_timeout_default() -> Duration {
        Duration::from_secs(20)
    }

    fn keepalive_default() -> Duration {
        Duration::from_secs(60)
    }

    fn reconnect_interval_default() -> Duration {
        Duration::from_secs(5)
    }

    fn message_channel_capacity_default() -> usize {
        100_000
    }

    fn mqtt_ver_default() -> Protocol {
        Protocol(MQTT_LEVEL_311)
    }

    #[inline]
    pub fn deserialize_mqtt_ver<'de, D>(deserializer: D) -> std::result::Result<Protocol, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = String::deserialize(deserializer)?;
        let protocol = match v.as_str() {
            "v3" | "V3" => Protocol(MQTT_LEVEL_31),
            "v4" | "V4" => Protocol(MQTT_LEVEL_311),
            "v5" | "V5" => Protocol(MQTT_LEVEL_5),
            _ => return Err(serde::de::Error::custom("invalid value")),
        };
        Ok(protocol)
    }

    #[inline]
    pub fn deserialize_server<'de, D>(deserializer: D) -> std::result::Result<ServerAddr, D::Error>
    where
        D: Deserializer<'de>,
    {
        let addr: String = String::deserialize(deserializer)?;
        let addr_split = addr.splitn(2, "://").collect::<Vec<_>>();

        match addr_split.len() {
            0 => Err(serde::de::Error::custom(format!("invalid value, {:?}", addr))),
            1 => Ok(ServerAddr { typ: AddrType::Tcp, addr }),
            _ => {
                let typ = match addr_split[0].to_lowercase().as_str() {
                    "tcp" => AddrType::Tcp,
                    "tls" => AddrType::Tls,
                    _ => return Err(serde::de::Error::custom(format!("invalid value, {:?}", addr))),
                };
                Ok(ServerAddr { typ, addr: addr_split[1].into() })
            }
        }
    }
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub(crate) enum AddrType {
    #[default]
    Tcp,
    Tls,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub(crate) struct ServerAddr {
    pub typ: AddrType,
    pub addr: String,
}

impl ServerAddr {
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn is_tcp(&self) -> bool {
        matches!(self.typ, AddrType::Tcp)
    }

    #[inline]
    pub(crate) fn is_tls(&self) -> bool {
        matches!(self.typ, AddrType::Tls)
    }
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct MoreV3 {
    #[serde(default = "MoreV3::clean_session_default")]
    pub clean_session: bool,
    #[serde(
        default,
        deserialize_with = "MoreV3::deserialize_last_will",
        serialize_with = "MoreV3::serialize_last_will"
    )]
    pub last_will: Option<LastWillV3>,
}

impl MoreV3 {
    fn clean_session_default() -> bool {
        true
    }

    #[inline]
    pub fn serialize_last_will<S>(
        v: &Option<LastWillV3>,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let josn_val = if let Some(v) = v {
            json!({
                "qos": v.qos,
                "retain": v.retain,
                "topic": v.topic,
                "message": v.message,
            })
        } else {
            serde_json::Value::Null
        };
        josn_val.serialize(serializer)
    }

    #[inline]
    pub fn deserialize_last_will<'de, D>(deserializer: D) -> std::result::Result<Option<LastWillV3>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let last_will: serde_json::Value = serde_json::Value::deserialize(deserializer)?;
        if let Some(obj) = last_will.as_object() {
            let (qos, retain, topic, message) = last_will_basic(obj).map_err(de::Error::custom)?;
            Ok(Some(LastWillV3 { qos, retain, topic, message }))
        } else {
            Ok(None)
        }
    }
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct MoreV5 {
    #[serde(default = "MoreV5::clean_start_default")]
    pub clean_start: bool,
    #[serde(default, deserialize_with = "deserialize_duration")]
    pub session_expiry_interval: Duration,
    #[serde(default = "MoreV5::receive_maximum_default")]
    pub receive_maximum: u16,
    #[serde(default = "MoreV5::maximum_packet_size_default")]
    pub maximum_packet_size: Bytesize,
    #[serde(default)]
    pub topic_alias_maximum: u16,
    #[serde(
        default,
        deserialize_with = "MoreV5::deserialize_last_will",
        serialize_with = "MoreV5::serialize_last_will"
    )]
    pub last_will: Option<LastWillV5>,
}

impl MoreV5 {
    fn clean_start_default() -> bool {
        true
    }

    fn receive_maximum_default() -> u16 {
        16
    }

    fn maximum_packet_size_default() -> Bytesize {
        Bytesize::from(1024 * 1024)
    }

    #[inline]
    pub fn serialize_last_will<S>(
        v: &Option<LastWillV5>,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let josn_val = if let Some(v) = v {
            json!({
                "qos": v.qos,
                "retain": v.retain,
                "topic": v.topic,
                "message": v.message,
                "will_delay_interval_sec": v.will_delay_interval_sec,
                "correlation_data": v.correlation_data,
                "message_expiry_interval": v.message_expiry_interval,
                "content_type": v.content_type,
                "user_properties": v.user_properties,
                "is_utf8_payload": v.is_utf8_payload,
                "response_topic": v.response_topic,
            })
        } else {
            serde_json::Value::Null
        };
        josn_val.serialize(serializer)
    }

    #[inline]
    pub fn deserialize_last_will<'de, D>(deserializer: D) -> std::result::Result<Option<LastWillV5>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let last_will: serde_json::Value = serde_json::Value::deserialize(deserializer)?;
        if let Some(obj) = last_will.as_object() {
            let (qos, retain, topic, message) = last_will_basic(obj).map_err(de::Error::custom)?;
            let will_delay_interval_sec = obj.get("will_delay_interval").and_then(|will_delay_interval| {
                will_delay_interval.as_str().map(|interval| to_duration(interval).as_secs() as u32)
            });
            let message_expiry_interval = obj
                .get("message_expiry_interval")
                .and_then(|message_expiry_interval| {
                    message_expiry_interval.as_str().and_then(|interval| {
                        let interval = to_duration(interval).as_secs() as u32;
                        if interval > 0 {
                            Some(NonZeroU32::new(interval as u32))
                        } else {
                            None
                        }
                    })
                })
                .flatten();
            let content_type =
                obj.get("content_type").and_then(|content_type| content_type.as_str().map(ByteString::from));
            let response_topic = obj
                .get("response_topic")
                .and_then(|response_topic| response_topic.as_str().map(ByteString::from));
            let correlation_data = obj.get("correlation_data").and_then(|correlation_data| {
                correlation_data.as_str().map(|correlation_data| Bytes::from(Vec::from(correlation_data)))
            });
            let user_properties = UserProperties::default(); //: UserProperties,
            let is_utf8_payload = None;
            Ok(Some(LastWillV5 {
                qos,
                retain,
                topic,
                message,
                will_delay_interval_sec,
                correlation_data,
                message_expiry_interval,
                content_type,
                user_properties,
                is_utf8_payload,
                response_topic,
            }))
        } else {
            Ok(None)
        }
    }
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Entry {
    #[serde(default)]
    pub local: Local,

    #[serde(default)]
    pub remote: Remote,
}

impl Entry {}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Remote {
    #[serde(default, deserialize_with = "Remote::deserialize_qos")]
    pub qos: Option<QoS>,
    #[serde(default, deserialize_with = "Remote::deserialize_topic")]
    pub topic: (String, HasPattern),
    #[serde(default)]
    pub retain: Option<bool>,
}

impl Remote {
    #[inline]
    pub fn topic(&self) -> &str {
        &self.topic.0
    }

    #[inline]
    pub fn topic_has_pattern(&self) -> bool {
        self.topic.1
    }

    #[inline]
    pub fn make_topic(&self, remote_topic: &str) -> ntex::util::ByteString {
        if self.topic_has_pattern() {
            ntex::util::ByteString::from(self.topic().replace("${local.topic}", remote_topic))
        } else {
            ntex::util::ByteString::from(self.topic())
        }
    }

    #[inline]
    pub fn make_retain(&self, remote_retain: bool) -> bool {
        self.retain.unwrap_or(remote_retain)
    }

    #[inline]
    pub fn make_qos(&self, remote_qos: rmqtt::types::QoS) -> QoS {
        self.qos.unwrap_or(match remote_qos {
            rmqtt::types::QoS::AtMostOnce => QoS::AtMostOnce,
            rmqtt::types::QoS::AtLeastOnce => QoS::AtLeastOnce,
            rmqtt::types::QoS::ExactlyOnce => QoS::ExactlyOnce,
        })
    }

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
        let has_pattern = topic.contains("${local.topic}");
        Ok((topic, has_pattern))
    }
}

type HasPattern = bool; //${local.topic}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Local {
    #[serde(default)]
    pub topic_filter: String,
}

impl Local {}

fn last_will_basic(obj: &Map<String, Value>) -> Result<(QoS, bool, ByteString, Bytes)> {
    let qos = obj
        .get("qos")
        .and_then(|q| q.as_u64().map(|q| QoS::try_from(q as u8)))
        .unwrap_or(Ok(QoS::AtMostOnce))
        .map_err(|e| anyhow!(format!("{:?}", e)))?;
    let retain = obj.get("retain").and_then(|retain| retain.as_bool()).unwrap_or_default();
    let topic = obj.get("topic").and_then(|topic| topic.as_str()).unwrap_or_default();
    let message = obj.get("message").and_then(|message| message.as_str()).unwrap_or_default();
    let encoding = obj.get("encoding").and_then(|encoding| encoding.as_str()).unwrap_or_default();
    let message = if encoding.eq_ignore_ascii_case("plain") {
        Bytes::from(String::from(message))
    } else if encoding.eq_ignore_ascii_case("base64") {
        Bytes::from(general_purpose::STANDARD.decode(message).map_err(anyhow::Error::new)?)
    } else {
        Bytes::from(String::from(message))
    };
    Ok((qos, retain, ByteString::from(topic), message))
}
