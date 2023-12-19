use rmqtt::serde_json;

use rmqtt_storage::Config;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default)]
    pub storage: Config,

    #[serde(default = "PluginConfig::cleanup_count_default")]
    pub cleanup_count: usize,
}

impl PluginConfig {
    fn cleanup_count_default() -> usize {
        2000
    }

    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!(self)
    }
}
