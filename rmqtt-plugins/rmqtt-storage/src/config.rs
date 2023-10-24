use serde::de::{Deserialize, Deserializer};
//use serde::ser::Serializer;

use rmqtt::serde_json;

use rmqtt::settings::Bytesize;
use rmqtt::{MqttError, Result};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default = "PluginConfig::storage_type_default")]
    pub storage_type: StorageType, // = "sled",
    #[serde(default)]
    pub sled: SledConfig,
}

impl PluginConfig {
    fn storage_type_default() -> StorageType {
        StorageType::Sled
    }

    #[inline]
    pub fn _to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SledConfig {
    #[serde(default = "SledConfig::path_default")]
    pub path: String,
    #[serde(default = "SledConfig::cache_capacity_default")]
    pub cache_capacity: Bytesize,
}

impl Default for SledConfig {
    fn default() -> Self {
        Self { path: Self::path_default(), cache_capacity: Self::cache_capacity_default() }
    }
}

impl SledConfig {
    fn path_default() -> String {
        "/var/log/rmqtt/.cache".into()
    }
    fn cache_capacity_default() -> Bytesize {
        Bytesize::from(1024 * 1024 * 1024) // 1gb
    }

    #[inline]
    pub fn to_sled_config(&self) -> Result<sled::Config> {
        if self.path.trim().is_empty() {
            return Err(MqttError::from("storage dir is empty"));
        }
        let sled_cfg = sled::Config::default()
            .path(self.path.trim())
            .cache_capacity(self.cache_capacity.as_u64())
            .mode(sled::Mode::HighThroughput);
        Ok(sled_cfg)
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum StorageType {
    //sled: high-performance embedded database with BTreeMap-like API for stateful systems.
    Sled,
    //redis:
    //Redis,
}

impl<'de> Deserialize<'de> for StorageType {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let t = match (String::deserialize(deserializer)?).to_ascii_lowercase().as_str() {
            "sled" => StorageType::Sled,
            //"redis" => StorageType::Redis,
            _ => StorageType::Sled,
        };
        Ok(t)
    }
}
