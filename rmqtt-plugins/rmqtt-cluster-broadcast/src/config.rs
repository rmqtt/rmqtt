use rmqtt::grpc::MessageType;
use rmqtt::serde_json;
use rmqtt::settings::NodeAddr;
use rmqtt::Result;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(default = "PluginConfig::message_type_default")]
    pub message_type: MessageType,

    pub node_grpc_addrs: Vec<NodeAddr>,
}

impl PluginConfig {
    fn message_type_default() -> MessageType {
        98
    }

    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
}
