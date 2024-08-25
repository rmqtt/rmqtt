use std::collections::BTreeMap;

use pulsar::compression::{Compression, CompressionLz4, CompressionSnappy, CompressionZlib, CompressionZstd};
use serde::de::{Deserialize, Deserializer};

use rmqtt::{HashMap, Result};

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
    pub servers: String,
    #[serde(default)]
    pub producer_name_prefix: Option<String>,

    #[serde(default)]
    pub auth: Auth,

    #[serde(default)]
    pub properties: HashMap<String, String>,

    #[serde(default)]
    pub entries: Vec<Entry>,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum Comp {
    Lz4,
    Zlib,
    Zstd,
    Snappy,
}

impl Comp {
    pub fn to_pulsar_comp(&self) -> Compression {
        match self {
            Comp::Lz4 => Compression::Lz4(CompressionLz4::default()),
            Comp::Zlib => Compression::Zlib(CompressionZlib::default()),
            Comp::Zstd => Compression::Zstd(CompressionZstd::default()),
            Comp::Snappy => Compression::Snappy(CompressionSnappy::default()),
        }
    }
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Opts {
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    pub compression: Option<Comp>,
    pub access_mode: Option<i32>,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Entry {
    #[serde(default)]
    pub local: Local,

    #[serde(default)]
    pub remote: Remote,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Remote {
    pub topic: String,

    #[serde(default)]
    pub forward_all_from: bool,
    #[serde(default)]
    pub forward_all_publish: bool,

    pub partition_key: Option<String>,
    #[serde(default, deserialize_with = "Remote::deserialize_string_bytes")]
    pub ordering_key: Option<Vec<u8>>,
    #[serde(default)]
    pub replicate_to: Vec<String>,
    #[serde(default, deserialize_with = "Remote::deserialize_string_bytes")]
    pub schema_version: Option<Vec<u8>>,
    #[serde(default)]
    pub options: Opts,
}

impl Remote {
    pub fn deserialize_string_bytes<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let ordering_key: String = String::deserialize(deserializer)?;
        Ok(Some(ordering_key.into_bytes()))
    }
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Local {
    #[serde(default)]
    pub topic_filter: String,
}
