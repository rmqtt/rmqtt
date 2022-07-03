#[macro_use]
extern crate serde;
#[macro_use]
extern crate serde_json;

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use crossbeam::channel::{bounded, Receiver, Sender};
use parking_lot::RwLock;

use config::PluginConfig;
use rmqtt::{
    broker::hook::{self, Handler, HookResult, Parameter, Register, ReturnType, Type},
    broker::types::{ConnectInfo, Id, MQTT_LEVEL_5, QoSEx},
    plugin::{DynPlugin, DynPluginResult, Plugin},
    Result, Runtime, Topic, TopicFilter
};
use rmqtt::broker::error::MqttError;

mod config;

#[inline]
pub async fn register(
    runtime: &'static Runtime,
    name: &'static str,
    descr: &'static str,
    default_startup: bool,
    immutable: bool,
) -> Result<()> {
    runtime
        .plugins
        .register(name, default_startup, immutable, move || -> DynPluginResult {
            Box::pin(async move {
                WebHookPlugin::new(runtime, name, descr).await.map(|p| -> DynPlugin { Box::new(p) })
            })
        })
        .await?;
    Ok(())
}

struct WebHookPlugin {
    runtime: &'static Runtime,
    name: String,
    descr: String,
    register: Box<dyn Register>,

    cfg: Arc<RwLock<PluginConfig>>,
    tx: Arc<RwLock<Sender<Message>>>,
    processings: Arc<AtomicIsize>,
}

impl WebHookPlugin {
    #[inline]
    async fn new<S: Into<String>>(runtime: &'static Runtime, name: S, descr: S) -> Result<Self> {
        let name = name.into();
        let cfg = Arc::new(RwLock::new(
            runtime
                .settings
                .plugins
                .load_config::<PluginConfig>(&name)
                .map_err(|e| MqttError::from(e.to_string()))?,
        ));
        log::debug!("{} WebHookPlugin cfg: {:?}", name, cfg.read());
        let processings = Arc::new(AtomicIsize::new(0));
        let tx = Arc::new(RwLock::new(Self::start(runtime, cfg.clone(), processings.clone())));
        let register = runtime.extends.hook_mgr().await.register();
        Ok(Self { runtime, name, descr: descr.into(), register, cfg, tx, processings })
    }

    fn start(
        _runtime: &'static Runtime,
        cfg: Arc<RwLock<PluginConfig>>,
        processings: Arc<AtomicIsize>,
    ) -> Sender<Message> {
        let (tx, rx): (Sender<Message>, Receiver<Message>) = bounded(cfg.read().queue_capacity);
        let _child = std::thread::Builder::new().name("web-hook".to_string()).spawn(move || {
            log::info!("start web-hook async worker.");
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(cfg.read().worker_threads)
                .thread_name("web-hook-worker")
                .thread_stack_size(4 * 1024 * 1024)
                .build()
                .unwrap();
            let concurrency_limit = cfg.read().concurrency_limit as isize;
            let runner = async {
                loop {
                    let cfg = cfg.clone();
                    let processings = processings.clone();
                    match rx.recv() {
                        Ok(msg) => {
                            log::trace!("received web-hook Message: {:?}", msg);
                            match msg {
                                Message::Body(typ, topic, data) => {
                                    let processings1 = processings.clone();
                                    let handle = async move {
                                        if let Err(e) =
                                        WebHookHandler::handle(cfg.clone(), typ, topic, data).await
                                        {
                                            log::error!("send web hook message error, {:?}", e);
                                        }
                                        processings1.fetch_sub(1, Ordering::SeqCst);
                                    };
                                    if processings.fetch_add(1, Ordering::SeqCst) < concurrency_limit {
                                        tokio::task::spawn(handle);
                                    } else {
                                        handle.await;
                                    }
                                }
                                Message::Exit => {
                                    log::debug!("Is Message::Exit message ...");
                                    break;
                                }
                            }
                        }
                        Err(e) => {
                            log::error!("web hook message channel abnormal, {:?}", e);
                            break;
                        }
                    }
                }
            };
            rt.block_on(runner);
            log::info!("exit web-hook async worker.");
        });
        tx
    }
}

#[async_trait]
impl Plugin for WebHookPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name);
        let tx = self.tx.clone();
        self.register.add(Type::SessionCreated, Box::new(WebHookHandler { tx: tx.clone() })).await;
        self.register.add(Type::SessionTerminated, Box::new(WebHookHandler { tx: tx.clone() })).await;
        self.register.add(Type::SessionSubscribed, Box::new(WebHookHandler { tx: tx.clone() })).await;
        self.register.add(Type::SessionUnsubscribed, Box::new(WebHookHandler { tx: tx.clone() })).await;

        self.register.add(Type::ClientConnect, Box::new(WebHookHandler { tx: tx.clone() })).await;
        self.register.add(Type::ClientConnack, Box::new(WebHookHandler { tx: tx.clone() })).await;
        self.register.add(Type::ClientConnected, Box::new(WebHookHandler { tx: tx.clone() })).await;
        self.register.add(Type::ClientDisconnected, Box::new(WebHookHandler { tx: tx.clone() })).await;
        self.register.add(Type::ClientSubscribe, Box::new(WebHookHandler { tx: tx.clone() })).await;
        self.register.add(Type::ClientUnsubscribe, Box::new(WebHookHandler { tx: tx.clone() })).await;

        self.register.add(Type::MessagePublish, Box::new(WebHookHandler { tx: tx.clone() })).await;
        self.register.add(Type::MessageDelivered, Box::new(WebHookHandler { tx: tx.clone() })).await;
        self.register.add(Type::MessageAcked, Box::new(WebHookHandler { tx: tx.clone() })).await;
        self.register.add(Type::MessageDropped, Box::new(WebHookHandler { tx: tx.clone() })).await;

        Ok(())
    }

    #[inline]
    fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().to_json()
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.runtime.settings.plugins.load_config::<PluginConfig>(&self.name)?;
        let cfg = { self.cfg.read().clone() };
        if cfg.worker_threads != new_cfg.worker_threads
            || cfg.queue_capacity != new_cfg.queue_capacity
            || cfg.concurrency_limit != new_cfg.concurrency_limit
        {
            let new_cfg = Arc::new(RwLock::new(new_cfg));
            //restart
            let new_tx = Self::start(self.runtime, new_cfg.clone(), self.processings.clone());
            if let Err(e) = self.tx.read().send_timeout(Message::Exit, std::time::Duration::from_secs(3)) {
                log::error!("restart web-hook failed, {:?}", e);
                return Err(MqttError::Error(Box::new(e)));
            }
            self.cfg = new_cfg;
            *self.tx.write() = new_tx;
        } else {
            *self.cfg.write() = new_cfg;
        }
        log::debug!("load_config ok,  {:?}", self.cfg);
        Ok(())
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
    async fn attrs(&self) -> serde_json::Value {
        json!({
            "queue_len": self.tx.read().len(),
            "active_tasks": self.processings.load(Ordering::SeqCst)
        })
    }
}

lazy_static::lazy_static! {
    static ref  HTTP_CLIENT: reqwest::Client = {
            reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(8))
                .timeout(Duration::from_secs(15))
                .build()
                .unwrap()
    };
}

#[derive(Debug)]
pub enum Message {
    Body(hook::Type, Option<TopicFilter>, serde_json::Value),
    Exit,
}

struct WebHookHandler {
    //cfg: Arc<RwLock<PluginConfig>>,
    tx: Arc<RwLock<Sender<Message>>>,
}

impl WebHookHandler {
    async fn handle(
        cfg: Arc<RwLock<PluginConfig>>,
        typ: hook::Type,
        topic: Option<TopicFilter>,
        body: serde_json::Value,
    ) -> Result<()> {
        let (timeout, default_urls) = {
            let cfg = cfg.read();
            (cfg.http_timeout, cfg.http_urls.clone())
        };

        let topic = if let Some(topic) = topic { Some(Topic::from_str(&topic)?) } else { None };

        let http_requests = if let Some(rules) = cfg.read().rules.get(&typ) {
            //get action and urls
            let action_urls = rules.iter().filter_map(|r| {
                let is_allowed = if let Some(topic) = &topic {
                    if let Some((rule_topics, _)) = &r.topics {
                        rule_topics.is_match(topic)
                    } else {
                        true
                    }
                } else {
                    true
                };

                if is_allowed {
                    let urls = if r.urls.is_empty() { &default_urls } else { &r.urls };
                    if urls.is_empty() {
                        None
                    } else {
                        Some((&r.action, urls))
                    }
                } else {
                    None
                }
            });

            //build http send futures
            let mut http_requests = Vec::new();
            for (action, urls) in action_urls {
                let mut new_body = body.clone();
                if let Some(obj) = new_body.as_object_mut() {
                    obj.insert("action".into(), serde_json::Value::String(action.clone()));
                }
                if urls.len() == 1 {
                    log::debug!("action: {}, url: {}", action, urls[0]);
                    http_requests.push(Self::http_request(urls[0].clone(), new_body, timeout));
                } else {
                    for url in urls {
                        log::debug!("action: {}, url: {}", action, url);
                        http_requests.push(Self::http_request(url.clone(), new_body.clone(), timeout));
                    }
                }
            }

            Some(http_requests)
        } else {
            None
        };

        //send http_requests
        if let Some(http_requests) = http_requests {
            log::debug!("http_requests length: {}", http_requests.len());
            let _ = futures::future::join_all(http_requests).await;
        }

        Ok(())
    }

    async fn http_request(url: String, body: serde_json::Value, timeout: Duration) {
        log::debug!("http_request, timeout: {:?}, url: {}, body: {}", timeout, url, body);
        match HTTP_CLIENT
            .clone()
            .request(reqwest::Method::POST, &url)
            .timeout(timeout)
            .json(&body)
            .send()
            .await
        {
            Err(e) => {
                log::error!("url:{:?}, error:{:?}", url, e);
            }
            Ok(resp) => {
                if !resp.status().is_success() {
                    log::warn!("response status is not OK, url:{:?}, response:{:?}", url, resp);
                }
            }
        }
    }
}

trait ToBody {
    fn to_body(&self) -> serde_json::Value;
}

impl ToBody for ConnectInfo {
    fn to_body(&self) -> serde_json::Value {
        match self {
            ConnectInfo::V3(id, conn_info) => {
                json!({
                    "node": id.node(),
                    "ipaddress": id.remote_addr,
                    "clientid": id.client_id,
                    "username": id.username,
                    "keepalive": conn_info.keep_alive,
                    "proto_ver": conn_info.protocol.level(),
                    "clean_session": conn_info.clean_session,
                })
            }
            ConnectInfo::V5(id, conn_info) => {
                json!({
                    "node": id.node(),
                    "ipaddress": id.remote_addr,
                    "clientid": id.client_id,
                    "username": id.username,
                    "keepalive": conn_info.keep_alive,
                    "proto_ver": MQTT_LEVEL_5,
                    "clean_start": conn_info.clean_start,
                })
            }
        }
    }
}

trait JsonTo {
    fn to(&self, json: serde_json::Value) -> serde_json::Value;
}

impl JsonTo for Id {
    fn to(&self, mut json: serde_json::Value) -> serde_json::Value {
        if let Some(obj) = json.as_object_mut() {
            obj.insert("node".into(), serde_json::Value::String(self.node()));
            obj.insert(
                "ipaddress".into(),
                self.remote_addr
                    .map(|a| serde_json::Value::String(a.to_string()))
                    .unwrap_or(serde_json::Value::Null),
            );
            obj.insert("clientid".into(), serde_json::Value::String(self.client_id.to_string()));
            obj.insert("username".into(), serde_json::Value::String(self.username.to_string()));
        }
        json
    }
}

trait JsonFrom {
    fn from(&self, json: serde_json::Value) -> serde_json::Value;
}

impl JsonFrom for Id {
    fn from(&self, mut json: serde_json::Value) -> serde_json::Value {
        if let Some(obj) = json.as_object_mut() {
            obj.insert("from_node".into(), serde_json::Value::String(self.node()));
            obj.insert(
                "from_ipaddress".into(),
                self.remote_addr
                    .map(|a| serde_json::Value::String(a.to_string()))
                    .unwrap_or(serde_json::Value::Null),
            );
            obj.insert("from_clientid".into(), serde_json::Value::String(self.client_id.to_string()));
            obj.insert("from_username".into(), serde_json::Value::String(self.username.to_string()));
        }
        json
    }
}

#[async_trait]
impl Handler for WebHookHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        let typ = param.get_type();

        let bodys = match param {
            Parameter::ClientConnect(conn_info) => {
                vec![(None, conn_info.to_body())]
            }
            Parameter::ClientConnack(conn_info, conn_ack) => {
                let mut body = conn_info.to_body();
                if let Some(obj) = body.as_object_mut() {
                    obj.insert("conn_ack".into(), serde_json::Value::String(conn_ack.reason().to_string()));
                }
                vec![(None, body)]
            }

            Parameter::ClientConnected(_session, client) => {
                let mut body = client.connect_info.to_body();
                if let Some(obj) = body.as_object_mut() {
                    obj.insert(
                        "connected_at".into(),
                        serde_json::Value::Number(serde_json::Number::from(client.connected_at)),
                    );
                    obj.insert("session_present".into(), serde_json::Value::Bool(client.session_present));
                }
                vec![(None, body)]
            }

            Parameter::ClientDisconnected(_session, client, reason) => {
                let body = json!({
                    "node": client.id.node(),
                    "ipaddress": client.id.remote_addr,
                    "clientid": client.id.client_id,
                    "username": client.id.username,
                    "disconnected_at": client.disconnected_at(),
                    "reason": reason
                });
                vec![(None, body)]
            }

            Parameter::ClientSubscribe(_session, client, subscribe) => {
                let body = json!({
                    "node": client.id.node(),
                    "ipaddress": client.id.remote_addr,
                    "clientid": client.id.client_id,
                    "username": client.id.username,
                    "topic": subscribe.topic_filter.to_string(),
                    "opts": json!({
                        "qos": subscribe.qos.value()
                    }),
                });
                vec![(Some(subscribe.topic_filter.clone()), body)]
            }

            Parameter::ClientUnsubscribe(_session, client, unsubscribe) => {
                let body = json!({
                    "node": client.id.node(),
                    "ipaddress": client.id.remote_addr,
                    "clientid": client.id.client_id,
                    "username": client.id.username,
                    "topic": unsubscribe.topic_filter,
                });
                vec![(Some(unsubscribe.topic_filter.clone()), body)]
            }

            Parameter::SessionSubscribed(_session, client, subscribe) => {
                let body = json!({
                    "node": client.id.node(),
                    "ipaddress": client.id.remote_addr,
                    "clientid": client.id.client_id,
                    "username": client.id.username,
                    "topic": subscribe.topic_filter,
                    "opts": json!({
                        "qos": subscribe.qos.value()
                    }),
                });
                vec![(Some(subscribe.topic_filter.clone()), body)]
            }

            Parameter::SessionUnsubscribed(_session, client, unsubscribed) => {
                let topic = unsubscribed.topic_filter.clone();
                let body = json!({
                    "node": client.id.node(),
                    "ipaddress": client.id.remote_addr,
                    "clientid": client.id.client_id,
                    "username": client.id.username,
                    "topic": topic,
                });
                vec![(Some(topic), body)]
            }

            Parameter::SessionCreated(session, client) => {
                let body = json!({
                    "node": client.id.node(),
                    "ipaddress": client.id.remote_addr,
                    "clientid": client.id.client_id,
                    "username": client.id.username,
                    "created_at": session.created_at,
                });
                vec![(None, body)]
            }

            Parameter::SessionTerminated(_session, client, reason) => {
                let body = json!({
                    "node": client.id.node(),
                    "ipaddress": client.id.remote_addr,
                    "clientid": client.id.client_id,
                    "username": client.id.username,
                    "reason": reason
                });
                vec![(None, body)]
            }

            Parameter::MessagePublish(_session, client, publish) => {
                let topic = publish.topic().clone();
                let body = json!({
                    "dup": publish.dup(),
                    "retain": publish.retain(),
                    "qos": publish.qos().value(),
                    "topic": topic.to_string(),
                    "packet_id": publish.packet_id(),
                    "payload": base64::encode(publish.payload()),
                    "ts": publish.create_time(),
                });
                let body = client.id.from(body);
                vec![(Some(topic), body)]
            }

            Parameter::MessageDelivered(_session, client, from, publish) => {
                let topic = publish.topic().clone();
                let body = json!({
                    "dup": publish.dup(),
                    "retain": publish.retain(),
                    "qos": publish.qos().value(),
                    "topic": topic.to_string(),
                    "packet_id": publish.packet_id(),
                    "payload": base64::encode(publish.payload()),
                    "ts": chrono::Local::now().timestamp_millis(),
                });
                let body = client.id.to(body);
                let body = from.from(body);
                vec![(Some(topic), body)]
            }

            Parameter::MessageAcked(_session, client, from, publish) => {
                let topic = publish.topic().clone();
                let body = json!({
                    "dup": publish.dup(),
                    "retain": publish.retain(),
                    "qos": publish.qos().value(),
                    "topic": topic.to_string(),
                    "packet_id": publish.packet_id(),
                    "payload": base64::encode(publish.payload()),
                    "ts": chrono::Local::now().timestamp_millis(),
                });
                let body = client.id.to(body);
                let body = from.from(body);
                vec![(Some(topic), body)]
            }

            Parameter::MessageDropped(to, from, publish, reason) => {
                let body = json!({
                    "dup": publish.dup(),
                    "retain": publish.retain(),
                    "qos": publish.qos().value(),
                    "topic": publish.topic().to_string(),
                    "packet_id": publish.packet_id(),
                    "payload": base64::encode(publish.payload()),
                    "reason": reason,
                    "ts": chrono::Local::now().timestamp_millis(),
                });
                let mut body = from.from(body);
                if let Some(to) = to {
                    body = to.to(body);
                }

                vec![(None, body)]
            }
            _ => {
                log::error!("parameter is: {:?}", param);
                Vec::new()
            }
        };

        log::debug!("bodys: {:?}", bodys);

        if !bodys.is_empty() {
            for (topic, body) in bodys {
                if let Err(e) = self.tx.read().try_send(Message::Body(typ, topic, body)) {
                    log::warn!("web-hook send error, typ: {:?}, {:?}", typ, e);
                }
            }
        }

        (true, acc)
    }
}
