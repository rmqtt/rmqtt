#[macro_use]
extern crate serde;

mod config;

use reqwest::header::HeaderMap;
use reqwest::{Method, Url};
use serde::ser::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
type DashMap<K, V> = dashmap::DashMap<K, V, ahash::RandomState>;
use async_trait::async_trait;

use rmqtt::{
    broker::hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    broker::session::ClientInfo,
    broker::types::{AsStr, AuthResult, Password, PublishAclResult, SubscribeAckReason, SubscribeAclResult},
    plugin::Plugin,
    ClientId, MqttError, Result, Runtime, TopicName,
};

use config::PluginConfig;

const IGNORE: &str = "ignore";
const SUB: &str = "1";
const PUB: &str = "2";

type CacheMap = Arc<DashMap<ClientId, CacheValue>>;

#[inline]
pub async fn init<N: Into<String>, D: Into<String>>(
    runtime: &'static Runtime,
    name: N,
    descr: D,
    default_startup: bool,
) -> Result<()> {
    runtime
        .plugins
        .register(Box::new(AuthHttpPlugin::new(runtime, name.into(), descr.into()).await?), default_startup)
        .await?;
    Ok(())
}

struct AuthHttpPlugin {
    runtime: &'static Runtime,
    name: String,
    descr: String,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
    cache_map: CacheMap,
}

impl AuthHttpPlugin {
    #[inline]
    async fn new(runtime: &'static Runtime, name: String, descr: String) -> Result<Self> {
        let cfg = Arc::new(RwLock::new(runtime.settings.plugins.load_config::<PluginConfig>(&name)?));
        log::debug!("{} AuthHttpPlugin cfg: {:?}", name, cfg.read().await);
        let register = runtime.extends.hook_mgr().await.register();
        let cache_map = Arc::new(DashMap::default());
        Ok(Self { runtime, name, descr, register, cfg, cache_map })
    }
}

#[async_trait]
impl Plugin for AuthHttpPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name);
        let cfg = &self.cfg;
        let cache_map = &self.cache_map;

        let priority = cfg.read().await.priority;
        self.register
            .add_priority(Type::ClientAuthenticate, priority, Box::new(AuthHandler::new(cfg, cache_map)))
            .await;
        self.register
            .add_priority(Type::ClientSubscribeCheckAcl, priority, Box::new(AuthHandler::new(cfg, cache_map)))
            .await;
        self.register
            .add_priority(Type::MessagePublishCheckAcl, priority, Box::new(AuthHandler::new(cfg, cache_map)))
            .await;
        self.register
            .add_priority(Type::ClientDisconnected, priority, Box::new(AuthHandler::new(cfg, cache_map)))
            .await;

        Ok(())
    }

    #[inline]
    fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name);
        self.register.start().await;
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::info!("{} stop", self.name);
        self.register.stop().await;
        Ok(true)
    }

    #[inline]
    fn version(&self) -> &str {
        "0.1.1"
    }

    #[inline]
    fn descr(&self) -> &str {
        &self.descr
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.runtime.settings.plugins.load_config::<PluginConfig>(&self.name)?;
        *self.cfg.write().await = new_cfg;
        log::debug!("load_config ok,  {:?}", self.cfg);
        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }
}

struct AuthHandler {
    cfg: Arc<RwLock<PluginConfig>>,
    cache_map: CacheMap,
}

impl AuthHandler {
    fn new(cfg: &Arc<RwLock<PluginConfig>>, cache_map: &CacheMap) -> Self {
        Self { cfg: cfg.clone(), cache_map: cache_map.clone() }
    }

    async fn http_get_request<T: Serialize + ?Sized>(
        url: Url,
        body: &T,
        headers: HeaderMap,
        timeout: Duration,
    ) -> Result<(bool, String)> {
        log::debug!("http_get_request, timeout: {:?}, url: {}", timeout, url);
        match HTTP_CLIENT.clone().get(url).headers(headers).timeout(timeout).query(body).send().await {
            Err(e) => {
                log::error!("error:{:?}", e);
                Err(MqttError::Msg(e.to_string()))
            }
            Ok(resp) => {
                let ok = resp.status().is_success();
                let body = resp.text().await.map_err(|e| MqttError::Msg(e.to_string()))?;
                Ok((ok, body))
            }
        }
    }

    async fn http_form_request<T: Serialize + ?Sized>(
        url: Url,
        method: Method,
        body: &T,
        headers: HeaderMap,
        timeout: Duration,
    ) -> Result<(bool, String)> {
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
            Ok(resp) => {
                let ok = resp.status().is_success();
                let body = resp.text().await.map_err(|e| MqttError::Msg(e.to_string()))?;
                Ok((ok, body))
            }
        }
    }

    async fn http_json_request<T: Serialize + ?Sized>(
        url: Url,
        method: Method,
        body: &T,
        headers: HeaderMap,
        timeout: Duration,
    ) -> Result<(bool, String)> {
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
            Ok(resp) => {
                let ok = resp.status().is_success();
                let body = resp.text().await.map_err(|e| MqttError::Msg(e.to_string()))?;
                Ok((ok, body))
            }
        }
    }

    fn replaces<'a>(
        params: &'a mut HashMap<String, String>,
        client: &ClientInfo,
        password: Option<&Password>,
        sub_or_pub: Option<(&str, &TopicName)>,
    ) {
        for v in params.values_mut() {
            *v = v.replace("%u", client.connect_info.username().map(|n| n.as_str()).unwrap_or(""));
            *v = v.replace("%c", &client.id.client_id);
            *v =
                v.replace("%a", &client.id.remote_addr.map(|addr| addr.ip().to_string()).unwrap_or_default());
            *v = v.replace("%r", "mqtt");
            *v = v.replace("%P", password.map(|p| p.as_str()).unwrap_or_default());
            if let Some((flag, topic)) = sub_or_pub {
                *v = v.replace("%A", flag);
                *v = v.replace("%t", topic);
            } else {
                *v = v.replace("%A", "");
                *v = v.replace("%t", "");
            }
        }
    }

    async fn request(
        &self,
        client: &ClientInfo,
        mut req_cfg: config::Req,
        password: Option<&Password>,
        sub_or_pub: Option<(&str, &TopicName)>,
        check_super: bool,
    ) -> Result<bool> {
        log::debug!("req_cfg.url.path(): {:?}", req_cfg.url.path());
        let catch_key_fn = || {
            CacheKey(
                req_cfg.url.path().to_owned(),
                client.id.remote_addr.map(|addr| addr.ip()),
                password.cloned(),
                sub_or_pub.map(|(f, t)| (f.to_owned(), t.to_string())),
            )
        };
        let catch_key = {
            if let Some(c_map) = self.cache_map.get(&client.id.client_id) {
                log::debug!("{:?} catch value: {:?}", client.id, c_map.value());
                if c_map.is_super {
                    return Ok(true);
                }

                let catch_key = catch_key_fn();
                if c_map.ignores.contains(&catch_key) {
                    return Ok(true);
                }
                catch_key
            } else {
                catch_key_fn()
            }
        };

        log::debug!("catch_key: {:?}", catch_key);

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

        let (allow, ignore) = if req_cfg.is_get() {
            let body = &mut req_cfg.params;
            Self::replaces(body, client, password, sub_or_pub);
            Self::http_get_request(req_cfg.url, body, headers, timeout).await?
        } else if req_cfg.json_body() {
            let body = &mut req_cfg.params;
            Self::replaces(body, client, password, sub_or_pub);
            Self::http_json_request(req_cfg.url, req_cfg.method, body, headers, timeout).await?
        } else {
            //form body
            let body = &mut req_cfg.params;
            Self::replaces(body, client, password, sub_or_pub);
            Self::http_form_request(req_cfg.url, req_cfg.method, body, headers, timeout).await?
        };

        log::debug!("allow: {:?}", allow);

        //IGNORE
        if allow && ignore == IGNORE {
            let mut c_map = self.cache_map.entry(client.id.client_id.clone()).or_default();
            if check_super {
                c_map.is_super = true;
            } else {
                c_map.ignores.insert(catch_key);
            }
        }

        Ok(allow)
    }

    async fn auth(&self, client: &ClientInfo, password: Option<&Password>) -> bool {
        if let Some(req) = { self.cfg.read().await.http_auth_req.clone() } {
            match self.request(client, req.clone(), password, None, false).await {
                Ok(resp) => resp,
                Err(e) => {
                    log::warn!("{:?} auth error, {:?}", client.id, e);
                    false
                }
            }
        } else {
            true
        }
    }

    async fn is_super(&self, client: &ClientInfo) -> bool {
        if let Some(req) = { self.cfg.read().await.http_super_req.clone() } {
            match self.request(client, req.clone(), None, None, true).await {
                Ok(resp) => resp,
                Err(e) => {
                    log::warn!("{:?} check super error, {:?}", client.id, e);
                    false
                }
            }
        } else {
            true
        }
    }

    async fn acl(&self, client: &ClientInfo, sub_or_pub: Option<(&str, &TopicName)>) -> bool {
        if let Some(req) = { self.cfg.read().await.http_acl_req.clone() } {
            match self.request(client, req.clone(), None, sub_or_pub, false).await {
                Ok(allow) => {
                    log::debug!("acl.allow: {:?}", allow);
                    allow
                }
                Err(e) => {
                    log::warn!("{:?} acl error, {:?}", client.id, e);
                    false
                }
            }
        } else {
            true
        }
    }
}

#[async_trait]
impl Handler for AuthHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        match param {
            Parameter::ClientAuthenticate(_session, client_info, password) => {
                log::debug!("ClientAuthenticate auth-http");
                if matches!(
                    acc,
                    Some(HookResult::AuthResult(AuthResult::BadUsernameOrPassword))
                        | Some(HookResult::AuthResult(AuthResult::NotAuthorized))
                ) {
                    return (false, acc);
                }

                let stop = !self.cfg.read().await.break_if_allow;

                if self.is_super(*client_info).await {
                    return (stop, Some(HookResult::AuthResult(AuthResult::Allow)));
                }

                return if !self.auth(*client_info, password.as_ref()).await {
                    (false, Some(HookResult::AuthResult(AuthResult::NotAuthorized)))
                } else {
                    (stop, Some(HookResult::AuthResult(AuthResult::Allow)))
                };
            }

            Parameter::ClientSubscribeCheckAcl(_session, client_info, subscribe) => {
                if let Some(HookResult::SubscribeAclResult(acl_result)) = &acc {
                    if acl_result.failure() {
                        return (false, acc);
                    }
                }

                return if self.acl(*client_info, Some((SUB, &subscribe.topic_filter))).await {
                    (
                        !self.cfg.read().await.break_if_allow,
                        Some(HookResult::SubscribeAclResult(SubscribeAclResult::new_success(subscribe.qos))),
                    )
                } else {
                    (
                        false,
                        Some(HookResult::SubscribeAclResult(SubscribeAclResult::new_failure(
                            SubscribeAckReason::NotAuthorized,
                        ))),
                    )
                };
            }

            Parameter::MessagePublishCheckAcl(_session, client_info, publish) => {
                log::debug!("MessagePublishCheckAcl");
                if let Some(HookResult::PublishAclResult(PublishAclResult::Rejected(_))) = &acc {
                    return (false, acc);
                }

                return if self.acl(*client_info, Some((PUB, publish.topic()))).await {
                    (
                        !self.cfg.read().await.break_if_allow,
                        Some(HookResult::PublishAclResult(PublishAclResult::Allow)),
                    )
                } else {
                    (false, Some(HookResult::PublishAclResult(PublishAclResult::Rejected(true))))
                    //@TODO ... Do you want to disconnect?
                };
            }
            Parameter::ClientDisconnected(_session, client_info, _reason) => {
                log::debug!("ClientDisconnected");
                self.cache_map.remove(&client_info.id.client_id);
            }
            _ => {
                log::error!("unimplemented, {:?}", param)
            }
        }
        (true, acc)
    }
}

lazy_static::lazy_static! {
    static ref  HTTP_CLIENT: reqwest::Client = {
            reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap()
    };
}

type Path = String;
#[derive(Clone, Debug)]
struct CacheKey(Path, Option<std::net::IpAddr>, Option<Password>, Option<(String, String)>);

impl PartialEq<CacheKey> for CacheKey {
    #[inline]
    fn eq(&self, other: &CacheKey) -> bool {
        self.0 == other.0 && self.1 == other.1 && self.2 == other.2 && self.3 == other.3
    }
}
impl Eq for CacheKey {}

impl std::hash::Hash for CacheKey {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
        self.1.hash(state);
        self.2.hash(state);
        self.3.hash(state);
    }
}

#[derive(Debug, Default)]
struct CacheValue {
    is_super: bool,
    ignores: HashSet<CacheKey>,
}
