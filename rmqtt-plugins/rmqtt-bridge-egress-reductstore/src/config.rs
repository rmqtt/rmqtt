use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::bridge::BridgeName;

use rmqtt::utils::deserialize_duration_option;

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
    pub(crate) producer_name_prefix: Option<String>,

    #[serde(default)]
    pub(crate) api_token: Option<String>,
    #[serde(default)]
    pub(crate) verify_ssl: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_duration_option")]
    pub(crate) timeout: Option<Duration>,

    #[serde(default)]
    pub(crate) entries: Vec<Entry>,
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
    pub local: Local,

    #[serde(default)]
    pub remote: Remote,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Remote {
    pub bucket: String,
    pub entry: String,
    pub quota_size: Option<u64>,
    pub exist_ok: Option<bool>,
    #[serde(default)]
    pub forward_all_from: bool,
    #[serde(default)]
    pub forward_all_publish: bool,
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Local {
    #[serde(default)]
    pub topic_filter: String,
}
