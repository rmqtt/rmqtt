use std::collections::BTreeMap;
use std::time::Duration;

use pulsar::consumer::{DeadLetterPolicy, InitialPosition};
use pulsar::ConsumerOptions;
use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{Serialize, Serializer};

use rmqtt::itertools::Itertools;
use rmqtt::{
    anyhow,
    bytes::Bytes,
    log,
    regex::Regex,
    serde_json::{self, json},
};
use rmqtt::{
    settings::{deserialize_duration, deserialize_duration_option},
    MqttError, QoS, QoSEx, Result, TopicName,
};

use crate::bridge::BridgeName;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default)]
    pub bridges: Vec<Bridge>,
}

impl PluginConfig {
    pub(crate) fn prepare(&mut self) {
        for bridge in self.bridges.iter_mut() {
            for entry in bridge.entries.iter_mut() {
                entry.local.prepare_topic();
            }
        }
    }
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Bridge {
    #[serde(default)]
    pub enable: bool,
    #[serde(default)]
    pub name: BridgeName,
    pub servers: String,
    #[serde(default)]
    pub consumer_name_prefix: Option<String>,

    // contains a list of PEM encoded certificates
    #[serde(default)]
    pub cert_chain_file: Option<String>,
    // allow insecure TLS connection if set to true
    // defaults to *false*
    #[serde(default)]
    pub allow_insecure_connection: bool,
    // whether hostname verification is enabled when insecure TLS connection is allowed
    // defaults to *true*
    #[serde(default = "Bridge::tls_hostname_verification_enabled_default")]
    pub tls_hostname_verification_enabled: bool,

    #[serde(default)]
    pub auth: Auth,

    #[serde(default)]
    pub entries: Vec<Entry>,

    #[serde(default = "Bridge::expiry_interval_default", deserialize_with = "deserialize_duration")]
    pub expiry_interval: Duration,
}

impl Bridge {
    fn tls_hostname_verification_enabled_default() -> bool {
        true
    }

    fn expiry_interval_default() -> Duration {
        Duration::from_secs(300)
    }
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Auth {
    pub name: Option<AuthName>,
    pub data: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum AuthName {
    Token,
    OAuth2,
}

#[derive(Default, Debug, Clone, Copy, Deserialize, Serialize)]
pub enum PayloadFormat {
    #[default]
    Bytes,
    Json,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum PayloadData {
    Bytes(Bytes),
    Json(serde_json::Value),
}

impl PayloadData {
    #[inline]
    pub fn bytes(self) -> Result<Bytes> {
        match self {
            PayloadData::Bytes(data) => Ok(data),
            PayloadData::Json(v) => serde_json::to_vec(&v).map(Bytes::from).map_err(MqttError::from),
        }
    }

    //payload_path - /aa/bb
    #[inline]
    pub fn extract(self, payload_path: Option<&str>) -> Result<Option<Bytes>> {
        match self {
            PayloadData::Bytes(data) => Ok(Some(data)),
            PayloadData::Json(mut val) => {
                let data = if let Some(payload_path) = payload_path {
                    val.pointer_mut(payload_path).and_then(|v| match v.take() {
                        serde_json::Value::Null => None,
                        serde_json::Value::String(s) => Some(Ok(Bytes::from(s))),
                        payload => {
                            Some(serde_json::to_vec(&payload).map(Bytes::from).map_err(MqttError::from))
                        }
                    })
                } else {
                    match val {
                        serde_json::Value::Null => None,
                        serde_json::Value::String(s) => Some(Ok(Bytes::from(s))),
                        payload => {
                            Some(serde_json::to_vec(&payload).map(Bytes::from).map_err(MqttError::from))
                        }
                    }
                };

                match data {
                    Some(Ok(d)) => Ok(Some(d)),
                    Some(Err(e)) => Err(e),
                    None => Ok(None),
                }
            }
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

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Remote {
    pub topic: Option<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    pub topic_regex: Option<String>,
    #[serde(
        default,
        deserialize_with = "Remote::deserialize_subscription_type",
        serialize_with = "Remote::serialize_subscription_type"
    )]
    pub subscription_type: Option<pulsar::SubType>,
    pub subscription: Option<String>,
    pub lookup_namespace: Option<String>,
    #[serde(default, deserialize_with = "deserialize_duration_option")]
    pub topic_refresh_interval: Option<Duration>,
    pub consumer_id: Option<u64>,
    pub batch_size: Option<u32>,
    #[serde(
        default,
        deserialize_with = "Remote::deserialize_dead_letter_policy",
        serialize_with = "Remote::serialize_dead_letter_policy"
    )]
    pub dead_letter_policy: Option<DeadLetterPolicy>,
    #[serde(default, deserialize_with = "deserialize_duration_option")]
    pub unacked_message_resend_delay: Option<Duration>,

    #[serde(
        default,
        deserialize_with = "Remote::deserialize_options",
        serialize_with = "Remote::serialize_options"
    )]
    pub options: ConsumerOptions,

    #[serde(default)]
    pub payload_format: PayloadFormat,
    #[serde(default, deserialize_with = "Remote::deserialize_payload_path")]
    pub payload_path: Option<String>,
}

impl Remote {
    pub fn deserialize_payload_path<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let payload_path = String::deserialize(deserializer)?;
        let payload_path = if !payload_path.is_empty() {
            if !payload_path.starts_with("/") {
                Some(["/", payload_path.as_str()].concat())
            } else {
                Some(payload_path)
            }
        } else {
            None
        };
        Ok(payload_path)
    }

    pub fn deserialize_subscription_type<'de, D>(deserializer: D) -> Result<Option<pulsar::SubType>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let sub_type = match String::deserialize(deserializer)?.to_lowercase().as_str() {
            "exclusive" => Some(pulsar::SubType::Exclusive),
            "shared" => Some(pulsar::SubType::Shared),
            "failover" => Some(pulsar::SubType::Failover),
            "key_shared" => Some(pulsar::SubType::KeyShared),
            _ => None,
        };
        Ok(sub_type)
    }

    #[inline]
    pub fn serialize_subscription_type<S>(
        sub_type: &Option<pulsar::SubType>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match sub_type {
            Some(pulsar::SubType::Exclusive) => Some("exclusive"),
            Some(pulsar::SubType::Shared) => Some("shared"),
            Some(pulsar::SubType::Failover) => Some("failover"),
            Some(pulsar::SubType::KeyShared) => Some("key_shared"),
            None => None,
        }
        .serialize(serializer)
    }

    pub fn deserialize_dead_letter_policy<'de, D>(
        deserializer: D,
    ) -> Result<Option<DeadLetterPolicy>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let json_val: serde_json::Value = serde_json::Value::deserialize(deserializer)?;
        log::debug!("DeadLetterPolicy json_val: {:?}", json_val);
        if let Some(obj) = json_val.as_object() {
            let max_redeliver_count =
                obj.get("max_redeliver_count").and_then(|v| v.as_u64().map(|v| v as usize));
            let dead_letter_topic = obj.get("dead_letter_topic").and_then(|v| v.as_str().map(|v| v.into()));
            if let (Some(max_redeliver_count), Some(dead_letter_topic)) =
                (max_redeliver_count, dead_letter_topic)
            {
                Ok(Some(DeadLetterPolicy { max_redeliver_count, dead_letter_topic }))
            } else {
                Err(de::Error::custom(format!("Invalid DeadLetterPolicy, {:?}", json_val)))
            }
        } else {
            Ok(None)
        }
    }

    #[inline]
    pub fn serialize_dead_letter_policy<S>(
        policy: &Option<DeadLetterPolicy>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if let Some(policy) = policy {
            json!({
                "max_redeliver_count": policy.max_redeliver_count,
                "dead_letter_topic": policy.dead_letter_topic,
            })
        } else {
            serde_json::Value::Null
        }
        .serialize(serializer)
    }

    pub fn deserialize_options<'de, D>(deserializer: D) -> Result<ConsumerOptions, D::Error>
    where
        D: Deserializer<'de>,
    {
        let json_val: serde_json::Value = serde_json::Value::deserialize(deserializer)?;
        log::debug!("ConsumerOptions json_val: {:?}", json_val);
        if let Some(obj) = json_val.as_object() {
            let priority_level = obj.get("priority_level").and_then(|v| v.as_i64().map(|v| v as i32));
            let durable = obj.get("durable").and_then(|v| v.as_bool());
            let read_compacted = obj.get("read_compacted").and_then(|v| v.as_bool());
            let initial_position = obj.get("initial_position").and_then(|v| v.as_str()).map(|v| {
                match v.to_lowercase().as_str() {
                    "latest" => Ok(InitialPosition::Latest),
                    "earliest" => Ok(InitialPosition::Earliest),
                    _ => Err(de::Error::custom(format!("Invalid InitialPosition, {:?}", v))),
                }
            });
            let initial_position = match initial_position {
                Some(Ok(initial_position)) => initial_position,
                None => InitialPosition::default(),
                Some(Err(e)) => return Err(e),
            };
            log::debug!("ConsumerOptions initial_position: {:?}", initial_position);
            let metadata = obj
                .get("metadata")
                .and_then(|v| {
                    v.as_object().map(|map| {
                        map.iter()
                            .map(|(k, v)| (String::from(k), String::from(v.as_str().unwrap_or_default())))
                            .collect::<BTreeMap<_, _>>()
                    })
                })
                .unwrap_or_default();
            Ok(ConsumerOptions {
                priority_level,
                durable,
                read_compacted,
                initial_position,
                metadata,
                ..Default::default()
            })
        } else {
            Ok(ConsumerOptions::default())
        }
    }

    #[inline]
    pub fn serialize_options<S>(opts: &ConsumerOptions, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let initial_position = match opts.initial_position {
            InitialPosition::Latest => "latest",
            InitialPosition::Earliest => "earliest",
        };
        json!({
            "priority_level": opts.priority_level,
            "durable": opts.durable,
            "read_compacted": opts.read_compacted,
            "initial_position": initial_position,
            "metadata": opts.metadata,
        })
        .serialize(serializer)
    }
}

const REMOTE_TOPIC_NAME: &str = "${remote.topic}";
const REMOTE_PROPERTIES_PATTERN: &str = r"\$\{remote\.properties\.(.*?)\}";
const REMOTE_PAYLOAD_PATTERN: &str = r"\$\{remote\.payload\.(.*?)\}";

type Placeholder = String;
type Path = String;

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum LocalTopicPattern {
    /// ${remote.topic}
    RemoteTopic,
    /// xxxx, ${remote.properties.xxxx}
    RemoteProperties(Path, Placeholder),
    /// xxxx, ${remote.payload.xxxx}
    /// a/b/c, ${remote.payload.a/b/c}, Path in JSON
    RemotePayload(Path, Placeholder),
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct LocalTopic {
    name: TopicName,
    pattern: Option<LocalTopicPattern>,
    perfect_match: bool,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Local {
    #[serde(default, deserialize_with = "Local::deserialize_topic")]
    pub topic: Option<LocalTopic>,
    #[serde(default, deserialize_with = "Local::deserialize_topics")]
    pub topics: Vec<LocalTopic>,

    #[serde(default, deserialize_with = "Local::deserialize_qos")]
    pub qos: Option<QoS>,

    #[serde(default)]
    pub retain: Option<bool>,

    #[serde(default = "Local::allow_empty_forward_default")]
    pub allow_empty_forward: bool,

    #[serde(default, skip)]
    pub(crate) props_topic_names: BTreeMap<String, String>,
    #[serde(default, skip)]
    pub(crate) payload_topic_names: BTreeMap<String, String>,
}

impl Local {
    fn allow_empty_forward_default() -> bool {
        true
    }

    #[inline]
    pub(crate) fn prepare_topic(&mut self) {
        if let Some(item) = self.topic.as_ref() {
            match &item.pattern {
                Some(LocalTopicPattern::RemoteProperties(name, ref placeholder)) => {
                    self.props_topic_names.insert(name.into(), placeholder.as_str().into());
                }
                Some(LocalTopicPattern::RemotePayload(path, ref placeholder)) => {
                    self.payload_topic_names.insert(["/", path].concat(), placeholder.as_str().into());
                }
                _ => {}
            }
        }

        for item in self.topics.iter() {
            match &item.pattern {
                Some(LocalTopicPattern::RemoteProperties(name, ref placeholder)) => {
                    self.props_topic_names.insert(name.into(), placeholder.as_str().into());
                }
                Some(LocalTopicPattern::RemotePayload(path, ref placeholder)) => {
                    self.payload_topic_names.insert(["/", path].concat(), placeholder.as_str().into());
                }
                _ => {}
            }
        }
    }

    #[inline]
    fn make_one_topic(
        topic: &TopicName,
        pattern: &LocalTopicPattern,
        perfect_match: bool,
        remote_topic: &str,
        props_topics: &BTreeMap<&str, &str>,
        payload_topics: &Option<BTreeMap<&str, Vec<&str>>>,
    ) -> Vec<TopicName> {
        let mut topics = Vec::new();
        match (pattern, perfect_match) {
            (LocalTopicPattern::RemoteTopic, true) => topics.push(TopicName::from(remote_topic)),
            (LocalTopicPattern::RemoteTopic, false) => {
                topics.push(TopicName::from(topic.replace(REMOTE_TOPIC_NAME, remote_topic)))
            }
            (LocalTopicPattern::RemoteProperties(ref name, ref placeholder), true) => {
                if let Some(t) = props_topics.get(placeholder.as_str()).map(|t| TopicName::from(*t)) {
                    topics.push(t);
                } else {
                    log::warn!(
                        "No matching topic found in RemoteProperties, placeholder: {}, name: {}, topic: {}",
                        placeholder,
                        name,
                        topic
                    );
                }
            }
            (LocalTopicPattern::RemoteProperties(ref name, ref placeholder), false) => {
                if let Some(t) = props_topics
                    .get(placeholder.as_str())
                    .map(|t| TopicName::from(topic.replace(placeholder, t)))
                {
                    topics.push(t);
                } else {
                    log::warn!(
                        "No matching topic found in RemoteProperties, placeholder: {}, name: {}, topic: {}",
                        placeholder,
                        name,
                        topic
                    );
                }
            }
            (LocalTopicPattern::RemotePayload(ref path, ref placeholder), true) => {
                if let Some(ts) = payload_topics.as_ref().and_then(|payload_topics| {
                    payload_topics
                        .get(placeholder.as_str())
                        .map(|ts| ts.iter().map(|t| TopicName::from(*t)).collect_vec())
                }) {
                    topics.extend(ts);
                } else {
                    log::warn!(
                        "No matching topic found in RemotePayload, placeholder: {}, path: {}, topic: {}",
                        placeholder,
                        path,
                        topic
                    );
                }
            }
            (LocalTopicPattern::RemotePayload(ref path, ref placeholder), false) => {
                if let Some(ts) = payload_topics.as_ref().and_then(|payload_topics| {
                    payload_topics.get(placeholder.as_str()).map(|ts| {
                        ts.iter().map(|t| TopicName::from(topic.replace(placeholder, t))).collect_vec()
                    })
                }) {
                    topics.extend(ts);
                } else {
                    log::warn!(
                        "No matching topic found in RemotePayload, placeholder: {}, path: {}, topic: {}",
                        placeholder,
                        path,
                        topic
                    );
                }
            }
        };
        topics
    }

    #[inline]
    pub fn make_topics(
        &self,
        remote_topic: &str,
        props_topics: BTreeMap<&str, &str>,
        payload_topics: Option<BTreeMap<&str, Vec<&str>>>,
    ) -> Vec<TopicName> {
        let mut topics = if let Some(item) = self.topic.as_ref() {
            if let Some(pattern) = &item.pattern {
                Self::make_one_topic(
                    &item.name,
                    pattern,
                    item.perfect_match,
                    remote_topic,
                    &props_topics,
                    &payload_topics,
                )
            } else {
                vec![item.name.clone()]
            }
        } else {
            Vec::default()
        };

        for item in self.topics.iter() {
            if let Some(pattern) = &item.pattern {
                topics.extend(Self::make_one_topic(
                    &item.name,
                    pattern,
                    item.perfect_match,
                    remote_topic,
                    &props_topics,
                    &payload_topics,
                ));
            } else {
                topics.push(item.name.clone());
            }
        }
        log::debug!("make_topics topics: {:?}", topics);
        topics
    }

    #[inline]
    pub fn make_retain(&self, remote_retain: Option<bool>) -> bool {
        remote_retain.unwrap_or(self.retain.unwrap_or_default())
    }

    #[inline]
    pub fn make_qos(&self, remote_qos: Option<QoS>) -> QoS {
        match (self.qos, remote_qos) {
            (Some(q1), Some(q2)) => q1.less_value(q2),
            (Some(q1), None) => q1,
            (None, Some(q2)) => q2,
            (None, None) => QoS::AtMostOnce,
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

    fn parse_topic_pattern(topic: String) -> Result<LocalTopic> {
        let re = Regex::new(REMOTE_PAYLOAD_PATTERN).map_err(anyhow::Error::new)?;
        if let Some(caps) = re.captures(topic.as_str()) {
            if let (Some(placeholder), Some(name)) = (caps.get(0), caps.get(1)) {
                let placeholder = placeholder.as_str().into();
                let perfect_match = placeholder == topic;
                let pattern = LocalTopicPattern::RemotePayload(name.as_str().into(), placeholder);
                return Ok(LocalTopic {
                    name: TopicName::from(topic),
                    pattern: Some(pattern),
                    perfect_match,
                });
            }
        }

        let re = Regex::new(REMOTE_PROPERTIES_PATTERN).map_err(anyhow::Error::new)?;
        if let Some(caps) = re.captures(topic.as_str()) {
            if let (Some(placeholder), Some(name)) = (caps.get(0), caps.get(1)) {
                let placeholder = placeholder.as_str().into();
                let perfect_match = placeholder == topic;
                let pattern = LocalTopicPattern::RemoteProperties(name.as_str().into(), placeholder);
                return Ok(LocalTopic {
                    name: TopicName::from(topic),
                    pattern: Some(pattern),
                    perfect_match,
                });
            }
        }

        if topic.contains(REMOTE_TOPIC_NAME) {
            let perfect_match = REMOTE_TOPIC_NAME == topic;
            Ok(LocalTopic {
                name: TopicName::from(topic),
                pattern: Some(LocalTopicPattern::RemoteTopic),
                perfect_match,
            })
        } else {
            Ok(LocalTopic { name: TopicName::from(topic), pattern: None, perfect_match: false })
        }
    }

    #[inline]
    pub fn deserialize_topic<'de, D>(deserializer: D) -> Result<Option<LocalTopic>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let topic = String::deserialize(deserializer)?;
        Self::parse_topic_pattern(topic).map(Some).map_err(de::Error::custom)
    }

    #[allow(clippy::type_complexity)]
    #[inline]
    pub fn deserialize_topics<'de, D>(deserializer: D) -> Result<Vec<LocalTopic>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let topics: Vec<String> = Vec::deserialize(deserializer)?;
        let topics = topics
            .into_iter()
            .map(Self::parse_topic_pattern)
            .collect::<Result<Vec<_>>>()
            .map_err(de::Error::custom)?;
        Ok(topics)
    }
}
