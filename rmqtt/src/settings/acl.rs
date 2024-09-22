use std::borrow::Cow;
use std::str::FromStr;

use crate::{anyhow::anyhow, serde_json};
use crate::{ConnectInfo, MqttError, Publish, QoS, Result, Subscribe};

pub const PLACEHOLDER_USERNAME: &str = "${username}";
pub const PLACEHOLDER_CLIENTID: &str = "${clientid}";
pub const PLACEHOLDER_IPADDR: &str = "${ipaddr}";
pub const PLACEHOLDER_PROTOCOL: &str = "${protocol}";

#[derive(Debug, Clone)]
pub struct Rule {
    pub permission: Permission,
    pub action: Action,
    pub qos: Option<Vec<QoS>>,
    pub retain: Option<bool>,
    pub topic: Topic,
}

impl Rule {
    #[inline]
    pub async fn subscribe_hit(&self, subscribe: &Subscribe) -> bool {
        if !matches!(self.action, Action::Subscribe | Action::All) {
            return false;
        }

        if !self.qos.as_ref().map(|qos| qos.contains(&subscribe.opts.qos())).unwrap_or(true) {
            return false;
        }

        if !self.topic.is_match(&subscribe.topic_filter).await {
            return false;
        }

        true
    }

    #[inline]
    pub async fn publish_allow_hit(&self, publish: &Publish) -> bool {
        if let Some(retain) = self.retain {
            if !retain && publish.retain {
                return false;
            }
        }
        self.publish_hit(publish).await
    }

    #[inline]
    pub async fn publish_deny_hit(&self, publish: &Publish) -> bool {
        if let Some(retain) = self.retain {
            if retain != publish.retain {
                return false;
            }
        }
        self.publish_hit(publish).await
    }

    #[inline]
    async fn publish_hit(&self, publish: &Publish) -> bool {
        if !matches!(self.action, Action::Publish | Action::All) {
            return false;
        }

        if !self.qos.as_ref().map(|qos| qos.contains(&publish.qos)).unwrap_or(true) {
            return false;
        }

        if !self.topic.is_match(&publish.topic).await {
            return false;
        }

        true
    }
}

impl std::convert::TryFrom<(&serde_json::Value, &ConnectInfo)> for Rule {
    type Error = MqttError;
    #[inline]
    fn try_from((acl_cfg, connect_info): (&serde_json::Value, &ConnectInfo)) -> Result<Self, Self::Error> {
        let err_msg = format!("ACL Rule config error, rule config is {:?}", acl_cfg);

        if let Some(obj) = acl_cfg.as_object() {
            let permission = obj
                .get("permission")
                .and_then(|permi| permi.as_str().map(Permission::try_from))
                .ok_or_else(|| MqttError::from(err_msg.as_str()))??;
            let action = obj
                .get("action")
                .and_then(|action| action.as_str().map(Action::try_from))
                .ok_or_else(|| MqttError::from(err_msg.as_str()))??;
            let qos = obj
                .get("qos")
                .map(|qos| {
                    if let Some(qos) = qos.as_array() {
                        qos.iter()
                            .flat_map(|q| {
                                q.as_u64()
                                    .map(|q| QoS::try_from(q as u8).map_err(|e| MqttError::from(anyhow!(e))))
                                    .ok_or_else(|| MqttError::from("Unknown QoS"))
                            })
                            .collect::<Result<Vec<QoS>>>()
                    } else if let Some(qos) = qos.as_u64() {
                        match QoS::try_from(qos as u8) {
                            Ok(q) => Ok(vec![q]),
                            Err(e) => Err(MqttError::from(anyhow!(e))),
                        }
                    } else {
                        Err(MqttError::from("Unknown QoS"))
                    }
                })
                .transpose()?;
            let retain = obj.get("retain").and_then(|retain| retain.as_bool());
            let topic = obj
                .get("topic")
                .and_then(|topic| topic.as_str().map(|t| Topic::try_from((t, connect_info))))
                .ok_or_else(|| MqttError::from(err_msg.as_str()))??;

            Ok(Rule { permission, action, qos, retain, topic })
        } else {
            Err(MqttError::from(err_msg))
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum Permission {
    Allow,
    Deny,
}

impl std::convert::TryFrom<&str> for Permission {
    type Error = MqttError;
    #[inline]
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "allow" => Ok(Permission::Allow),
            "deny" => Ok(Permission::Deny),
            _ => Err(MqttError::from("Unknown Permission")),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Action {
    ///PUBLISH and SUBSCRIBE
    All,
    ///PUBLISH
    Publish,
    ///SUBSCRIBE
    Subscribe,
}

impl std::convert::TryFrom<&str> for Action {
    type Error = MqttError;
    #[inline]
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "all" => Ok(Action::All),
            "publish" => Ok(Action::Publish),
            "subscribe" => Ok(Action::Subscribe),
            _ => Err(MqttError::from("Unknown Action")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Topic {
    pub eq_topic_filter: Option<String>,
    //"sensor/${clientid}/ctrl", "sensor/${username}/ctrl"
    pub topic_filter: Option<crate::Topic>,
}

impl Topic {
    #[inline]
    pub async fn is_match(&self, topic: &str) -> bool {
        if let Some(eq_topic_filter) = &self.eq_topic_filter {
            return eq_topic_filter == topic;
        }
        if let Some(topic_filter) = &self.topic_filter {
            return topic_filter.matches_str(topic);
        }
        false
    }
}

#[inline]
fn replaces<'a>(topic_cfg: &'a str, connect_info: &ConnectInfo) -> Result<Cow<'a, str>> {
    let topic = match (topic_cfg.contains(PLACEHOLDER_USERNAME), topic_cfg.contains(PLACEHOLDER_CLIENTID)) {
        (true, true) => {
            if let Some(username) = connect_info.username() {
                Cow::Owned(
                    topic_cfg
                        .replace(PLACEHOLDER_USERNAME, username)
                        .replace(PLACEHOLDER_CLIENTID, connect_info.client_id()),
                )
            } else {
                return Err(MqttError::from("username does not exist"));
            }
        }
        (true, false) => {
            if let Some(username) = connect_info.username() {
                Cow::Owned(topic_cfg.replace(PLACEHOLDER_USERNAME, username))
            } else {
                return Err(MqttError::from("username does not exist"));
            }
        }
        (false, true) => Cow::Owned(topic_cfg.replace(PLACEHOLDER_CLIENTID, connect_info.client_id())),
        (false, false) => Cow::Borrowed(topic_cfg),
    };
    Ok(topic)
}

impl std::convert::TryFrom<(&str, &ConnectInfo)> for Topic {
    type Error = MqttError;
    #[inline]
    fn try_from((topic_cfg, connect_info): (&str, &ConnectInfo)) -> Result<Self, Self::Error> {
        let mut eq_topic_filter = None;
        let mut topic_filter = None;
        if let Some(stripped) = topic_cfg.strip_prefix("eq ") {
            eq_topic_filter = Some(replaces(stripped, connect_info)?.into());
        } else if !topic_cfg.is_empty() {
            topic_filter = Some(crate::Topic::from_str(replaces(topic_cfg, connect_info)?.as_ref())?);
        } else {
            return Err(MqttError::from(format!("ACL Rule config error, topic config is {:?}", topic_cfg)));
        }

        Ok(Topic { eq_topic_filter, topic_filter })
    }
}
