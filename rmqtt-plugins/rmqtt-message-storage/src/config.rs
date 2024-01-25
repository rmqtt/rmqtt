use rmqtt::serde_json;
use rmqtt::settings::Bytesize;
use serde::de::{self, Deserialize, Deserializer};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default)]
    #[serde(deserialize_with = "PluginConfig::deserialize_storage")]
    pub storage: Config,
    #[serde(default = "PluginConfig::cleanup_count_default")]
    pub cleanup_count: usize,
}

impl PluginConfig {
    fn cleanup_count_default() -> usize {
        2000
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
                    Ok(ram) => Ok(Config::Ram(ram)),
                }
            }
            _ => match serde_json::from_value::<rmqtt_storage::Config>(storage) {
                Err(e) => Err(de::Error::custom(e.to_string())),
                Ok(s_cfg) => Ok(Config::Storage(s_cfg)),
            },
        }
    }

    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!(self)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum Config {
    Ram(RamConfig),
    Storage(rmqtt_storage::Config),
}

impl Default for Config {
    #[inline]
    fn default() -> Self {
        Config::Ram(RamConfig::default())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RamConfig {
    pub cache_capacity: Bytesize,
    pub cache_max_count: usize,
    pub encode: bool,
}

impl Default for RamConfig {
    #[inline]
    fn default() -> Self {
        RamConfig {
            cache_capacity: Bytesize::from(1024 * 1024 * 1024 * 2),
            cache_max_count: usize::MAX,
            encode: false,
        }
    }
}
