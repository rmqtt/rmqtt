use std::convert::TryFrom;
use std::str::FromStr;
use std::sync::Arc;

use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{self, Serialize};

use rmqtt::broker::hook::Priority;
use rmqtt::broker::topic::TopicTree;
use rmqtt::{
    ahash, dashmap, log,
    serde_json::{self, Value},
    tokio::sync::RwLock,
    Id,
};
use rmqtt::{ClientId, MqttError, Password, Result, Superuser, Topic, UserName};

type DashSet<V> = dashmap::DashSet<V, ahash::RandomState>;

pub const PH_C: &str = "%c";
pub const PH_U: &str = "%u";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    ///Disconnect if publishing is rejected
    #[serde(default = "PluginConfig::disconnect_if_pub_rejected_default")]
    pub disconnect_if_pub_rejected: bool,

    ///Hook priority
    #[serde(default = "PluginConfig::priority_default")]
    pub priority: Priority,

    #[serde(
        default,
        serialize_with = "PluginConfig::serialize_rules",
        deserialize_with = "PluginConfig::deserialize_rules"
    )]
    rules: (Vec<Rule>, serde_json::Value),
}

impl PluginConfig {
    fn disconnect_if_pub_rejected_default() -> bool {
        true
    }

    fn priority_default() -> Priority {
        10
    }

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
            let access_cfg = cfg_items.first().ok_or_else(|| MqttError::from(err_msg.as_str()))?;
            let user_cfg = cfg_items.get(1).ok_or_else(|| MqttError::from(err_msg))?;
            let control_cfg = cfg_items.get(2);
            let topics_cfg = cfg_items.get(3);

            let access = Access::try_from(access_cfg)?;
            let user = User::try_from((user_cfg, access))?;
            let control = Control::try_from(control_cfg)?;
            let topics = Topics::try_from(topics_cfg)?;
            if topics_cfg.is_some() && matches!(control, Control::Connect) {
                log::warn!("ACL Rule config, the third column of a quadruple is Connect, but the fourth column is not empty! topics config is {:?}", topics_cfg);
            }
            Ok(Rule { access, user, control, topics })
        } else {
            Err(MqttError::from(err_msg))
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Access {
    Allow,
    Deny,
}

#[derive(Debug, Clone)]
pub enum User {
    Username(UserName, Option<Password>, Superuser),
    Clientid(ClientId),
    Ipaddr(String),
    All,
}

impl User {
    #[inline]
    pub fn hit(&self, id: &Id, password: Option<&Password>, allow: bool) -> (bool, Superuser) {
        match self {
            User::All => (true, false),
            User::Username(name1, password1, superuser) => {
                match (id.username.as_ref(), password, password1, allow) {
                    (Some(name2), Some(password2), Some(password1), true) => {
                        (name1 == name2 && password1 == password2, *superuser)
                    }
                    (Some(name2), Some(_), &Some(_), false) => (name1 == name2, false),
                    (Some(name2), _, None, true) => (name1 == name2, *superuser),
                    (Some(name2), _, None, false) => (name1 == name2, false),
                    (Some(_), None, Some(_), _) => (false, false),
                    (None, _, _, _) => (false, false),
                }
            }
            User::Clientid(clientid) => (id.client_id == clientid, false),
            User::Ipaddr(ipaddr) => {
                if let Some(remote_addr) = id.remote_addr {
                    (ipaddr == remote_addr.ip().to_string().as_str(), false) //@TODO Consider using integer representation of IP addresses
                } else {
                    (false, false)
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
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
    pub all: bool,
    pub eqs: Arc<DashSet<String>>,
    pub eq_placeholders: Vec<String>,
    //"sensor/%u/ctrl", "sensor/%c/ctrl"
    pub tree: Arc<RwLock<TopicTree<()>>>,
    pub placeholders: Vec<String>, //"sensor/%u/ctrl", "sensor/%c/ctrl"
}

impl Topics {
    pub async fn is_match(&self, topic_filter: &Topic, topic_filter_str: &str) -> bool {
        if self.all {
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

impl std::convert::TryFrom<(&serde_json::Value, Access)> for User {
    type Error = MqttError;
    #[inline]
    fn try_from(user_cfg_access: (&serde_json::Value, Access)) -> Result<Self, Self::Error> {
        let (user_cfg, access) = user_cfg_access;
        let err_msg = format!("ACL Rule config error, user config is {:?}", user_cfg);
        let user = match user_cfg {
            Value::String(all) => {
                if all.to_lowercase() == "all" {
                    Ok(User::All)
                } else {
                    Err(MqttError::from(err_msg))
                }
            }
            Value::Object(map) => {
                match (
                    access,
                    map.get("user"),
                    map.get("password"),
                    map.get("superuser"),
                    map.get("clientid"),
                    map.get("ipaddr"),
                ) {
                    (Access::Allow, Some(Value::String(name)), password, superuser, _, _) => {
                        if name.is_empty() {
                            Err(MqttError::from(err_msg))
                        } else {
                            let password = match password {
                                Some(Value::String(p)) => Some(Password::from(p.to_owned())),
                                None => None,
                                _ => return Err(MqttError::from(err_msg)),
                            };
                            let superuser = superuser.and_then(|s| s.as_bool()).unwrap_or_default();
                            Ok(User::Username(UserName::from(name.as_str()), password, superuser))
                        }
                    }
                    (Access::Deny, Some(Value::String(name)), None, None, _, _) => {
                        if name.is_empty() {
                            Err(MqttError::from(err_msg))
                        } else {
                            Ok(User::Username(UserName::from(name.as_str()), None, false))
                        }
                    }
                    (_, _, _, _, Some(Value::String(clientid)), _) => {
                        if clientid.is_empty() {
                            Err(MqttError::from(err_msg))
                        } else {
                            Ok(User::Clientid(ClientId::from(clientid.as_str())))
                        }
                    }
                    (_, _, _, _, _, Some(Value::String(ipaddr))) => {
                        if ipaddr.is_empty() {
                            Err(MqttError::from(err_msg))
                        } else {
                            Ok(User::Ipaddr(ipaddr.clone()))
                        }
                    }
                    _ => Err(MqttError::from(err_msg)),
                }
            }
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
        let mut all = false;
        let eqs = DashSet::default();
        let mut tree = TopicTree::default();
        let mut placeholders = Vec::new();
        let mut eq_placeholders = Vec::new();
        match topics_cfg {
            None => all = true,
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
            all,
            eqs: Arc::new(eqs),
            eq_placeholders,
            tree: Arc::new(RwLock::new(tree)),
            placeholders,
        })
    }
}
