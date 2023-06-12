use std::convert::TryFrom;
use std::str::FromStr;
use std::sync::Arc;

use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{self, Serialize};

use rmqtt::broker::hook::Priority;
use rmqtt::broker::topic::TopicTree;
use rmqtt::grpc::MessageType;
use rmqtt::{
    ahash, dashmap, log,
    serde_json::{self, Value},
    tokio::sync::RwLock,
};
use rmqtt::{ClientId, ConnectInfo, MqttError, Password, Result, Superuser, Topic, UserName};

type DashSet<V> = dashmap::DashSet<V, ahash::RandomState>;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default = "PluginConfig::message_type_default")]
    pub message_type: MessageType,
}

impl PluginConfig {
    fn message_type_default() -> MessageType {
        69
    }

    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
}
