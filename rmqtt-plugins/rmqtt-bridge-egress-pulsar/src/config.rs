//! Configuration types for the Apache Pulsar egress bridge plugin.
//!
//! Defines bridge connection parameters (server, authentication, TLS),
//! entry routing with topic filters and remote Pulsar topic mapping,
//! and message options (compression, ordering key, partition key).

use std::collections::BTreeMap;

use pulsar::compression::{Compression, CompressionLz4, CompressionSnappy, CompressionZlib, CompressionZstd};
use rand::{distr::Uniform, rng, Rng};
use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use rmqtt::types::ClientId;

use crate::bridge::BridgeName;

/// Top-level plugin configuration containing a list of bridge definitions.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default)]
    pub bridges: Vec<Bridge>,
}

/// A Pulsar egress bridge definition.
///
/// Specifies connection parameters (server, authentication, TLS) and entries.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Bridge {
    #[serde(default)]
    pub enable: bool,
    #[serde(default)]
    pub name: BridgeName,
    pub servers: String,
    #[serde(default)]
    pub producer_name_prefix: Option<String>,

    #[serde(default)]
    pub auth: Auth,

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
    pub entries: Vec<Entry>,
}

impl Bridge {
    fn tls_hostname_verification_enabled_default() -> bool {
        true
    }
}

/// Pulsar authentication configuration (Token or OAuth2).
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Auth {
    pub name: Option<AuthName>,
    pub data: Option<String>,
}

/// Supported Pulsar authentication methods.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthName {
    Token,
    OAuth2,
}

/// Supported Pulsar compression codecs.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Comp {
    Lz4,
    Zlib,
    Zstd,
    Snappy,
}

impl Comp {
    /// Converts this enum to the Pulsar `Compression` type.
    pub fn to_pulsar_comp(&self) -> Compression {
        match self {
            Comp::Lz4 => Compression::Lz4(CompressionLz4::default()),
            Comp::Zlib => Compression::Zlib(CompressionZlib::default()),
            Comp::Zstd => Compression::Zstd(CompressionZstd::default()),
            Comp::Snappy => Compression::Snappy(CompressionSnappy::default()),
        }
    }
}

/// Producer options including metadata, compression, and access mode.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Opts {
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    pub compression: Option<Comp>,
    pub access_mode: Option<i32>,
}

/// A routing entry pairing a local topic filter with a remote Pulsar topic.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Entry {
    #[serde(default)]
    pub local: Local,

    #[serde(default)]
    pub remote: Remote,
}

/// Remote Pulsar topic configuration for a bridge entry.
///
/// Controls the target topic, metadata forwarding, ordering key,
/// partition key, schema version, compression, and skip levels.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Remote {
    pub topic: String,

    #[serde(default)]
    pub forward_all_from: bool,
    #[serde(default)]
    pub forward_all_publish: bool,

    pub partition_key: Option<String>,
    #[serde(
        default,
        deserialize_with = "Remote::deserialize_ordering_key",
        serialize_with = "Remote::serialize_ordering_key"
    )]
    pub ordering_key: Option<OrderingKey>,
    #[serde(default)]
    pub replicate_to: Vec<String>,
    #[serde(default, deserialize_with = "Remote::deserialize_string_bytes")]
    pub schema_version: Option<Vec<u8>>,
    #[serde(default)]
    pub options: Opts,
    #[serde(default)]
    pub skip_levels: usize,
}

impl Remote {
    /// Deserializes an ordering key from a string ("clientid", "uuid", "random")
    /// or an object with type and length.
    pub fn deserialize_ordering_key<'de, D>(deserializer: D) -> Result<Option<OrderingKey>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let json_cfg: serde_json::Value = serde_json::Value::deserialize(deserializer)?;
        log::debug!("json_cfg: {json_cfg:?}");
        let ordering_key = if let Some(key) = json_cfg.as_str() {
            let ordering_key = match key {
                "clientid" => OrderingKey::Clientid,
                "uuid" => OrderingKey::Uuid,
                "random" => {
                    let uniform = Uniform::new_inclusive(b'a', b'z').map_err(de::Error::custom)?;
                    OrderingKey::Random(uniform, ORDERINGKEY_RANDOM_DEF_LEN)
                }
                _ => return Err(de::Error::custom(format!("Invalid OrderingKey, {key:?}"))),
            };
            Some(ordering_key)
        } else if let Some(obj) = json_cfg.as_object() {
            let ordering_key = match obj.get("type").and_then(|t| t.as_str()) {
                Some("clientid") => OrderingKey::Clientid,
                Some("uuid") => OrderingKey::Uuid,
                Some("random") => {
                    let n = obj
                        .get("len")
                        .and_then(|l| l.as_u64().map(|l| l as u8))
                        .unwrap_or(ORDERINGKEY_RANDOM_DEF_LEN);
                    let uniform = Uniform::new_inclusive(b'a', b'z').map_err(de::Error::custom)?;
                    OrderingKey::Random(uniform, if n > 0 { n } else { ORDERINGKEY_RANDOM_DEF_LEN })
                }
                _ => return Err(de::Error::custom(format!("Invalid OrderingKey, {obj:?}"))),
            };
            Some(ordering_key)
        } else {
            None
        };
        Ok(ordering_key)
    }

    /// Serializes an ordering key as a string ("clientid", "uuid", "random", or empty).
    #[inline]
    pub fn serialize_ordering_key<S>(
        ordering_key: &Option<OrderingKey>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match ordering_key {
            Some(OrderingKey::Clientid) => "clientid",
            Some(OrderingKey::Uuid) => "uuid",
            Some(OrderingKey::Random(_, _)) => "random",
            None => "",
        }
        .serialize(serializer)
    }

    /// Deserializes a string into bytes for schema version.
    pub fn deserialize_string_bytes<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s: String = String::deserialize(deserializer)?;
        Ok(Some(s.into_bytes()))
    }
}

/// Local topic filter for a bridge entry.
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Local {
    #[serde(default)]
    pub topic_filter: String,
}

/// Ordering key strategy for Pulsar message ordering.
///
/// Supports client ID-based, UUID-based, and random key generation.
#[derive(Default, Debug, Clone)]
pub(crate) enum OrderingKey {
    #[default]
    Clientid,
    Random(Uniform<u8>, u8),
    Uuid,
}

impl OrderingKey {
    /// Generates an ordering key based on the configured strategy.
    #[inline]
    pub(crate) fn generate(&self, clientid: &ClientId) -> Vec<u8> {
        match self {
            OrderingKey::Clientid => clientid.as_bytes().to_vec(),
            OrderingKey::Random(uniform, n) => Self::gen_random_key(uniform, *n),
            OrderingKey::Uuid => {
                Uuid::new_v4().as_simple().encode_lower(&mut Uuid::encode_buffer()).as_bytes().to_vec()
            }
        }
    }

    #[inline]
    fn gen_random_key(uniform: &Uniform<u8>, n: u8) -> Vec<u8> {
        let mut rng = rng();
        (0..n).map(|_| rng.sample(uniform)).collect()
    }
}

const ORDERINGKEY_RANDOM_DEF_LEN: u8 = 10;
