//! Configuration types for the MQTT ingress bridge plugin.
//!
//! Defines bridge connection parameters (server, TLS, credentials, keepalive),
//! MQTT protocol version, per-version settings (v3/v5), and routing entries
//! mapping remote topics to local MQTT topics (including `${remote.topic}` pattern).

use anyhow::anyhow;
use std::num::NonZeroU32;
use std::time::Duration;

use base64::prelude::{Engine, BASE64_STANDARD};
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

use rmqtt::codec::types::{Protocol, MQTT_LEVEL_31, MQTT_LEVEL_311, MQTT_LEVEL_5};

use rmqtt::{
    types::TopicName,
    utils::{deserialize_duration, deserialize_expand_env_vars_option, to_duration, Bytesize},
    Result,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default)]
    pub bridges: Vec<Bridge>,
}

/// How many SUBSCRIBE attempts a single entry makes on one connection before it
/// is given up on until the next reconnect.
///
/// MQTT 3.1.1 cannot say *why* a subscription was refused, and MQTT 5 reports
/// transient reasons (`0x80`, `0x83`, `0x91`, `0x97`) in the same shape as
/// permanent ones, so a refusal is retried on a bounded, exponentially growing
/// backoff rather than on the fixed five-second cadence that hid issue #540.
pub(crate) const MAX_SUBSCRIBE_ATTEMPTS: u32 = 8;

/// Upper bound of the exponential backoff between those attempts. With the
/// default `reconnect_interval` (5 s) the eight attempts span about ten minutes.
pub(crate) const MAX_SUBSCRIBE_BACKOFF: Duration = Duration::from_secs(300);

/// How often to repeat, at `warn`, that an entry is still inactive after its
/// subscription was given up on, for as long as the connection stays up. The
/// one-time `error` at give-up is easy to miss in a log that has been rolling
/// for hours, and the peer's ACL or quota may long since have been fixed: the
/// entry comes back only when the bridge reconnects, so the reminder names
/// that, too.
pub(crate) const INACTIVE_REMINDER_INTERVAL: Duration = Duration::from_secs(300);

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
    pub root_cert: Option<String>,
    // #Client Certificate File
    pub client_cert: Option<String>,
    // #Client key file
    pub client_key: Option<String>,

    #[serde(default, deserialize_with = "deserialize_expand_env_vars_option")]
    pub username: Option<String>,
    #[serde(default, deserialize_with = "deserialize_expand_env_vars_option")]
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
        Protocol(MQTT_LEVEL_311)
    }

    fn expiry_interval_default() -> Duration {
        Duration::from_secs(300)
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
            0 => Err(serde::de::Error::custom(format!("invalid value, {addr:?}"))),
            1 => Ok(ServerAddr { typ: AddrType::Tcp, addr }),
            _ => {
                let typ = match addr_split[0].to_lowercase().as_str() {
                    "tcp" => AddrType::Tcp,
                    "tls" => AddrType::Tls,
                    _ => return Err(serde::de::Error::custom(format!("invalid value, {addr:?}"))),
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
    pub fn deserialize_qos<'de, D>(deserializer: D) -> std::result::Result<QoS, D::Error>
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
            TopicName::from(self.topic().replace(REMOTE_TOPIC_PLACEHOLDER, remote_topic))
        } else {
            TopicName::from(self.topic())
        }
    }

    /// The part of `local.topic` that is fixed at configuration time, with
    /// [`REMOTE_TOPIC_PLACEHOLDER`] reduced to a single legal level.
    ///
    /// This is what [`PluginConfig::validate`] checks: a wildcard outside the
    /// placeholder can never become a legal topic name, whatever the peer
    /// publishes, whereas the substituted value itself is only known at runtime.
    #[inline]
    pub fn topic_skeleton(&self) -> String {
        if self.topic_has_pattern() {
            self.topic().replace(REMOTE_TOPIC_PLACEHOLDER, PLACEHOLDER_STAND_IN)
        } else {
            self.topic().to_owned()
        }
    }

    #[inline]
    pub fn make_retain(&self, remote_retain: bool) -> bool {
        self.retain.unwrap_or(remote_retain)
    }

    #[inline]
    pub fn make_qos(&self, remote_qos: QoS) -> rmqtt::types::QoS {
        let qos = self.qos.unwrap_or(remote_qos);
        match qos {
            QoS::AtMostOnce => rmqtt::types::QoS::AtMostOnce,
            QoS::AtLeastOnce => rmqtt::types::QoS::AtLeastOnce,
            QoS::ExactlyOnce => rmqtt::types::QoS::ExactlyOnce,
        }
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
        let has_pattern = topic.contains(REMOTE_TOPIC_PLACEHOLDER);
        Ok((topic, has_pattern))
    }
}

/// The `${remote.topic}` placeholder as it appears in `local.topic`.
const REMOTE_TOPIC_PLACEHOLDER: &str = "${remote.topic}";

/// The level substituted for [`REMOTE_TOPIC_PLACEHOLDER`] when a mapping's
/// fixed part is checked: one legal level, which is what the placeholder
/// resolves to at runtime (a topic name taken off the wire).
const PLACEHOLDER_STAND_IN: &str = "x";

impl PluginConfig {
    /// Reject bridge mappings that can never produce a legal topic name.
    ///
    /// In an ingress entry `local.topic` is the topic the bridge publishes
    /// *into* the local broker — a topic NAME. The filter side of the same
    /// entry is `remote.topic`, which is what the bridge subscribes to on the
    /// peer. A wildcard that ends up in a name cannot be encoded on the wire,
    /// and a peer that checks names answers by dropping the connection, which
    /// is the reconnect-and-resend loop reported as issue #451.
    ///
    /// Only the part of the mapping that is fixed at configuration time is
    /// checked, with the placeholder reduced to a single stand-in level, so
    /// `a/#/${remote.topic}` is still rejected. Whether the substituted value
    /// itself is a legal name is a runtime concern, not a configuration one.
    ///
    /// Disabled bridges are skipped: they never connect, and failing them here
    /// would break configurations that are currently harmless.
    pub fn validate(&self) -> Result<()> {
        for (i, bridge) in self.bridges.iter().enumerate() {
            if !bridge.enable {
                continue;
            }
            for (j, entry) in bridge.entries.iter().enumerate() {
                if !rmqtt::topic::is_valid_topic_name(entry.local.topic_skeleton()) {
                    return Err(anyhow!(
                        "bridges[{i}].entries[{j}].local.topic = {:?} cannot produce a legal topic \
                         name: a topic name must not be empty, must not contain the wildcards `+` \
                         or `#` (MQTT 5.0 [MQTT-3.3.2-2]), and must be at most 65535 bytes long. \
                         `+` and `#` belong in `remote.topic`, which is a topic filter.",
                        entry.local.topic()
                    ));
                }

                // `remote.topic` is the filter this entry sends in SUBSCRIBE.
                // A peer answers a malformed filter either with a SUBACK failure
                // or by dropping the connection, and the bridge then retries on
                // a fixed cadence without ever naming the field — the entry
                // delivers nothing and the operator sees a healthy bridge
                // (issue #540). Refuse the configuration instead.
                if !rmqtt::topic::is_valid_subscription_filter(&entry.remote.topic) {
                    return Err(anyhow!(
                        "bridges[{i}].entries[{j}].remote.topic = {:?} is not a valid MQTT topic \
                         filter: a topic filter must be at least one character long, must be at \
                         most 65535 bytes, must not contain U+0000, `#` is legal only as the last \
                         level (MQTT 5.0 [MQTT-4.7.1-1]), and `$share/<group>/<filter>` needs a \
                         filter behind the group.",
                        entry.remote.topic
                    ));
                }
            }
        }
        Ok(())
    }
}

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
        Bytes::from(BASE64_STANDARD.decode(message).map_err(anyhow::Error::new)?)
    } else {
        Bytes::from(String::from(message))
    };
    Ok((qos, retain, ByteString::from(topic), message))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-entry config carrying both sides of the mapping explicitly.
    ///
    /// Keeping both settable matters: an entry with a legal `local.topic` but
    /// an empty `remote.topic` would otherwise be reported for the filter side,
    /// and vice versa.
    fn config_with(enable: bool, local_topic: &str, remote_topic: &str) -> PluginConfig {
        PluginConfig {
            bridges: vec![Bridge {
                enable,
                entries: vec![Entry {
                    remote: Remote { topic: remote_topic.to_owned(), ..Remote::default() },
                    local: Local {
                        topic: (local_topic.to_owned(), local_topic.contains(REMOTE_TOPIC_PLACEHOLDER)),
                        ..Local::default()
                    },
                }],
                ..Bridge::default()
            }],
        }
    }

    /// The name-side tests: the filter side is held legal on purpose.
    fn config(enable: bool, local_topic: &str) -> PluginConfig {
        config_with(enable, local_topic, "up/plain/#")
    }

    #[test]
    fn validate_accepts_legal_local_topics() {
        for topic in ["${remote.topic}", "${remote.topic}/suffix", "ing/plain", "ing/${remote.topic}"] {
            assert!(config(true, topic).validate().is_ok(), "`{topic}` must be accepted");
        }
    }

    #[test]
    fn validate_rejects_filter_shaped_local_topic() {
        let err = config(true, "ing/mirror/#").validate().unwrap_err().to_string();
        assert!(err.contains("bridges[0].entries[0].local.topic"), "{err}");
        assert!(err.contains("ing/mirror/#"), "{err}");
    }

    #[test]
    fn validate_rejects_a_wildcard_in_the_fixed_part() {
        assert!(config(true, "ing/#/${remote.topic}").validate().is_err());
        assert!(config(true, "ing/+/${remote.topic}").validate().is_err());
    }

    #[test]
    fn validate_skips_disabled_bridges() {
        assert!(config(false, "ing/mirror/#").validate().is_ok());
    }

    /// `remote.topic` is the filter sent in SUBSCRIBE: `#` is legal there (and
    /// only as the last level), and a shared subscription is a legal filter —
    /// including the empty ShareName that `parse_topic_filter` accepts.
    #[test]
    fn validate_accepts_legal_remote_topics() {
        for topic in [
            "a",
            "#",
            "+",
            "a/b/#",
            "up/plain/#",
            "up/+/#",
            "$share/g/up/a/#",
            "$share/g/#",
            // [MQTT-4.8.2-1] requires a non-empty ShareName, but the broker has
            // always taken an empty group as it stands; a check describing what a
            // peer will accept must not be stricter than the parser.
            "$share//up/a",
        ] {
            assert!(config_with(true, "ing/plain", topic).validate().is_ok(), "`{topic}` must be accepted");
        }
    }

    /// Issue #540: a malformed filter is refused while the config is loaded, and
    /// the message names the offending field (the `bridge_config_reject` suite
    /// matches on exactly that).
    #[test]
    fn validate_rejects_a_malformed_remote_topic() {
        for topic in ["", "up/#/x", "a#", "#/a", "a/#/", "$share/g", "$share/g/", "$share/+/a"] {
            let err = config_with(true, "ing/plain", topic).validate().unwrap_err().to_string();
            assert!(err.contains("bridges[0].entries[0].remote.topic"), "`{topic}`: {err}");
            assert!(err.contains("topic filter"), "`{topic}`: {err}");
        }
    }

    #[test]
    fn validate_skips_disabled_bridges_on_the_filter_side_too() {
        assert!(config_with(false, "ing/plain", "up/#/x").validate().is_ok());
    }
}
