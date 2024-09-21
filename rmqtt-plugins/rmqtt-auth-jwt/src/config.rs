use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use jsonwebtoken::DecodingKey;
use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{self, Serialize, Serializer};

use rmqtt::{ahash, anyhow, anyhow::anyhow, itertools::Itertools, serde_json, timestamp};
use rmqtt::{broker::hook::Priority, ConnectInfo, MqttError, Publish, QoS, Result, Subscribe};

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

#[derive(Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    ///Disconnect if publishing is rejected
    #[serde(default = "PluginConfig::disconnect_if_pub_rejected_default")]
    pub disconnect_if_pub_rejected: bool,

    ///Hook priority
    #[serde(default = "PluginConfig::priority_default")]
    pub priority: Priority,

    #[serde(default = "PluginConfig::from_default")]
    pub from: JWTFrom,

    #[serde(
        default = "PluginConfig::encrypt_default",
        serialize_with = "PluginConfig::serialize_encrypt",
        deserialize_with = "PluginConfig::deserialize_encrypt"
    )]
    pub encrypt: JWTEncrypt,

    #[serde(default)]
    pub hmac_secret: String,
    pub hmac_base64: bool,
    #[serde(default)]
    pub public_key: String,

    #[serde(default = "PluginConfig::disconnect_if_expiry_default")]
    pub disconnect_if_expiry: bool,

    #[serde(
        default,
        serialize_with = "PluginConfig::serialize_validate_claims",
        deserialize_with = "PluginConfig::deserialize_validate_claims"
    )]
    pub validate_claims: ValidateClaims,

    #[serde(skip, default = "PluginConfig::decoded_key_default")]
    pub decoded_key: DecodingKey,
}

impl fmt::Debug for PluginConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match serde_json::to_string(self) {
            Ok(cfg) => f.debug_set().entry(&cfg).finish(),
            Err(e) => f.debug_set().entry(&e).finish(),
        }
    }
}

impl PluginConfig {
    #[inline]
    pub(crate) fn init_decoding_key(&mut self) -> Result<()> {
        match &self.encrypt {
            JWTEncrypt::HmacBased => {
                self.decoded_key = if self.hmac_base64 {
                    DecodingKey::from_base64_secret(&self.hmac_secret).map_err(anyhow::Error::new)?
                } else {
                    DecodingKey::from_secret(self.hmac_secret.as_bytes())
                };
            }
            JWTEncrypt::PublicKey => {
                self.decoded_key = DecodingKey::from_rsa_pem(&std::fs::read(&self.public_key)?)
                    .map_err(anyhow::Error::new)?;
            }
        }
        Ok(())
    }

    fn decoded_key_default() -> DecodingKey {
        DecodingKey::from_secret(b"")
    }

    fn from_default() -> JWTFrom {
        JWTFrom::Password
    }

    fn encrypt_default() -> JWTEncrypt {
        JWTEncrypt::HmacBased
    }

    fn disconnect_if_pub_rejected_default() -> bool {
        true
    }

    fn priority_default() -> Priority {
        50
    }

    fn disconnect_if_expiry_default() -> bool {
        false
    }

    #[inline]
    fn serialize_encrypt<S>(enc: &JWTEncrypt, s: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        let enc = match enc {
            JWTEncrypt::HmacBased => "hmac-based",
            JWTEncrypt::PublicKey => "public-key",
        };
        enc.serialize(s)
    }

    #[inline]
    fn deserialize_encrypt<'de, D>(deserializer: D) -> std::result::Result<JWTEncrypt, D::Error>
    where
        D: Deserializer<'de>,
    {
        let enc: String = String::deserialize(deserializer)?;
        let enc = match enc.as_str() {
            "hmac-based" => JWTEncrypt::HmacBased,
            "public-key" => JWTEncrypt::PublicKey,
            _ => {
                return Err(de::Error::custom(
                    "Invalid encryption method, only 'hmac-based' and 'public-key' are supported.",
                ))
            }
        };
        Ok(enc)
    }

    #[inline]
    fn serialize_validate_claims<S>(claims: &ValidateClaims, s: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        claims.validate_customs.serialize(s)
    }

    #[inline]
    fn deserialize_validate_claims<'de, D>(deserializer: D) -> std::result::Result<ValidateClaims, D::Error>
    where
        D: Deserializer<'de>,
    {
        let claims_json: serde_json::Value =
            serde_json::Value::deserialize(deserializer).map_err(de::Error::custom)?;
        let mut validate_customs = HashMap::default();
        let mut validate_exp_enable = false;
        let mut validate_nbf_enable = false;
        // let mut validate_iat_enable = false;
        let mut validate_sub = None;
        let mut validate_iss = None;
        let mut validate_aud = None;
        if let Some(objs) = claims_json.as_object() {
            for (claim, val) in objs {
                let items = if let Some(arr) = val.as_array() {
                    arr.iter().map(|v| parse(claim.as_str(), v)).collect_vec()
                } else if val.as_str().is_some() {
                    vec![parse(claim.as_str(), val)]
                } else if let Some(true) = val.as_bool() {
                    vec![parse(claim.as_str(), val)]
                } else {
                    return Err(de::Error::custom(format!("invalid value, {}:{}", claim, val)));
                };
                for (exp_enable, nbf_enable, _) in items.iter() {
                    if *exp_enable && !validate_exp_enable {
                        validate_exp_enable = true;
                    } else if *nbf_enable && !validate_nbf_enable {
                        validate_nbf_enable = true;
                        // } else if *iat_enable && !validate_iat_enable {
                        //     validate_iat_enable = true;
                    }
                }
                let items = items.into_iter().filter_map(|(_, _, item)| item).collect_vec();
                if !items.is_empty() {
                    match claim.as_str() {
                        "sub" => validate_sub = Some(items[0].clone()),
                        "iss" => validate_iss = Some(items),
                        "aud" => validate_aud = Some(items),
                        _ => {
                            validate_customs.insert(claim.into(), items);
                        }
                    }
                }
            }
        }

        Ok(ValidateClaims {
            validate_customs,
            validate_exp_enable,
            validate_nbf_enable,
            validate_sub,
            validate_iss,
            validate_aud,
        })
    }

    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub(crate) enum JWTFrom {
    Username,
    Password,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum JWTEncrypt {
    HmacBased,
    PublicKey,
}

pub const PLACEHOLDER_USERNAME: &str = "${username}";
pub const PLACEHOLDER_CLIENTID: &str = "${clientid}";
pub const PLACEHOLDER_IPADDR: &str = "${ipaddr}";
pub const PLACEHOLDER_PROTOCOL: &str = "${protocol}";
type HasPlaceholderUsername = bool;
type HasPlaceholderClientid = bool;
type HasPlaceholderIpaddr = bool;
type HasPlaceholderProtocol = bool;
type ValidateExpEnable = bool;
type ValidateNbfEnable = bool;

type ClaimItem =
    (String, HasPlaceholderUsername, HasPlaceholderClientid, HasPlaceholderIpaddr, HasPlaceholderProtocol);

//"exp", "nbf", "aud", "iss", "sub"
#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub(crate) struct ValidateClaims {
    pub validate_customs: HashMap<String, Vec<ClaimItem>>,
    pub validate_exp_enable: ValidateExpEnable,
    pub validate_nbf_enable: ValidateNbfEnable,
    pub validate_sub: Option<ClaimItem>,
    pub validate_iss: Option<Vec<ClaimItem>>,
    pub validate_aud: Option<Vec<ClaimItem>>,
}

impl ValidateClaims {}

fn parse(name: &str, val: &serde_json::Value) -> (ValidateExpEnable, ValidateNbfEnable, Option<ClaimItem>) {
    match name {
        "exp" => {
            if val.as_bool().unwrap_or_default() {
                return (true, false, None);
            }
        }
        "nbf" => {
            if val.as_bool().unwrap_or_default() {
                return (false, true, None);
            }
        }
        _ => {
            if let Some(s) = val.as_str() {
                return (
                    false,
                    false,
                    Some((
                        s.into(),
                        s.contains(PLACEHOLDER_USERNAME),
                        s.contains(PLACEHOLDER_CLIENTID),
                        s.contains(PLACEHOLDER_IPADDR),
                        s.contains(PLACEHOLDER_PROTOCOL),
                    )),
                );
            }
        }
    }
    (false, false, None)
}

#[derive(Debug, Clone)]
pub(crate) struct Rule {
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
pub(crate) enum Permission {
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
pub(crate) enum Action {
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
pub(crate) struct Topic {
    pub eq_topic_filter: Option<String>,
    //"sensor/${clientid}/ctrl", "sensor/${username}/ctrl"
    pub topic_filter: Option<rmqtt::Topic>,
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
            topic_filter = Some(rmqtt::Topic::from_str(replaces(topic_cfg, connect_info)?.as_ref())?);
        } else {
            return Err(MqttError::from(format!("ACL Rule config error, topic config is {:?}", topic_cfg)));
        }

        Ok(Topic { eq_topic_filter, topic_filter })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AuthInfo {
    pub superuser: bool,
    pub exp: Option<Duration>,
    pub rules: Vec<Rule>,
}

impl AuthInfo {
    #[inline]
    pub fn is_expired(&self) -> bool {
        self.exp.map(|exp| exp < timestamp()).unwrap_or_default()
    }
}
