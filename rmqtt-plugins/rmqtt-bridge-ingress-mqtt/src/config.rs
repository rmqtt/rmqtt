use std::fs::File;
use std::num::NonZeroU32;
use std::time::Duration;

use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};

use ntex::util::{ByteString, Bytes};
use ntex_mqtt::v3::codec::LastWill as LastWillV3;
use ntex_mqtt::v5::codec::LastWill as LastWillV5;
use ntex_mqtt::v5::codec::UserProperties;
use ntex_mqtt::QoS;
use rustls::{Certificate, PrivateKey};
use rustls_pemfile::read_one;

use rmqtt::{
    anyhow,
    base64::prelude::{Engine, BASE64_STANDARD},
    ntex_mqtt::types::{Protocol, MQTT_LEVEL_31, MQTT_LEVEL_311, MQTT_LEVEL_5},
    serde_json::{self, Map, Value},
};

use rmqtt::serde_json::json;
use rmqtt::{
    settings::{deserialize_duration, to_duration, Bytesize},
    MqttError, Result, TopicName,
};

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
    pub name: String,
    #[serde(default)]
    pub client_id_prefix: String,
    #[serde(default, deserialize_with = "Bridge::deserialize_server")]
    pub server: ServerAddr,
    #[serde(
        default,
        deserialize_with = "Bridge::deserialize_cert",
        serialize_with = "Bridge::serialize_cert"
    )]
    pub root_cert: Option<Cert>,
    // #Client Certificate File
    #[serde(
        default,
        deserialize_with = "Bridge::deserialize_cert",
        serialize_with = "Bridge::serialize_cert"
    )]
    pub client_cert: Option<Cert>,
    // #Client key file
    #[serde(default, deserialize_with = "Bridge::deserialize_key", serialize_with = "Bridge::serialize_key")]
    pub client_key: Option<Key>,

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

    #[serde(default = "Bridge::expiry_interval_default", deserialize_with = "deserialize_duration")]
    pub expiry_interval: Duration,

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

    fn mqtt_ver_default() -> Protocol {
        Protocol::MQTT(MQTT_LEVEL_311)
    }

    fn expiry_interval_default() -> Duration {
        Duration::from_secs(300)
    }

    #[inline]
    pub fn deserialize_mqtt_ver<'de, D>(deserializer: D) -> Result<Protocol, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = String::deserialize(deserializer)?;
        let protocol = match v.as_str() {
            "v3" | "V3" => Protocol::MQTT(MQTT_LEVEL_31),
            "v4" | "V4" => Protocol::MQTT(MQTT_LEVEL_311),
            "v5" | "V5" => Protocol::MQTT(MQTT_LEVEL_5),
            _ => return Err(serde::de::Error::custom("invalid value")),
        };
        Ok(protocol)
    }

    #[inline]
    pub fn deserialize_server<'de, D>(deserializer: D) -> Result<ServerAddr, D::Error>
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

    #[inline]
    pub fn deserialize_cert<'de, D>(deserializer: D) -> Result<Option<Cert>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let file_name: String = String::deserialize(deserializer)?;
        let cert = load_cert(&file_name).map_err(serde::de::Error::custom)?;

        Ok(Some(Cert { file_name, cert }))
    }

    #[inline]
    pub fn serialize_cert<S>(c: &Option<Cert>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if let Some(c) = c {
            c.file_name.serialize(serializer)
        } else {
            "".serialize(serializer)
        }
    }

    #[inline]
    pub fn deserialize_key<'de, D>(deserializer: D) -> Result<Option<Key>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let file_name: String = String::deserialize(deserializer)?;
        let key = load_key(&file_name).map_err(serde::de::Error::custom)?;

        Ok(Some(Key { file_name, key }))
    }

    #[inline]
    pub fn serialize_key<S>(c: &Option<Key>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if let Some(c) = c {
            c.file_name.serialize(serializer)
        } else {
            "".serialize(serializer)
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

#[derive(Debug, Clone)]
pub(crate) struct Cert {
    pub file_name: String,
    pub cert: Vec<Certificate>,
}

#[derive(Debug, Clone)]
pub(crate) struct Key {
    pub file_name: String,
    pub key: PrivateKey,
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
    pub fn serialize_last_will<S>(v: &Option<LastWillV3>, serializer: S) -> Result<S::Ok, S::Error>
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
    pub fn deserialize_last_will<'de, D>(deserializer: D) -> Result<Option<LastWillV3>, D::Error>
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
    pub fn serialize_last_will<S>(v: &Option<LastWillV5>, serializer: S) -> Result<S::Ok, S::Error>
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
    pub fn deserialize_last_will<'de, D>(deserializer: D) -> Result<Option<LastWillV5>, D::Error>
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
    pub remote: Remote,

    #[serde(default)]
    pub local: Local,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Remote {
    #[serde(default = "Remote::qos_default", deserialize_with = "Remote::deserialize_qos")]
    pub qos: QoS,
    #[serde(default)]
    pub topic: String,
}

impl Default for Remote {
    fn default() -> Self {
        Self { qos: QoS::AtMostOnce, topic: String::default() }
    }
}

impl Remote {
    fn qos_default() -> QoS {
        QoS::AtMostOnce
    }

    #[inline]
    pub fn deserialize_qos<'de, D>(deserializer: D) -> Result<QoS, D::Error>
    where
        D: Deserializer<'de>,
    {
        match u8::deserialize(deserializer)? {
            0 => Ok(QoS::AtMostOnce),
            1 => Ok(QoS::AtLeastOnce),
            2 => Ok(QoS::ExactlyOnce),
            _ => Err(de::Error::custom("invalid value")),
        }
    }
}

type HasPattern = bool; //${remote.topic}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Local {
    #[serde(default, deserialize_with = "Local::deserialize_qos")]
    pub qos: Option<QoS>,
    #[serde(default, deserialize_with = "Local::deserialize_topic")]
    pub topic: (String, HasPattern),
    // #[serde(default)]
    // pub payload: String,
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
    pub fn make_topic(&self, remote_topic: &str) -> TopicName {
        if self.topic_has_pattern() {
            TopicName::from(self.topic().replace("${remote.topic}", remote_topic))
        } else {
            TopicName::from(self.topic())
        }
    }

    #[inline]
    pub fn make_retain(&self, remote_retain: bool) -> bool {
        self.retain.unwrap_or(remote_retain)
    }

    #[inline]
    pub fn make_qos(&self, remote_qos: QoS) -> rmqtt::QoS {
        let qos = self.qos.unwrap_or(remote_qos);
        match qos {
            QoS::AtMostOnce => rmqtt::QoS::AtMostOnce,
            QoS::AtLeastOnce => rmqtt::QoS::AtLeastOnce,
            QoS::ExactlyOnce => rmqtt::QoS::ExactlyOnce,
        }
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
        let has_pattern = topic.contains("${remote.topic}");
        Ok((topic, has_pattern))
    }
}

fn last_will_basic(obj: &Map<String, Value>) -> Result<(QoS, bool, ByteString, Bytes)> {
    let qos = obj
        .get("qos")
        .and_then(|q| q.as_u64().map(|q| QoS::try_from(q as u8)))
        .unwrap_or(Ok(QoS::AtMostOnce))
        .map_err(|e| MqttError::from(format!("{:?}", e)))?;
    let retain = obj.get("retain").and_then(|retain| retain.as_bool()).unwrap_or_default();
    let topic = obj.get("topic").and_then(|topic| topic.as_str()).unwrap_or_default();
    let message = obj.get("message").and_then(|message| message.as_str()).unwrap_or_default();
    let encoding = obj.get("encoding").and_then(|encoding| encoding.as_str()).unwrap_or_default();
    let message = if encoding.eq_ignore_ascii_case("plain") {
        Bytes::from(String::from(message))
    } else if encoding.eq_ignore_ascii_case("base64") {
        Bytes::from(BASE64_STANDARD.decode(message).map_err(anyhow::Error::new)?)
    } else {
        Bytes::from(String::from(message))
    };
    Ok((qos, retain, ByteString::from(topic), message))
}

fn load_cert(path: &str) -> Result<Vec<Certificate>> {
    let cert_pem = std::fs::read_to_string(path)?;
    let certs =
        rustls_pemfile::certs(&mut cert_pem.as_bytes())?.into_iter().map(Certificate).collect::<Vec<_>>();
    if certs.is_empty() {
        Err(MqttError::from("No certificate was found in the certificate file."))
    } else {
        Ok(certs)
    }
}

fn load_key(path: &str) -> Result<PrivateKey> {
    use std::io::Read;
    let mut file = File::open(path)?;
    // 读取文件内容
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;

    let mut pem_reader = &contents[..];
    let key = read_one(&mut pem_reader)?;

    match key {
        Some(rustls_pemfile::Item::X509Certificate(key)) => Ok(PrivateKey(key)),
        Some(rustls_pemfile::Item::PKCS8Key(key)) => Ok(PrivateKey(key)),
        Some(rustls_pemfile::Item::ECKey(key)) => Ok(PrivateKey(key)),
        Some(rustls_pemfile::Item::RSAKey(key)) => Ok(PrivateKey(key)),
        Some(rustls_pemfile::Item::Crl(key)) => Ok(PrivateKey(key)),
        _ => Err(MqttError::from("Invalid key format")),
    }
}
