use std::fmt;

use jsonwebtoken::DecodingKey;
use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{self, Serialize, Serializer};

use rmqtt::{ahash, anyhow, itertools::Itertools, serde_json};
use rmqtt::{
    broker::hook::Priority,
    settings::acl::{PLACEHOLDER_CLIENTID, PLACEHOLDER_IPADDR, PLACEHOLDER_PROTOCOL, PLACEHOLDER_USERNAME},
    Result,
};

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

#[derive(Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    ///Disconnect if publishing is rejected
    #[serde(default = "PluginConfig::disconnect_if_pub_rejected_default")]
    pub disconnect_if_pub_rejected: bool,

    #[serde(default = "PluginConfig::disconnect_if_expiry_default")]
    pub disconnect_if_expiry: bool,

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
                self.decoded_key = if let Ok(key) =
                    DecodingKey::from_rsa_pem(&std::fs::read(&self.public_key)?)
                {
                    key
                } else if let Ok(key) = DecodingKey::from_ec_pem(&std::fs::read(&self.public_key)?) {
                    key
                } else {
                    DecodingKey::from_ed_pem(&std::fs::read(&self.public_key)?).map_err(anyhow::Error::new)?
                };
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
