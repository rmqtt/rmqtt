#![deny(unsafe_code)]
#[macro_use]
extern crate serde;

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use backoff::future::retry;
use backoff::ExponentialBackoff;
use futures::channel::mpsc::{channel, Receiver, Sender};
use futures::StreamExt;

use crate::config::Url;
use config::PluginConfig;
use rmqtt::tokio::fs::{File, OpenOptions};
use rmqtt::tokio::io::AsyncWriteExt;
use rmqtt::{
    anyhow::anyhow,
    async_trait::async_trait,
    base64,
    bytestring::ByteString,
    chrono, futures, lazy_static, log, reqwest,
    rust_box::std_ext::ArcExt,
    rust_box::task_exec_queue::SpawnExt,
    serde_json::{self, json},
    tokio, DashMap, RwLock,
};
use rmqtt::{
    broker::error::MqttError,
    broker::hook::{self, Handler, HookResult, Parameter, Register, ReturnType, Type},
    broker::stats::Counter,
    broker::types::{ConnectInfo, Id, QoSEx, MQTT_LEVEL_5},
    plugin::{DynPlugin, DynPluginResult, Plugin},
    Result, Runtime, Topic, TopicFilter,
};
use rmqtt::{
    once_cell::sync::OnceCell,
    rust_box::task_exec_queue::{Builder, TaskExecQueue},
};

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
    writers: Arc<DashMap<ByteString, HookWriter>>,
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
        cfg.write().merge_urls();
        log::debug!("{} WebHookPlugin cfg: {:?}", name, cfg.read());
        let writers = Arc::new(DashMap::default());
        let tx = Arc::new(RwLock::new(Self::start(runtime, cfg.clone(), writers.clone())));
        let register = runtime.extends.hook_mgr().await.register();
        Ok(Self { runtime, name, descr: descr.into(), register, cfg, tx, writers })
    }

    fn start(
        _runtime: &'static Runtime,
        cfg: Arc<RwLock<PluginConfig>>,
        writers: Arc<DashMap<ByteString, HookWriter>>,
    ) -> Sender<Message> {
        let (tx, mut rx): (Sender<Message>, Receiver<Message>) = channel(cfg.read().queue_capacity);
        let _child = std::thread::Builder::new().name("web-hook".to_string()).spawn(move || {
            log::info!("start web-hook async worker.");
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(cfg.read().worker_threads)
                .thread_name("web-hook-worker")
                .thread_stack_size(4 * 1024 * 1024)
                .build()
                .unwrap();
            let runner = async {
                init_task_exec_queue(cfg.read().concurrency_limit, cfg.read().queue_capacity);
                let backoff_strategy = cfg.read().get_backoff_strategy().arc();
                loop {
                    let cfg = cfg.clone();
                    let writers = writers.clone();
                    let backoff_strategy = backoff_strategy.clone();
                    match rx.next().await {
                        Some(msg) => {
                            log::trace!("received web-hook Message: {:?}", msg);
                            match msg {
                                Message::Body(typ, topic, data) => {
                                    if let Err(e) = WebHookHandler::handle(
                                        cfg,
                                        writers,
                                        backoff_strategy.clone(),
                                        typ,
                                        topic,
                                        data,
                                    )
                                    .await
                                    {
                                        log::warn!("Failed to build the web-hook message, {:?}", e);
                                    }
                                }
                                Message::Exit => {
                                    log::debug!("Is Message::Exit message ...");
                                    break;
                                }
                            }
                        }
                        None => {
                            log::error!("web hook message channel is closed!");
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
            let new_tx = Self::start(self.runtime, new_cfg.clone(), self.writers.clone());
            if let Err(e) = self.tx.write().try_send(Message::Exit) {
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
        let exec = task_exec_queue();

        json!({
            "active_count": exec.active_count(),
            "waiting_count": exec.waiting_count(),
            "completed_count": exec.completed_count(),
            "failure_count()": fails().count(),
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
    tx: Arc<RwLock<Sender<Message>>>,
}

impl WebHookHandler {
    async fn handle(
        cfg: Arc<RwLock<PluginConfig>>,
        writers: Arc<DashMap<ByteString, HookWriter>>,
        backoff_strategy: Arc<ExponentialBackoff>,
        typ: hook::Type,
        topic: Option<TopicFilter>,
        body: serde_json::Value,
    ) -> Result<()> {
        let topic = if let Some(topic) = topic { Some(Topic::from_str(&topic)?) } else { None };

        let hook_writes = {
            let cfg = cfg.read();
            if let Some(rules) = cfg.rules.get(&typ) {
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
                        let urls = if r.urls.is_empty() { cfg.urls() } else { &r.urls };
                        if urls.is_empty() {
                            None
                        } else {
                            Some((&r.action, urls))
                        }
                    } else {
                        None
                    }
                });

                //build hook log write futures
                let mut hook_writes = Vec::new();
                for (action, urls) in action_urls {
                    let mut new_body = body.clone();
                    if let Some(obj) = new_body.as_object_mut() {
                        obj.insert("action".into(), serde_json::Value::String(action.clone()));
                    }
                    if urls.len() == 1 {
                        log::debug!("action: {}, url: {:?}", action, urls[0]);
                        hook_writes.push(Self::write(
                            writers.clone(),
                            backoff_strategy.clone(),
                            urls[0].clone(),
                            new_body.arc(),
                            cfg.http_timeout,
                        ));
                    } else {
                        let new_body = new_body.arc();
                        for url in urls {
                            log::debug!("action: {}, url: {:?}", action, url);
                            hook_writes.push(Self::write(
                                writers.clone(),
                                backoff_strategy.clone(),
                                url.clone(),
                                new_body.clone(),
                                cfg.http_timeout,
                            ));
                        }
                    }
                }

                Some(hook_writes)
            } else {
                None
            }
        };

        //send hook_writes
        if let Some(hook_writes) = hook_writes {
            log::debug!("hook_writes length: {}", hook_writes.len());
            let _ = futures::future::join_all(hook_writes).await;
        }

        Ok(())
    }

    #[inline]
    async fn write(
        writers: Arc<DashMap<ByteString, HookWriter>>,
        backoff_strategy: Arc<ExponentialBackoff>,
        url: Url,
        body: Arc<serde_json::Value>,
        timeout: Duration,
    ) {
        if url.is_file() {
            //is file
            let data = match serde_json::to_vec(body.as_ref()) {
                Ok(data) => data,
                Err(e) => {
                    log::warn!("write hook message failure, {:?}", e);
                    return;
                }
            };
            let mut writer = writers.entry(url.loc.clone()).or_insert_with(|| HookWriter::new(&url.loc));
            if let Err(e) = writer.log(data.as_slice()).await {
                log::warn!("write hook message failure, file: {:?}, {:?}", writer.file_name, e);
            }
        } else {
            //is http
            Self::http_request(backoff_strategy, url, body, timeout).await;
        }
    }

    async fn http_request(
        backoff_strategy: Arc<ExponentialBackoff>,
        url: Url,
        body: Arc<serde_json::Value>,
        timeout: Duration,
    ) {
        if let Err(e) = async move {
            if let Err(e) = retry(backoff_strategy.as_ref().clone(), || async {
                Ok(Self::_http_request(&url.loc, body.clone(), timeout).await?)
            })
            .await
            {
                fails().current_inc();
                log::warn!("send web hook message failure, {:?}", e);
            }
        }
        .spawn(task_exec_queue())
        .await
        {
            fails().current_inc();
            log::error!("send web hook message failure, exec task error, {:?}", e.to_string());
        }
    }

    async fn _http_request(url: &str, body: Arc<serde_json::Value>, timeout: Duration) -> Result<()> {
        log::debug!("http_request, timeout: {:?}, url: {}, body: {}", timeout, url, body);

        let resp = HTTP_CLIENT
            .clone()
            .request(reqwest::Method::POST, url)
            .timeout(timeout)
            .json(body.as_ref())
            .send()
            .await
            .map_err(|e| MqttError::Anyhow(anyhow!(e)))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(MqttError::from(format!("response status is not OK, url:{:?}, response:{:?}", url, resp)))
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
            obj.insert("node".into(), serde_json::Value::Number(serde_json::Number::from(self.node())));
            obj.insert(
                "ipaddress".into(),
                self.remote_addr
                    .map(|a| serde_json::Value::String(a.to_string()))
                    .unwrap_or(serde_json::Value::Null),
            );
            obj.insert("clientid".into(), serde_json::Value::String(self.client_id.to_string()));
            obj.insert("username".into(), serde_json::Value::String(self.username_ref().into()));
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
            obj.insert("from_node".into(), serde_json::Value::Number(serde_json::Number::from(self.node())));
            obj.insert(
                "from_ipaddress".into(),
                self.remote_addr
                    .map(|a| serde_json::Value::String(a.to_string()))
                    .unwrap_or(serde_json::Value::Null),
            );
            obj.insert("from_clientid".into(), serde_json::Value::String(self.client_id.to_string()));
            obj.insert("from_username".into(), serde_json::Value::String(self.username_ref().into()));
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
                    "topic": subscribe.topic_filter,
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
                let topic = publish.topic();
                let body = json!({
                    "dup": publish.dup(),
                    "retain": publish.retain(),
                    "qos": publish.qos().value(),
                    "topic": topic,
                    "packet_id": publish.packet_id(),
                    "payload": base64::encode(publish.payload()),
                    "ts": publish.create_time(),
                });
                let body = client.id.from(body);
                vec![(Some(topic.clone()), body)]
            }

            Parameter::MessageDelivered(_session, client, from, publish) => {
                let topic = publish.topic();
                let body = json!({
                    "dup": publish.dup(),
                    "retain": publish.retain(),
                    "qos": publish.qos().value(),
                    "topic": topic,
                    "packet_id": publish.packet_id(),
                    "payload": base64::encode(publish.payload()),
                    "pts": publish.create_time(),
                    "ts": chrono::Local::now().timestamp_millis(),
                });
                let body = client.id.to(body);
                let body = from.from(body);
                vec![(Some(topic.clone()), body)]
            }

            Parameter::MessageAcked(_session, client, from, publish) => {
                let topic = publish.topic();
                let body = json!({
                    "dup": publish.dup(),
                    "retain": publish.retain(),
                    "qos": publish.qos().value(),
                    "topic": topic,
                    "packet_id": publish.packet_id(),
                    "payload": base64::encode(publish.payload()),
                    "pts": publish.create_time(),
                    "ts": chrono::Local::now().timestamp_millis(),
                });
                let body = client.id.to(body);
                let body = from.from(body);
                vec![(Some(topic.clone()), body)]
            }

            Parameter::MessageDropped(to, from, publish, reason) => {
                let body = json!({
                    "dup": publish.dup(),
                    "retain": publish.retain(),
                    "qos": publish.qos().value(),
                    "topic": publish.topic(),
                    "packet_id": publish.packet_id(),
                    "payload": base64::encode(publish.payload()),
                    "reason": reason,
                    "pts": publish.create_time(),
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
            let mut tx = self.tx.read().clone();
            for (topic, body) in bodys {
                if let Err(e) = tx.try_send(Message::Body(typ, topic, body)) {
                    log::warn!("web-hook send error, typ: {:?}, {:?}", typ, e);
                }
            }
        }

        (true, acc)
    }
}

struct HookWriter {
    file_name: String,
    file: Option<File>,
}

impl HookWriter {
    fn new(file: &str) -> Self {
        Self { file_name: String::from(file), file: None }
    }

    #[inline]
    pub async fn log(&mut self, msg: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(file) = self.file.as_mut() {
            file.write_all(msg).await?;
            file.write_all(b"\n").await?;
        } else {
            Self::create_dirs(Path::new(&self.file_name)).await?;
            let mut file = OpenOptions::new().create(true).append(true).open(&self.file_name).await?;
            file.write_all(msg).await?;
            file.write_all(b"\n").await?;
            self.file.replace(file);
        }
        Ok(())
    }

    #[inline]
    async fn create_dirs(path: &Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            // If the parent directory does not exist, create it recursively.
            if !parent.exists() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        Ok(())
    }
}

static TASK_EXEC_QUEUE: OnceCell<TaskExecQueue> = OnceCell::new();

#[inline]
fn init_task_exec_queue(workers: usize, queue_max: usize) {
    let (exec, task_runner) = Builder::default().workers(workers).queue_max(queue_max).build();

    tokio::spawn(async move {
        task_runner.await;
    });

    TASK_EXEC_QUEUE.set(exec).ok().expect("Failed to initialize task execution queue")
}

#[inline]
pub(crate) fn task_exec_queue() -> &'static TaskExecQueue {
    TASK_EXEC_QUEUE.get().expect("TaskExecQueue not initialized")
}

//Failure count
#[inline]
pub(crate) fn fails() -> &'static Counter {
    static INSTANCE: OnceCell<Counter> = OnceCell::new();
    INSTANCE.get_or_init(Counter::new)
}
