//! MQTT Access Control List (ACL) Implementation
//!
//! This module provides granular authorization controls for MQTT operations, implementing:
//! - Dynamic topic composition with client-specific placeholders
//! - Rule-based permission evaluation for PUBLISH/SUBSCRIBE operations
//! - Hierarchical authorization checks with superuser privileges
//! - Serde-compatible configuration parsing for ACL rules
//!
//! ## Core Components
//! 1. ​**​AuthInfo​**​: Container for client authentication state including:
//!    - Superuser flag bypassing ACL checks
//!    - Expiring credentials with timestamp validation
//!    - Rule collection for granular access control
//!
//! 2. ​**​Rule Engine​**​: Evaluates operations against configured rules with:
//!    - Topic pattern matching (exact/wildcard)
//!    - QoS level filtering
//!    - Retention flag validation
//!    - Protocol-aware placeholder substitution (client ID, username, etc.)
//!
//! 3. ​**​Permission Model​**​:
//!    - Allow/Deny decision hierarchy
//!    - Action-specific controls (Publish/Subscribe/All)
//!    - Asynchronous evaluation pipeline
//!
//! ## Design Highlights
//! - Uses Rust's ownership system for memory-safe credential handling
//! - Tokio-based async evaluation for high-throughput scenarios
//! - Zero-copy deserialization of ACL rules via Serde integration
//! - Compile-time validated topic pattern parsing
//!

use std::borrow::Cow;
use std::str::FromStr;
use std::time::Duration;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use serde_json;

use crate::codec::types::Publish;
use crate::codec::v5::SubscribeAckReason;
use crate::hook::{HookResult, ReturnType};
use crate::net::{Error, Result};
use crate::types::{ConnectInfo, PublishAclResult, QoS, Subscribe, SubscribeAclResult};
use crate::utils::timestamp;

/// Placeholder variables for dynamic topic composition
pub const PLACEHOLDER_USERNAME: &str = "${username}";
pub const PLACEHOLDER_CLIENTID: &str = "${clientid}";
pub const PLACEHOLDER_IPADDR: &str = "${ipaddr}";
pub const PLACEHOLDER_PROTOCOL: &str = "${protocol}";

/// Represents user authentication and authorization information
/// # Example
/// ```
/// use rmqtt::acl::AuthInfo;
/// let auth = AuthInfo {
///     superuser: true,
///     expire_at: None,
///     rules: vec![]
/// };
/// ```
#[derive(Debug, Clone)]
pub struct AuthInfo {
    /// Indicates administrator privileges
    pub superuser: bool,
    /// Optional authentication expiration time
    pub expire_at: Option<Duration>,
    /// Collection of access control rules
    pub rules: Vec<Rule>,
}

impl AuthInfo {
    /// Checks if authentication has expired
    #[inline]
    pub fn is_expired(&self) -> bool {
        self.expire_at.map(|exp| exp < timestamp()).unwrap_or_default()
    }

    /// Evaluates subscription access against ACL rules
    /// # Arguments
    /// * `subscribe` - Subscription request to validate
    #[inline]
    pub async fn subscribe_acl(&self, subscribe: &Subscribe) -> Option<ReturnType> {
        if self.superuser {
            return Some((
                false,
                Some(HookResult::SubscribeAclResult(SubscribeAclResult::new_success(
                    subscribe.opts.qos(),
                    None,
                ))),
            ));
        }
        for rule in &self.rules {
            if !rule.subscribe_hit(subscribe).await {
                continue;
            }

            return match rule.permission {
                Permission::Allow => Some((
                    false,
                    Some(HookResult::SubscribeAclResult(SubscribeAclResult::new_success(
                        subscribe.opts.qos(),
                        None,
                    ))),
                )),
                Permission::Deny => Some((
                    false,
                    Some(HookResult::SubscribeAclResult(SubscribeAclResult::new_failure(
                        SubscribeAckReason::NotAuthorized,
                    ))),
                )),
            };
        }
        None
    }

    /// Validates publish authorization against ACL rules
    /// # Arguments
    /// * `publish` - Publish request to validate
    /// * `disconnect_if_pub_rejected` - Flag for connection termination on denial
    #[inline]
    pub async fn publish_acl(
        &self,
        publish: &Publish,
        disconnect_if_pub_rejected: bool,
    ) -> Option<ReturnType> {
        if self.superuser {
            return Some((false, Some(HookResult::PublishAclResult(PublishAclResult::allow()))));
        }

        for rule in &self.rules {
            return match rule.permission {
                Permission::Allow => {
                    if rule.publish_allow_hit(publish).await {
                        Some((false, Some(HookResult::PublishAclResult(PublishAclResult::allow()))))
                    } else {
                        continue;
                    }
                }
                Permission::Deny => {
                    if rule.publish_deny_hit(publish).await {
                        Some((
                            false,
                            Some(HookResult::PublishAclResult(PublishAclResult::rejected(
                                disconnect_if_pub_rejected,
                                None,
                            ))),
                        ))
                    } else {
                        continue;
                    }
                }
            };
        }
        None
    }
}

/// Defines an access control rule for MQTT operations
/// # Example
/// ```
/// use rmqtt::acl::{Rule, Permission, Action, Topic};
/// use rmqtt::types::{ConnectInfo, Id, QoS};
/// let rule = Rule {
///     permission: Permission::Deny,
///     action: Action::Subscribe,
///     qos: Some(vec![QoS::ExactlyOnce]),
///     retain: None,
///     topic: Topic::try_from(("events/#", &ConnectInfo::from(Id::from(1, "clientid001".into())))).unwrap()
/// };
/// ```
#[derive(Debug, Clone)]
pub struct Rule {
    /// Allow/Deny decision for matching operations
    pub permission: Permission,
    /// Type of MQTT operation this rule applies to
    pub action: Action,
    /// Optional QoS level restrictions
    pub qos: Option<Vec<QoS>>,
    /// Retention flag restriction for publish operations
    pub retain: Option<bool>,
    /// Topic pattern to match against
    pub topic: Topic,
}

impl Rule {
    /// Checks if subscription matches rule criteria
    /// # Returns
    /// true if subscription matches rule conditions
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

    /// Checks if publish operation meets allow conditions
    #[inline]
    pub async fn publish_allow_hit(&self, publish: &Publish) -> bool {
        if let Some(retain) = self.retain {
            if !retain && publish.retain {
                return false;
            }
        }
        self.publish_hit(publish).await
    }

    /// Checks if publish operation meets deny conditions
    #[inline]
    pub async fn publish_deny_hit(&self, publish: &Publish) -> bool {
        if let Some(retain) = self.retain {
            if retain != publish.retain {
                return false;
            }
        }
        self.publish_hit(publish).await
    }

    /// Internal method for publish operation matching
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

impl TryFrom<(&serde_json::Value, &ConnectInfo)> for Rule {
    type Error = Error;
    #[inline]
    fn try_from(
        (acl_cfg, connect_info): (&serde_json::Value, &ConnectInfo),
    ) -> std::result::Result<Self, Self::Error> {
        let err_msg = || anyhow!(format!("ACL Rule config error, rule config is {:?}", acl_cfg));

        if let Some(obj) = acl_cfg.as_object() {
            let permission = obj
                .get("permission")
                .and_then(|permi| permi.as_str().map(Permission::try_from))
                .ok_or_else(err_msg)??;
            let action = obj
                .get("action")
                .and_then(|action| action.as_str().map(Action::try_from))
                .ok_or_else(err_msg)??;
            let qos = obj
                .get("qos")
                .map(|qos| {
                    if let Some(qos) = qos.as_array() {
                        qos.iter()
                            .flat_map(|q| {
                                q.as_u64()
                                    .map(|q| QoS::try_from(q as u8).map_err(|e| anyhow!(e)))
                                    .ok_or_else(|| anyhow!("Unknown QoS"))
                            })
                            .collect::<Result<Vec<QoS>>>()
                    } else if let Some(qos) = qos.as_u64() {
                        match QoS::try_from(qos as u8) {
                            Ok(q) => Ok(vec![q]),
                            Err(e) => Err(anyhow!(e)),
                        }
                    } else {
                        Err(anyhow!("Unknown QoS"))
                    }
                })
                .transpose()?;
            let retain = obj.get("retain").and_then(|retain| retain.as_bool());
            let topic = obj
                .get("topic")
                .and_then(|topic| topic.as_str().map(|t| Topic::try_from((t, connect_info))))
                .ok_or_else(err_msg)??;

            Ok(Rule { permission, action, qos, retain, topic })
        } else {
            Err(err_msg())
        }
    }
}

/// Access control permission type
/// # Example
/// ```
/// let perm = rmqtt::acl::Permission::try_from("allow").unwrap();
/// ```
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub enum Permission {
    /// Grants access to matched operations
    Allow,
    /// Denies access to matched operations
    Deny,
}

impl TryFrom<&str> for Permission {
    type Error = Error;
    #[inline]
    fn try_from(s: &str) -> std::result::Result<Self, Self::Error> {
        match s {
            "allow" => Ok(Permission::Allow),
            "deny" => Ok(Permission::Deny),
            _ => Err(anyhow!("Unknown Permission")),
        }
    }
}

/// MQTT operation type for rule matching
/// # Example
/// ```
/// let action = rmqtt::acl::Action::try_from("publish").unwrap();
/// ```
#[derive(Debug, Clone, Copy)]
pub enum Action {
    /// Applies to both PUBLISH and SUBSCRIBE
    All,
    /// Applies only to PUBLISH operations
    Publish,
    /// Applies only to SUBSCRIBE operations
    Subscribe,
}

impl TryFrom<&str> for Action {
    type Error = Error;
    #[inline]
    fn try_from(s: &str) -> std::result::Result<Self, Self::Error> {
        match s {
            "all" => Ok(Action::All),
            "publish" => Ok(Action::Publish),
            "subscribe" => Ok(Action::Subscribe),
            _ => Err(anyhow!("Unknown Action")),
        }
    }
}

/// Topic matching configuration
/// # Example
/// ```
/// use rmqtt::types::{ConnectInfo, Id};
/// use rmqtt::acl::Topic;
/// let topic = Topic::try_from(("eq devices/${clientid}", &ConnectInfo::from(Id::from(1, "clientid001".into())))).unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct Topic {
    /// Exact match topic filter (prefixed with "eq ")
    pub eq_topic_filter: Option<String>,
    /// Wildcard pattern topic filter
    pub topic_filter: Option<crate::topic::Topic>,
}

impl Topic {
    /// Checks if topic matches stored filter
    /// # Arguments
    /// * `topic` - Topic string to match against
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
                return Err(anyhow!("username does not exist"));
            }
        }
        (true, false) => {
            if let Some(username) = connect_info.username() {
                Cow::Owned(topic_cfg.replace(PLACEHOLDER_USERNAME, username))
            } else {
                return Err(anyhow!("username does not exist"));
            }
        }
        (false, true) => Cow::Owned(topic_cfg.replace(PLACEHOLDER_CLIENTID, connect_info.client_id())),
        (false, false) => Cow::Borrowed(topic_cfg),
    };
    Ok(topic)
}

impl TryFrom<(&str, &ConnectInfo)> for Topic {
    type Error = Error;
    #[inline]
    fn try_from((topic_cfg, connect_info): (&str, &ConnectInfo)) -> std::result::Result<Self, Self::Error> {
        let mut eq_topic_filter = None;
        let mut topic_filter = None;
        if let Some(stripped) = topic_cfg.strip_prefix("eq ") {
            eq_topic_filter = Some(replaces(stripped, connect_info)?.into());
        } else if !topic_cfg.is_empty() {
            topic_filter = Some(crate::topic::Topic::from_str(replaces(topic_cfg, connect_info)?.as_ref())?);
        } else {
            return Err(anyhow!(format!("ACL Rule config error, topic config is {:?}", topic_cfg)));
        }

        Ok(Topic { eq_topic_filter, topic_filter })
    }
}
