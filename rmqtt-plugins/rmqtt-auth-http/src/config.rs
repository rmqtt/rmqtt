use std::str::FromStr;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use reqwest::{Method, Url};
use serde::de::{self, Deserialize, Deserializer};
use serde::ser::{self, Serialize};

use rmqtt::broker::hook::Priority;
use rmqtt::settings::deserialize_duration;
use rmqtt::Result;
use rmqtt::{ahash, reqwest, serde_json};

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    ///Stop the hook chain after successful authentication, including auth, pub-acl and sub-acl
    //    #[serde(default = "PluginConfig::break_if_allow_default")]
    //    pub break_if_allow: bool,

    ///Disconnect if publishing is rejected
    #[serde(default = "PluginConfig::disconnect_if_pub_rejected_default")]
    pub disconnect_if_pub_rejected: bool,

    ///Hook priority
    #[serde(default = "PluginConfig::priority_default")]
    pub priority: Priority,

    #[serde(default = "PluginConfig::http_timeout_default", deserialize_with = "deserialize_duration")]
    pub http_timeout: Duration,
    #[serde(
        default,
        serialize_with = "PluginConfig::serialize_http_headers",
        deserialize_with = "PluginConfig::deserialize_http_headers"
    )]
    pub http_headers: (HeaderMap, HashMap<String, String>),
    #[serde(default)]
    pub http_retry: Retry,

    pub http_auth_req: Option<Req>,
    pub http_super_req: Option<Req>,
    pub http_acl_req: Option<Req>,
}

impl PluginConfig {
    pub fn headers(&self) -> Option<&HeaderMap> {
        let (headers, _) = &self.http_headers;
        if !headers.is_empty() {
            Some(headers)
        } else {
            None
        }
    }

    //    fn break_if_allow_default() -> bool {
    //        true
    //    }
    fn disconnect_if_pub_rejected_default() -> bool {
        true
    }
    fn priority_default() -> Priority {
        100
    }
    fn http_timeout_default() -> Duration {
        Duration::from_secs(5)
    }

    #[inline]
    fn serialize_http_headers<S>(
        headers: &(HeaderMap, HashMap<String, String>),
        s: S,
    ) -> std::result::Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        let (_, headers) = headers;
        headers.serialize(s)
    }

    #[inline]
    pub fn deserialize_http_headers<'de, D>(
        deserializer: D,
    ) -> std::result::Result<(HeaderMap, HashMap<String, String>), D::Error>
    where
        D: Deserializer<'de>,
    {
        let hs: HashMap<String, String> = HashMap::deserialize(deserializer)?;
        let mut headers = HeaderMap::new();
        for (k, v) in &hs {
            let name = HeaderName::from_str(k).map_err(de::Error::custom)?;
            let value = HeaderValue::from_str(v).map_err(de::Error::custom)?;
            headers.insert(name, value);
        }
        Ok((headers, hs))
    }

    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
}

#[derive(Default, Debug, Clone, Deserialize, Serialize)]
pub struct Retry {
    #[serde(default = "Retry::times_default")]
    pub times: usize,
    //= 3
    #[serde(default = "Retry::interval_default", deserialize_with = "deserialize_duration")]
    pub interval: Duration,
    //= "1s"
    #[serde(default = "Retry::backoff_default")]
    pub backoff: f32, //= 2.0
}

impl Retry {
    fn times_default() -> usize {
        3
    }
    fn interval_default() -> Duration {
        Duration::from_secs(1)
    }
    fn backoff_default() -> f32 {
        2.0
    }
}

#[derive(Debug, Clone)]
pub enum ContentType {
    Json,
    Form,
}

type Headers = (Option<ContentType>, HeaderMap, HashMap<String, String>);

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Req {
    #[serde(serialize_with = "Req::serialize_url", deserialize_with = "Req::deserialize_url")]
    pub url: Url,
    #[serde(serialize_with = "Req::serialize_method", deserialize_with = "Req::deserialize_method")]
    pub method: Method,
    #[serde(
        default,
        serialize_with = "Req::serialize_headers",
        deserialize_with = "Req::deserialize_headers"
    )]
    pub headers: Headers,
    pub params: HashMap<String, String>,
}

impl Req {
    pub fn is_get(&self) -> bool {
        self.method == Method::GET
    }

    pub fn headers(&self) -> Option<&HeaderMap> {
        let (_, headers, _) = &self.headers;
        if !headers.is_empty() {
            Some(headers)
        } else {
            None
        }
    }

    pub fn json_body(&self) -> bool {
        matches!(self.headers, (Some(ContentType::Json), _, _))
    }

    #[inline]
    fn serialize_url<S>(url: &Url, s: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        url.to_string().serialize(s)
    }

    #[inline]
    pub fn deserialize_url<'de, D>(deserializer: D) -> std::result::Result<Url, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = String::deserialize(deserializer)?;
        Url::from_str(&v).map_err(de::Error::custom)
    }

    #[inline]
    fn serialize_method<S>(method: &Method, s: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        method.to_string().serialize(s)
    }

    #[inline]
    pub fn deserialize_method<'de, D>(deserializer: D) -> std::result::Result<Method, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = String::deserialize(deserializer)?.to_uppercase();
        Method::from_str(&v).map_err(de::Error::custom)
    }

    #[inline]
    fn serialize_headers<S>(headers: &Headers, s: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        let (_, _, headers) = headers;
        headers.serialize(s)
    }

    #[inline]
    pub fn deserialize_headers<'de, D>(deserializer: D) -> std::result::Result<Headers, D::Error>
    where
        D: Deserializer<'de>,
    {
        let hs: HashMap<String, String> = HashMap::deserialize(deserializer)?;
        let mut headers = HeaderMap::new();
        for (k, v) in &hs {
            let name = HeaderName::from_str(k).map_err(de::Error::custom)?;
            let value = HeaderValue::from_str(v).map_err(de::Error::custom)?;
            headers.insert(name, value);
        }
        let c_type = headers.remove(CONTENT_TYPE).and_then(|h| {
            let h = h.to_str().unwrap_or_default();
            if h.contains("application/x-www-form-urlencoded") {
                Some(ContentType::Form)
            } else if h.contains("application/json") {
                Some(ContentType::Json)
            } else {
                None
            }
        });
        Ok((c_type, headers, hs))
    }
}
