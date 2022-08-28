use std::convert::TryFrom;
use std::str::FromStr;
use std::sync::Arc;

use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{self, Serialize};

use rmqtt::{ClientId, ConnectInfo, MqttError, Result, Topic, UserName};
use rmqtt::{ahash, dashmap, serde_json::{self, Value}, tokio::sync::RwLock};
use rmqtt::broker::topic::TopicTree;

type DashSet<V> = dashmap::DashSet<V, ahash::RandomState>;

pub const PH_C: &str = "%c";
pub const PH_U: &str = "%u";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    #[serde(
    default,
    serialize_with = "PluginConfig::serialize_rules",
    deserialize_with = "PluginConfig::deserialize_rules"
    )]
    rules: (Vec<Rule>, serde_json::Value),
}

impl PluginConfig {
    #[inline]
    pub fn rules(&self) -> &Vec<Rule> {
        let (_rules, _) = &self.rules;
        _rules
    }

    #[inline]
    fn serialize_rules<S>(
        rules: &(Vec<Rule>, serde_json::Value),
        s: S,
    ) -> std::result::Result<S::Ok, S::Error>
        where
            S: ser::Serializer,
    {
        let (_, rules) = rules;
        rules.serialize(s)
    }

    #[inline]
    pub fn deserialize_rules<'de, D>(
        deserializer: D,
    ) -> std::result::Result<(Vec<Rule>, serde_json::Value), D::Error>
        where
            D: Deserializer<'de>,
    {
        let json_rules = serde_json::Value::deserialize(deserializer)?;
        let mut rules = Vec::new();
        if let Some(rules_cfg) = json_rules.as_array() {
            for rule_cfg in rules_cfg {
                let r = Rule::try_from(rule_cfg).map_err(de::Error::custom)?;
                rules.push(r);
            }
        }
        Ok((rules, json_rules))
    }

    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub access: Access,
    pub user: User,
    pub control: Control,
    pub topics: Topics,
}

impl Rule {
    #[inline]
    pub async fn add_topic_filter(&self, topic_filter: &str) -> Result<()> {
        let t = Topic::from_str(topic_filter)?;
        self.topics.tree.write().await.insert(&t, ());
        Ok(())
    }

    #[inline]
    pub fn add_topic_to_eqs(&self, topic: String) {
        self.topics.eqs.insert(topic);
    }
}

impl std::convert::TryFrom<&serde_json::Value> for Rule {
    type Error = MqttError;
    #[inline]
    fn try_from(rule_cfg: &serde_json::Value) -> Result<Self, Self::Error> {
        let err_msg = format!("ACL Rule config error, rule config is {:?}", rule_cfg);
        if let Some(cfg_items) = rule_cfg.as_array() {
            let access_cfg = cfg_items.get(0).ok_or_else(|| MqttError::from(err_msg.as_str()))?;
            let user_cfg = cfg_items.get(1).ok_or_else(|| MqttError::from(err_msg))?;
            let control_cfg = cfg_items.get(2);
            let topics_cfg = cfg_items.get(3);
            let r = Rule {
                access: Access::try_from(access_cfg)?,
                user: User::try_from(user_cfg)?,
                control: Control::try_from(control_cfg)?,
                topics: Topics::try_from(topics_cfg)?,
            };
            Ok(r)
        } else {
            Err(MqttError::from(err_msg))
        }
    }
}

#[derive(Debug, Clone)]
pub enum Access {
    Allow,
    Deny,
}

#[derive(Debug, Clone)]
pub enum User {
    Username(UserName),
    Clientid(ClientId),
    Ipaddr(String),
    All,
}

impl User {
    #[inline]
    pub fn hit(&self, connect_info: &ConnectInfo) -> bool {
        match self {
            User::All => true,
            User::Username(name1) => {
                if let Some(name2) = connect_info.username() {
                    name1 == name2
                } else {
                    false
                }
            }
            User::Clientid(clientid) => connect_info.client_id() == clientid,
            User::Ipaddr(ipaddr) => {
                if let Some(remote_addr) = connect_info.id().remote_addr {
                    ipaddr == remote_addr.ip().to_string().as_str()
                } else {
                    false
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum Control {
    ///ALL
    All,
    ///CONNECT
    Connect,
    ///PUBLISH
    Publish,
    ///SUBSCRIBE
    Subscribe,
    ///PUBLISH and SUBSCRIBE
    Pubsub,
}

#[derive(Debug, Clone)]
pub struct Topics {
    pub allow_all: bool,
    pub eqs: Arc<DashSet<String>>,
    pub eq_placeholders: Vec<String>,
    //"sensor/%u/ctrl", "sensor/%c/ctrl"
    pub tree: Arc<RwLock<TopicTree<()>>>,
    pub placeholders: Vec<String>, //"sensor/%u/ctrl", "sensor/%c/ctrl"
}

impl Topics {
    pub async fn is_match(&self, topic_filter: &Topic, topic_filter_str: &str) -> bool {
        if self.allow_all {
            return true;
        }
        if self.eqs.contains(topic_filter_str) {
            return true;
        }
        self.tree.read().await.is_match(topic_filter)
    }
}

impl std::convert::TryFrom<&serde_json::Value> for Access {
    type Error = MqttError;
    #[inline]
    fn try_from(access_cfg: &serde_json::Value) -> Result<Self, Self::Error> {
        let err_msg = format!("ACL Rule config error, access config is {:?}", access_cfg);
        match access_cfg.as_str().ok_or_else(|| MqttError::from(err_msg.as_str()))?.to_lowercase().as_str() {
            "allow" => Ok(Access::Allow),
            "deny" => Ok(Access::Deny),
            _ => Err(MqttError::from(err_msg)),
        }
    }
}

impl std::convert::TryFrom<&serde_json::Value> for User {
    type Error = MqttError;
    #[inline]
    fn try_from(user_cfg: &serde_json::Value) -> Result<Self, Self::Error> {
        let err_msg = format!("ACL Rule config error, user config is {:?}", user_cfg);
        let user = match user_cfg {
            Value::String(all) => {
                if all.to_lowercase() == "all" {
                    Ok(User::All)
                } else {
                    Err(MqttError::from(err_msg))
                }
            }
            Value::Object(map) => match (map.get("user"), map.get("clientid"), map.get("ipaddr")) {
                (Some(Value::String(name)), _, _) => Ok(User::Username(UserName::from(name.as_str()))),
                (_, Some(Value::String(clientid)), _) => {
                    Ok(User::Clientid(ClientId::from(clientid.as_str())))
                }
                (_, _, Some(Value::String(ipaddr))) => Ok(User::Ipaddr(ipaddr.clone())),
                _ => Err(MqttError::from(err_msg)),
            },
            _ => Err(MqttError::from(err_msg)),
        };
        user
    }
}

impl std::convert::TryFrom<Option<&serde_json::Value>> for Control {
    type Error = MqttError;
    #[inline]
    fn try_from(control_cfg: Option<&serde_json::Value>) -> Result<Self, Self::Error> {
        let err_msg = format!("ACL Rule config error, control config is {:?}", control_cfg);
        let control = match control_cfg {
            None => Ok(Control::All),
            Some(Value::String(control)) => match control.to_lowercase().as_str() {
                "connect" => Ok(Control::Connect),
                "publish" => Ok(Control::Publish),
                "subscribe" => Ok(Control::Subscribe),
                "pubsub" => Ok(Control::Pubsub),
                "all" => Ok(Control::All),
                _ => Err(MqttError::from(err_msg)),
            },
            _ => Err(MqttError::from(err_msg)),
        };
        control
    }
}

impl std::convert::TryFrom<Option<&serde_json::Value>> for Topics {
    type Error = MqttError;
    #[inline]
    fn try_from(topics_cfg: Option<&serde_json::Value>) -> Result<Self, Self::Error> {
        let err_msg = format!("ACL Rule config error, topics config is {:?}", topics_cfg);
        let mut allow_all = false;
        let eqs = DashSet::default();
        let mut tree = TopicTree::default();
        let mut placeholders = Vec::new();
        let mut eq_placeholders = Vec::new();
        match topics_cfg {
            None => allow_all = true,
            Some(Value::Array(topics)) => {
                for topic in topics.iter() {
                    match topic {
                        Value::String(topic) => {
                            if topic.contains(PH_U) || topic.contains(PH_C) {
                                placeholders.push(topic.clone());
                            } else {
                                tree.insert(&Topic::from_str(topic.as_str())?, ());
                            }
                        }
                        Value::Object(eq_map) => match eq_map.get("eq") {
                            Some(Value::String(eq)) => {
                                if eq.contains(PH_U) || eq.contains(PH_C) {
                                    eq_placeholders.push(eq.clone());
                                } else {
                                    eqs.insert(eq.clone());
                                }
                            }
                            _ => return Err(MqttError::from(err_msg)),
                        },
                        _ => return Err(MqttError::from(err_msg)),
                    }
                }
            }
            _ => return Err(MqttError::from(err_msg)),
        }
        Ok(Topics {
            allow_all,
            eqs: Arc::new(eqs),
            eq_placeholders,
            tree: Arc::new(RwLock::new(tree)),
            placeholders,
        })
    }
}
