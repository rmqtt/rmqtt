use std::borrow::Cow;
use std::path::PathBuf;
use std::time::Duration;

use rmqtt::types::{QoS, TopicName};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

use crate::bridge::BridgeName;

use rmqtt::utils::{deserialize_duration, deserialize_duration_option};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default)]
    pub bridges: Vec<Bridge>,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Bridge {
    #[serde(default)]
    pub(crate) enable: bool,
    #[serde(default)]
    pub(crate) name: BridgeName,
    pub(crate) servers: String,
    #[serde(default)]
    pub(crate) consumer_name_prefix: Option<String>,

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

    #[serde(default = "Bridge::expiry_interval_default", deserialize_with = "deserialize_duration")]
    pub expiry_interval: Duration,
}

impl Bridge {
    fn expiry_interval_default() -> Duration {
        Duration::from_secs(300)
    }

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

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Auth {
    pub(crate) jwt: Option<String>,
    pub(crate) jwt_seed: Option<String>,
    pub(crate) nkey: Option<String>,
    pub(crate) username: Option<String>,
    pub(crate) password: Option<String>,
    pub(crate) token: Option<String>,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Entry {
    #[serde(default)]
    pub remote: Remote,

    #[serde(default)]
    pub local: Local,
}

type HasPattern = bool; //${remote.topic}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Remote {
    pub topic: String,
    pub group: Option<String>,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Local {
    #[serde(default, deserialize_with = "Local::deserialize_qos")]
    pub qos: Option<QoS>,
    #[serde(default, deserialize_with = "Local::deserialize_topic")]
    pub topic: (String, HasPattern),
    #[serde(default)]
    pub retain: Option<bool>,

    #[serde(default = "Local::allow_empty_forward_default")]
    pub allow_empty_forward: bool,
}

impl Local {
    #[inline]
    fn allow_empty_forward_default() -> bool {
        true
    }

    #[inline]
    pub fn topic(&self) -> &str {
        &self.topic.0
    }

    #[inline]
    pub fn topic_has_pattern(&self) -> bool {
        self.topic.1
    }

    #[inline]
    pub fn make_topic(&self, remote_topic: Option<Cow<str>>) -> TopicName {
        if self.topic_has_pattern() {
            if let Some(remote_topic) = remote_topic {
                TopicName::from(self.topic().replace("${remote.topic}", remote_topic.as_ref()))
            } else {
                TopicName::from(self.topic().replace("${remote.topic}", ""))
            }
        } else {
            TopicName::from(self.topic())
        }
    }

    // #[inline]
    // pub fn make_topics(
    //     &self,
    //     remote_topic: &str,
    //     props_topics: BTreeMap<&str, &str>,
    //     payload_topics: Option<BTreeMap<&str, Vec<&str>>>,
    // ) -> Vec<TopicName> {
    //     let mut topics = if let Some(item) = self.topic.as_ref() {
    //         if let Some(pattern) = &item.pattern {
    //             Self::make_one_topic(
    //                 &item.name,
    //                 pattern,
    //                 item.perfect_match,
    //                 remote_topic,
    //                 &props_topics,
    //                 &payload_topics,
    //             )
    //         } else {
    //             vec![item.name.clone()]
    //         }
    //     } else {
    //         Vec::default()
    //     };
    //
    //     for item in self.topics.iter() {
    //         if let Some(pattern) = &item.pattern {
    //             topics.extend(Self::make_one_topic(
    //                 &item.name,
    //                 pattern,
    //                 item.perfect_match,
    //                 remote_topic,
    //                 &props_topics,
    //                 &payload_topics,
    //             ));
    //         } else {
    //             topics.push(item.name.clone());
    //         }
    //     }
    //     log::debug!("make_topics topics: {topics:?}");
    //     topics
    // }

    #[inline]
    pub fn make_retain(&self, remote_retain: Option<bool>) -> bool {
        self.retain.unwrap_or(remote_retain.unwrap_or_default())
    }

    #[inline]
    pub fn make_qos(&self, remote_qos: Option<QoS>) -> QoS {
        self.qos.unwrap_or(remote_qos.unwrap_or(QoS::AtLeastOnce))
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
        let has_pattern = topic.contains("${remote.topic}");
        Ok((topic, has_pattern))
    }
}
