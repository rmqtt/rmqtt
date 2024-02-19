#![deny(unsafe_code)]
#[macro_use]
extern crate serde;

#[macro_use]
extern crate rmqtt_macros;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use reqwest::{Method, Url};
use serde::ser::Serialize;
use tokio::sync::RwLock;

use config::PluginConfig;
use rmqtt::ntex::util::ByteString;
use rmqtt::reqwest::Response;
use rmqtt::{ahash, async_trait, chrono, log, once_cell::sync::Lazy, reqwest, serde_json, tokio, Id};
use rmqtt::{
    broker::hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    broker::types::{
        AuthResult, Password, PublishAclResult, SubscribeAckReason, SubscribeAclResult, Superuser,
    },
    plugin::{DynPlugin, DynPluginResult, PackageInfo, Plugin},
    MqttError, Result, Runtime, TopicName,
};

mod config;

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

const CACHEABLE: &str = "X-Cache";
const SUPERUSER: &str = "X-Superuser";

const CACHE_KEY: &str = "ACL-CACHE-MAP";

#[derive(Clone, Debug, PartialEq, Eq, Hash, Copy)]
enum ResponseResult {
    Allow(Superuser),
    Deny,
    Ignore,
}

impl ResponseResult {
    #[inline]
    fn from(s: &str, superuser: Superuser) -> Self {
        match s {
            "allow" => ResponseResult::Allow(superuser),
            "deny" => ResponseResult::Deny,
            "ignore" => ResponseResult::Ignore,
            _ => ResponseResult::Allow(superuser),
        }
    }
}

type Cacheable = Option<i64>;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Copy)]
enum ACLType {
    Sub = 1,
    Pub = 2,
}

impl ACLType {
    fn as_str(&self) -> &str {
        match self {
            Self::Sub => "1",
            Self::Pub => "2",
        }
    }
}

#[inline]
pub async fn register(
    runtime: &'static Runtime,
    name: &'static str,
    default_startup: bool,
    immutable: bool,
) -> Result<()> {
    runtime
        .plugins
        .register(name, default_startup, immutable, move || -> DynPluginResult {
            Box::pin(
                async move { AuthHttpPlugin::new(runtime, name).await.map(|p| -> DynPlugin { Box::new(p) }) },
            )
        })
        .await?;
    Ok(())
}

#[derive(Plugin)]
struct AuthHttpPlugin {
    runtime: &'static Runtime,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
}

impl AuthHttpPlugin {
    #[inline]
    async fn new<S: Into<String>>(runtime: &'static Runtime, name: S) -> Result<Self> {
        let name = name.into();
        let cfg = Arc::new(RwLock::new(runtime.settings.plugins.load_config::<PluginConfig>(&name)?));
        log::debug!("{} AuthHttpPlugin cfg: {:?}", name, cfg.read().await);
        let register = runtime.extends.hook_mgr().await.register();
        Ok(Self { runtime, register, cfg })
    }
}

#[async_trait]
impl Plugin for AuthHttpPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        let cfg = &self.cfg;

        let priority = cfg.read().await.priority;
        self.register.add_priority(Type::ClientAuthenticate, priority, Box::new(AuthHandler::new(cfg))).await;
        self.register
            .add_priority(Type::ClientSubscribeCheckAcl, priority, Box::new(AuthHandler::new(cfg)))
            .await;
        self.register
            .add_priority(Type::MessagePublishCheckAcl, priority, Box::new(AuthHandler::new(cfg)))
            .await;

        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.runtime.settings.plugins.load_config::<PluginConfig>(self.name())?;
        *self.cfg.write().await = new_cfg;
        log::debug!("load_config ok,  {:?}", self.cfg);
        Ok(())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name());
        self.register.start().await;
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::info!("{} stop", self.name());
        self.register.stop().await;
        Ok(true)
    }

    #[inline]
    async fn attrs(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

struct AuthHandler {
    cfg: Arc<RwLock<PluginConfig>>,
}

impl AuthHandler {
    fn new(cfg: &Arc<RwLock<PluginConfig>>) -> Self {
        Self { cfg: cfg.clone() }
    }

    async fn response_result(resp: Response) -> Result<(ResponseResult, Superuser, Cacheable)> {
        if resp.status().is_success() {
            let superuser = resp.headers().contains_key(SUPERUSER);
            let cache_timeout = if let Some(tm) = resp.headers().get(CACHEABLE).and_then(|v| v.to_str().ok())
            {
                match tm.parse::<i64>() {
                    Ok(tm) => Some(tm),
                    Err(e) => {
                        log::warn!("Parse X-Cache error, {:?}", e);
                        None
                    }
                }
            } else {
                None
            };
            log::debug!("Cache timeout is {:?}", cache_timeout);
            let body = resp.text().await.map_err(|e| MqttError::Msg(e.to_string()))?;
            Ok((ResponseResult::from(body.as_str(), superuser), superuser, cache_timeout))
        } else {
            Ok((ResponseResult::Ignore, false, None))
        }
    }

    async fn http_get_request<T: Serialize + ?Sized>(
        url: Url,
        body: &T,
        headers: HeaderMap,
        timeout: Duration,
    ) -> Result<(ResponseResult, Superuser, Cacheable)> {
        log::debug!("http_get_request, timeout: {:?}, url: {}", timeout, url);
        match HTTP_CLIENT.clone().get(url).headers(headers).timeout(timeout).query(body).send().await {
            Err(e) => {
                log::error!("error:{:?}", e);
                Err(MqttError::Msg(e.to_string()))
            }
            Ok(resp) => Self::response_result(resp).await,
        }
    }

    async fn http_form_request<T: Serialize + ?Sized>(
        url: Url,
        method: Method,
        body: &T,
        headers: HeaderMap,
        timeout: Duration,
    ) -> Result<(ResponseResult, Superuser, Cacheable)> {
        log::debug!("http_form_request, method: {:?}, timeout: {:?}, url: {}", method, timeout, url);
        match HTTP_CLIENT
            .clone()
            .request(method, url)
            .headers(headers)
            .timeout(timeout)
            .form(body)
            .send()
            .await
        {
            Err(e) => {
                log::error!("error:{:?}", e);
                Err(MqttError::Msg(e.to_string()))
            }
            Ok(resp) => Self::response_result(resp).await,
        }
    }

    async fn http_json_request<T: Serialize + ?Sized>(
        url: Url,
        method: Method,
        body: &T,
        headers: HeaderMap,
        timeout: Duration,
    ) -> Result<(ResponseResult, Superuser, Cacheable)> {
        log::debug!("http_json_request, method: {:?}, timeout: {:?}, url: {}", method, timeout, url);
        match HTTP_CLIENT
            .clone()
            .request(method, url)
            .headers(headers)
            .timeout(timeout)
            .json(body)
            .send()
            .await
        {
            Err(e) => {
                log::error!("error:{:?}", e);
                Err(MqttError::Msg(e.to_string()))
            }
            Ok(resp) => Self::response_result(resp).await,
        }
    }

    fn replaces(
        params: &mut HashMap<String, String>,
        id: &Id,
        password: Option<&Password>,
        sub_or_pub: Option<(ACLType, &TopicName)>,
    ) -> Result<()> {
        let password =
            if let Some(p) = password { ByteString::try_from(p.clone())? } else { ByteString::default() };
        let client_id = id.client_id.as_ref();
        let username = id.username.as_ref().map(|n| n.as_ref()).unwrap_or("");
        let remote_addr = id.remote_addr.map(|addr| addr.ip().to_string()).unwrap_or_default();
        for v in params.values_mut() {
            *v = v.replace("%u", username);
            *v = v.replace("%c", client_id);
            *v = v.replace("%a", &remote_addr);
            *v = v.replace("%r", "mqtt");
            *v = v.replace("%P", &password);
            if let Some((ref acl_type, topic)) = sub_or_pub {
                *v = v.replace("%A", acl_type.as_str());
                *v = v.replace("%t", topic);
            } else {
                *v = v.replace("%A", "");
                *v = v.replace("%t", "");
            }
        }
        Ok(())
    }

    async fn request(
        &self,
        id: &Id,
        mut req_cfg: config::Req,
        password: Option<&Password>,
        sub_or_pub: Option<(ACLType, &TopicName)>,
    ) -> Result<(ResponseResult, Cacheable)> {
        log::debug!("{:?} req_cfg.url.path(): {:?}", id, req_cfg.url.path());
        let (headers, timeout) = {
            let cfg = self.cfg.read().await;
            let headers = match (cfg.headers(), req_cfg.headers()) {
                (Some(def_headers), Some(req_headers)) => {
                    let mut headers = def_headers.clone();
                    headers.extend(req_headers.clone());
                    headers
                }
                (Some(def_headers), None) => def_headers.clone(),
                (None, Some(req_headers)) => req_headers.clone(),
                (None, None) => HeaderMap::new(),
            };
            (headers, cfg.http_timeout)
        };

        let (auth_result, superuser, cacheable) = if req_cfg.is_get() {
            let body = &mut req_cfg.params;
            Self::replaces(body, id, password, sub_or_pub)?;
            Self::http_get_request(req_cfg.url, body, headers, timeout).await?
        } else if req_cfg.json_body() {
            let body = &mut req_cfg.params;
            Self::replaces(body, id, password, sub_or_pub)?;
            Self::http_json_request(req_cfg.url, req_cfg.method, body, headers, timeout).await?
        } else {
            //form body
            let body = &mut req_cfg.params;
            Self::replaces(body, id, password, sub_or_pub)?;
            Self::http_form_request(req_cfg.url, req_cfg.method, body, headers, timeout).await?
        };
        log::debug!("auth_result: {:?}, superuser: {}, cacheable: {:?}", auth_result, superuser, cacheable);
        Ok((auth_result, cacheable))
    }

    async fn auth(&self, id: &Id, password: Option<&Password>) -> ResponseResult {
        if let Some(req) = { self.cfg.read().await.http_auth_req.clone() } {
            match self.request(id, req.clone(), password, None).await {
                Ok((auth_res, _)) => {
                    log::debug!("auth result: {:?}", auth_res);
                    auth_res
                }
                Err(e) => {
                    log::warn!("{:?} auth error, {:?}", id, e);
                    if self.cfg.read().await.deny_if_error {
                        ResponseResult::Deny
                    } else {
                        ResponseResult::Ignore
                    }
                }
            }
        } else {
            ResponseResult::Ignore
        }
    }

    async fn acl(&self, id: &Id, sub_or_pub: Option<(ACLType, &TopicName)>) -> (ResponseResult, Cacheable) {
        if let Some(req) = { self.cfg.read().await.http_acl_req.clone() } {
            match self.request(id, req.clone(), None, sub_or_pub).await {
                Ok(acl_res) => {
                    log::debug!("acl result: {:?}", acl_res);
                    acl_res
                }
                Err(e) => {
                    log::warn!("{:?} acl error, {:?}", id, e);
                    if self.cfg.read().await.deny_if_error {
                        (ResponseResult::Deny, None)
                    } else {
                        (ResponseResult::Ignore, None)
                    }
                }
            }
        } else {
            (ResponseResult::Ignore, None)
        }
    }
}

#[async_trait]
impl Handler for AuthHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::ClientAuthenticate(connect_info) => {
                log::debug!("ClientAuthenticate auth-http");
                if matches!(
                    acc,
                    Some(HookResult::AuthResult(AuthResult::BadUsernameOrPassword))
                        | Some(HookResult::AuthResult(AuthResult::NotAuthorized))
                ) {
                    return (false, acc);
                }

                return match self.auth(connect_info.id(), connect_info.password()).await {
                    ResponseResult::Allow(superuser) => {
                        (false, Some(HookResult::AuthResult(AuthResult::Allow(superuser))))
                    }
                    ResponseResult::Deny => {
                        (false, Some(HookResult::AuthResult(AuthResult::BadUsernameOrPassword)))
                    }
                    ResponseResult::Ignore => (true, None),
                };
            }

            Parameter::ClientSubscribeCheckAcl(session, subscribe) => {
                if let Some(HookResult::SubscribeAclResult(acl_result)) = &acc {
                    if acl_result.failure() {
                        return (false, acc);
                    }
                }

                //ResponseResult, Cacheable
                let (acl_res, _) = self.acl(&session.id, Some((ACLType::Sub, &subscribe.topic_filter))).await;
                return match acl_res {
                    ResponseResult::Allow(_) => (
                        false,
                        Some(HookResult::SubscribeAclResult(SubscribeAclResult::new_success(
                            subscribe.opts.qos(),
                            None,
                        ))),
                    ),
                    ResponseResult::Deny => (
                        false,
                        Some(HookResult::SubscribeAclResult(SubscribeAclResult::new_failure(
                            SubscribeAckReason::NotAuthorized,
                        ))),
                    ),
                    ResponseResult::Ignore => (true, None),
                };
            }

            Parameter::MessagePublishCheckAcl(session, publish) => {
                log::debug!("MessagePublishCheckAcl");
                if let Some(HookResult::PublishAclResult(PublishAclResult::Rejected(_))) = &acc {
                    return (false, acc);
                }

                let acl_res = if let Some((acl_res, expire)) = session
                    .extra_attrs
                    .read()
                    .await
                    .get::<HashMap<TopicName, (ResponseResult, i64)>>(CACHE_KEY)
                    .and_then(|cache_map| cache_map.get(publish.topic()))
                {
                    if *expire < 0 || chrono::Local::now().timestamp_millis() < *expire {
                        Some(*acl_res)
                    } else {
                        None
                    }
                } else {
                    None
                };

                let acl_res = if let Some(acl_res) = acl_res {
                    acl_res
                } else {
                    //ResponseResult, Cacheable
                    let (acl_res, cacheable) =
                        self.acl(&session.id, Some((ACLType::Pub, publish.topic()))).await;
                    if let Some(tm) = cacheable {
                        let expire = if tm < 0 { tm } else { chrono::Local::now().timestamp_millis() + tm };
                        if let Some(cache_map) = session
                            .extra_attrs
                            .write()
                            .await
                            .get_default_mut(CACHE_KEY.into(), HashMap::default)
                        {
                            cache_map.insert(publish.topic().clone(), (acl_res, expire));
                        }
                    }
                    acl_res
                };

                return match acl_res {
                    ResponseResult::Allow(_) => {
                        (false, Some(HookResult::PublishAclResult(PublishAclResult::Allow)))
                    }
                    ResponseResult::Deny => (
                        false,
                        Some(HookResult::PublishAclResult(PublishAclResult::Rejected(
                            self.cfg.read().await.disconnect_if_pub_rejected,
                        ))),
                    ),
                    ResponseResult::Ignore => (true, None),
                };
            }
            _ => {
                log::error!("unimplemented, {:?}", param)
            }
        }
        (true, acc)
    }
}

static HTTP_CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
});
