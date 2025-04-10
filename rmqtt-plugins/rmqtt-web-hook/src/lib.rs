#![deny(unsafe_code)]

use std::path::Path;
use std::str::FromStr;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use backoff::{future::retry, ExponentialBackoff};
use base64::prelude::{Engine, BASE64_STANDARD};
use bytestring::ByteString;
use once_cell::sync::{Lazy, OnceCell};
use rust_box::task_exec_queue::{Builder, SpawnExt, TaskExecQueue};
use serde_json::{self, json};
use tokio::{
    self,
    fs::{File, OpenOptions},
    io::AsyncWriteExt,
    sync::mpsc::{channel, Receiver, Sender},
    sync::RwLock,
    time,
};

use rmqtt::{
    context::ServerContext,
    hook::{self, Handler, HookResult, Parameter, Register, ReturnType, Type},
    macros::Plugin,
    plugin::{PackageInfo, Plugin},
    register,
    types::{DashMap, Topic, TopicFilter},
    utils::Counter,
    Result,
};

use config::{PluginConfig, Url};

mod config;

type HookWriters = Arc<DashMap<ByteString, Arc<RwLock<HookWriter>>>>;

register!(WebHookPlugin::new);

#[derive(Plugin)]
struct WebHookPlugin {
    scx: ServerContext,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
    chan_queue_count: Arc<AtomicIsize>,
    tx: Arc<RwLock<Sender<Message>>>,
    writers: HookWriters,
    exec: TaskExecQueue,
}

impl WebHookPlugin {
    #[inline]
    async fn new<S: Into<String>>(scx: ServerContext, name: S) -> Result<Self> {
        let name = name.into();
        let cfg = Arc::new(RwLock::new(Self::load_config(&scx, &name)?));
        log::debug!("{} WebHookPlugin cfg: {:?}", name, cfg.read().await);
        let writers = Arc::new(DashMap::default());
        let chan_queue_count = Arc::new(AtomicIsize::new(0));
        let (tx, exec) = Self::start(&scx, cfg.clone(), writers.clone(), chan_queue_count.clone()).await;
        let tx = Arc::new(RwLock::new(tx));
        let register = scx.extends.hook_mgr().register();
        Ok(Self { scx, register, cfg, chan_queue_count, tx, writers, exec })
    }

    async fn start(
        _scx: &ServerContext,
        cfg: Arc<RwLock<PluginConfig>>,
        writers: HookWriters,
        chan_queue_count: Arc<AtomicIsize>,
    ) -> (Sender<Message>, TaskExecQueue) {
        let worker_threads = cfg.read().await.worker_threads;
        let (tx, mut rx): (Sender<Message>, Receiver<Message>) = channel(cfg.read().await.queue_capacity);

        let (exec_tx, exec_rx) = tokio::sync::oneshot::channel();
        let _child = std::thread::Builder::new().name("web-hook".to_string()).spawn(move || {
            log::info!("start web-hook async worker.");
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(worker_threads)
                .thread_name("web-hook-worker")
                .thread_stack_size(4 * 1024 * 1024)
                .build()
                .expect("tokio runtime build failed");
            let runner = async {
                let exec =
                    init_task_exec_queue(cfg.read().await.concurrency_limit, cfg.read().await.queue_capacity);
                if exec_tx.send(exec.clone()).is_err() {
                    log::error!("tokio oneshot channel send failed");
                }
                let backoff_strategy = Arc::new(cfg.read().await.get_backoff_strategy());
                loop {
                    let cfg = cfg.clone();
                    let writers = writers.clone();
                    let backoff_strategy = backoff_strategy.clone();
                    match rx.recv().await {
                        Some(msg) => {
                            chan_queue_count.fetch_sub(1, Ordering::SeqCst);
                            log::trace!("received web-hook Message: {:?}", msg);
                            if exec.is_full() {
                                loop {
                                    time::sleep(Duration::from_millis(1)).await;
                                    if !exec.is_full() {
                                        break;
                                    }
                                }
                            }
                            Self::handle_msg(&exec, cfg, writers, backoff_strategy, msg).await;
                        }
                        None => {
                            log::info!("web hook message channel is closed!");
                            break;
                        }
                    }
                }
            };
            rt.block_on(runner);
            log::info!("exit web-hook async worker.");
        });
        let exec = exec_rx.await.expect("tokio oneshot channel recv failed");
        (tx, exec)
    }

    #[inline]
    async fn handle_msg(
        exec: &TaskExecQueue,
        cfg: Arc<RwLock<PluginConfig>>,
        writers: HookWriters,
        backoff_strategy: Arc<ExponentialBackoff>,
        msg: Message,
    ) {
        if let Err(e) = async move {
            let (typ, topic, data) = msg;
            if let Err(e) = WebHookHandler::handle(cfg, writers, backoff_strategy, typ, topic, data).await {
                log::warn!("Failed to build the web-hook message, {:?}", e);
            }
        }
        .spawn(exec)
        .await
        {
            log::error!("send web hook message failure, exec task error, {:?}", e.to_string());
        }
    }

    #[inline]
    fn load_config(scx: &ServerContext, name: &str) -> Result<PluginConfig> {
        let mut cfg = scx.plugins.read_config_with::<PluginConfig>(name, &["urls"])?;
        cfg.merge_urls();
        Ok(cfg)
    }
}

#[async_trait]
impl Plugin for WebHookPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        let tx = self.tx.clone();
        let chan_queue_count = self.chan_queue_count.clone();
        self.register
            .add(
                Type::SessionCreated,
                Box::new(WebHookHandler { tx: tx.clone(), chan_queue_count: chan_queue_count.clone() }),
            )
            .await;
        self.register
            .add(
                Type::SessionTerminated,
                Box::new(WebHookHandler { tx: tx.clone(), chan_queue_count: chan_queue_count.clone() }),
            )
            .await;
        self.register
            .add(
                Type::SessionSubscribed,
                Box::new(WebHookHandler { tx: tx.clone(), chan_queue_count: chan_queue_count.clone() }),
            )
            .await;
        self.register
            .add(
                Type::SessionUnsubscribed,
                Box::new(WebHookHandler { tx: tx.clone(), chan_queue_count: chan_queue_count.clone() }),
            )
            .await;

        self.register
            .add(
                Type::ClientConnect,
                Box::new(WebHookHandler { tx: tx.clone(), chan_queue_count: chan_queue_count.clone() }),
            )
            .await;
        self.register
            .add(
                Type::ClientConnack,
                Box::new(WebHookHandler { tx: tx.clone(), chan_queue_count: chan_queue_count.clone() }),
            )
            .await;
        self.register
            .add(
                Type::ClientConnected,
                Box::new(WebHookHandler { tx: tx.clone(), chan_queue_count: chan_queue_count.clone() }),
            )
            .await;
        self.register
            .add(
                Type::ClientDisconnected,
                Box::new(WebHookHandler { tx: tx.clone(), chan_queue_count: chan_queue_count.clone() }),
            )
            .await;
        self.register
            .add(
                Type::ClientSubscribe,
                Box::new(WebHookHandler { tx: tx.clone(), chan_queue_count: chan_queue_count.clone() }),
            )
            .await;
        self.register
            .add(
                Type::ClientUnsubscribe,
                Box::new(WebHookHandler { tx: tx.clone(), chan_queue_count: chan_queue_count.clone() }),
            )
            .await;

        self.register
            .add(
                Type::MessagePublish,
                Box::new(WebHookHandler { tx: tx.clone(), chan_queue_count: chan_queue_count.clone() }),
            )
            .await;
        self.register
            .add(
                Type::MessageDelivered,
                Box::new(WebHookHandler { tx: tx.clone(), chan_queue_count: chan_queue_count.clone() }),
            )
            .await;
        self.register
            .add(
                Type::MessageAcked,
                Box::new(WebHookHandler { tx: tx.clone(), chan_queue_count: chan_queue_count.clone() }),
            )
            .await;
        self.register
            .add(
                Type::MessageDropped,
                Box::new(WebHookHandler { tx: tx.clone(), chan_queue_count: chan_queue_count.clone() }),
            )
            .await;

        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = Self::load_config(&self.scx, self.name())?;
        let cfg = { self.cfg.read().await.clone() };
        if cfg.worker_threads != new_cfg.worker_threads
            || cfg.queue_capacity != new_cfg.queue_capacity
            || cfg.concurrency_limit != new_cfg.concurrency_limit
        {
            let new_cfg = Arc::new(RwLock::new(new_cfg));
            //restart
            let (new_tx, new_exec) =
                Self::start(&self.scx, new_cfg.clone(), self.writers.clone(), self.chan_queue_count.clone())
                    .await;
            self.exec = new_exec;
            self.cfg = new_cfg;
            *self.tx.write().await = new_tx;
        } else {
            *self.cfg.write().await = new_cfg;
        }
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
        let chan_queue_count = self.chan_queue_count.load(Ordering::SeqCst);
        let exec = &self.exec;
        json!({
            "chan_queue_count": chan_queue_count,
            "task_exec_queue": {
                "active_count": exec.active_count(),
                "waiting_count": exec.waiting_count(),
                "completed_count": exec.completed_count().await,
                "failure_count": fails().count(),
            }
        })
    }
}

static HTTP_CLIENT: Lazy<Result<reqwest::Client>> = Lazy::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| anyhow!(e))
});

type Message = (hook::Type, Option<TopicFilter>, serde_json::Value);

struct WebHookHandler {
    tx: Arc<RwLock<Sender<Message>>>,
    chan_queue_count: Arc<AtomicIsize>,
}

impl WebHookHandler {
    async fn handle(
        cfg: Arc<RwLock<PluginConfig>>,
        writers: HookWriters,
        backoff_strategy: Arc<ExponentialBackoff>,
        typ: hook::Type,
        topic: Option<TopicFilter>,
        body: serde_json::Value,
    ) -> Result<()> {
        let topic = if let Some(topic) = topic { Some(Topic::from_str(&topic)?) } else { None };
        let hook_writes = {
            let cfg = cfg.read().await;
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
                            Arc::new(new_body),
                            cfg.http_timeout,
                        ));
                    } else {
                        let new_body = Arc::new(new_body);
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
        if let Some(mut hook_writes) = hook_writes {
            let c = hook_writes.len();
            match c {
                0 => {}
                1 => {
                    hook_writes.remove(0).await;
                }
                _ => {
                    let _ = futures::future::join_all(hook_writes).await;
                }
            }
        }

        Ok(())
    }

    #[inline]
    async fn write(
        writers: HookWriters,
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
            let writer = writers
                .entry(url.loc.clone())
                .or_insert_with(|| Arc::new(RwLock::new(HookWriter::new(url.loc))))
                .value()
                .clone();
            let mut writer = writer.write().await;
            log::debug!("writer.log start ... ");
            //time::sleep(time::Duration::from_secs(2)).await;
            if let Err(e) = writer.log(data.as_slice()).await {
                log::warn!("write hook message failure, file: {:?}, {:?}", writer.file_name, e);
            }
            log::debug!("writer.log end ... ");
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
        if let Err(e) = retry(backoff_strategy.as_ref().clone(), || async {
            Ok(Self::_http_request(&url.loc, body.clone(), timeout).await?)
        })
        .await
        {
            fails().current_inc();
            log::warn!("send web hook message failure, {:?}", e);
        }
    }

    async fn _http_request(url: &str, body: Arc<serde_json::Value>, timeout: Duration) -> Result<()> {
        log::debug!("http_request, timeout: {:?}, url: {}, body: {}", timeout, url, body);

        let resp = HTTP_CLIENT
            .as_ref()
            .map_err(|e| anyhow!(e))?
            .clone()
            .request(reqwest::Method::POST, url)
            .timeout(timeout)
            .json(body.as_ref())
            .send()
            .await
            .map_err(|e| anyhow!(e))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            Err(anyhow!(format!("response status is not OK, url:{:?}, response:{:?}", url, resp)))
        }
    }
}

#[async_trait]
impl Handler for WebHookHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        let typ = param.get_type();
        let now = chrono::Local::now();
        let now_time = now.format("%Y-%m-%d %H:%M:%S%.3f").to_string();
        let bodys = match param {
            Parameter::ClientConnect(conn_info) => {
                let mut body = conn_info.to_hook_body();
                if let Some(obj) = body.as_object_mut() {
                    obj.insert("time".into(), serde_json::Value::String(now_time));
                }
                Some((None, body))
            }
            Parameter::ClientConnack(conn_info, conn_ack) => {
                let mut body = conn_info.to_hook_body();
                if let Some(obj) = body.as_object_mut() {
                    obj.insert("conn_ack".into(), serde_json::Value::String(conn_ack.reason().to_string()));
                    obj.insert("time".into(), serde_json::Value::String(now_time));
                }
                Some((None, body))
            }

            Parameter::ClientConnected(session) => {
                let mut body = session.connect_info().await.map(|c| c.to_hook_body()).unwrap_or_default();
                if let Some(obj) = body.as_object_mut() {
                    obj.insert(
                        "connected_at".into(),
                        serde_json::Value::Number(serde_json::Number::from(
                            session.connected_at().await.unwrap_or_default(),
                        )),
                    );
                    obj.insert(
                        "session_present".into(),
                        serde_json::Value::Bool(session.session_present().await.unwrap_or_default()),
                    );
                    obj.insert("time".into(), serde_json::Value::String(now_time));
                }
                Some((None, body))
            }

            Parameter::ClientDisconnected(session, reason) => {
                let body = json!({
                    "node": session.id.node(),
                    "ipaddress": session.id.remote_addr,
                    "clientid": session.id.client_id,
                    "username": session.id.username_ref(),
                    "disconnected_at": session.disconnected_at().await.unwrap_or_default(),
                    "reason": reason.to_string(),
                    "time": now_time
                });
                Some((None, body))
            }

            Parameter::ClientSubscribe(session, subscribe) => {
                let body = json!({
                    "node": session.id.node(),
                    "ipaddress": session.id.remote_addr,
                    "clientid": session.id.client_id,
                    "username": session.id.username_ref(),
                    "topic": subscribe.topic_filter,
                    "opts": subscribe.opts.to_json(),
                    "time": now_time
                });
                Some((Some(subscribe.topic_filter.clone()), body))
            }

            Parameter::ClientUnsubscribe(session, unsubscribe) => {
                let body = json!({
                    "node": session.id.node(),
                    "ipaddress": session.id.remote_addr,
                    "clientid": session.id.client_id,
                    "username": session.id.username_ref(),
                    "topic": unsubscribe.topic_filter,
                    "time": now_time
                });
                Some((Some(unsubscribe.topic_filter.clone()), body))
            }

            Parameter::SessionSubscribed(session, subscribe) => {
                let body = json!({
                    "node": session.id.node(),
                    "ipaddress": session.id.remote_addr,
                    "clientid": session.id.client_id,
                    "username": session.id.username_ref(),
                    "topic": subscribe.topic_filter,
                    "opts": subscribe.opts.to_json(),
                    "time": now_time
                });
                Some((Some(subscribe.topic_filter.clone()), body))
            }

            Parameter::SessionUnsubscribed(session, unsubscribed) => {
                let topic = unsubscribed.topic_filter.clone();
                let body = json!({
                    "node": session.id.node(),
                    "ipaddress": session.id.remote_addr,
                    "clientid": session.id.client_id,
                    "username": session.id.username_ref(),
                    "topic": topic,
                    "time": now_time
                });
                Some((Some(topic), body))
            }

            Parameter::SessionCreated(session) => {
                let body = json!({
                    "node": session.id.node(),
                    "ipaddress": session.id.remote_addr,
                    "clientid": session.id.client_id,
                    "username": session.id.username_ref(),
                    "created_at": session.created_at().await.unwrap_or_default(),
                    "time": now_time
                });
                Some((None, body))
            }

            Parameter::SessionTerminated(session, reason) => {
                let body = json!({
                    "node": session.id.node(),
                    "ipaddress": session.id.remote_addr,
                    "clientid": session.id.client_id,
                    "username": session.id.username_ref(),
                    "reason": reason.to_string(),
                    "time": now_time
                });
                Some((None, body))
            }

            Parameter::MessagePublish(_session, from, publish) => {
                let topic = &publish.topic;
                let body = json!({
                    "dup": publish.dup,
                    "retain": publish.retain,
                    "qos": publish.qos.value(),
                    "topic": topic,
                    "packet_id": publish.packet_id,
                    "payload": BASE64_STANDARD.encode(publish.payload.as_ref()),
                    "ts": publish.create_time,
                    "time": now_time
                });
                let body = from.to_from_json(body);
                Some((Some(topic.clone()), body))
            }

            Parameter::MessageDelivered(session, from, publish) => {
                if from.is_system() {
                    None
                } else {
                    let topic = &publish.topic;
                    let body = json!({
                        "dup": publish.dup,
                        "retain": publish.retain,
                        "qos": publish.qos.value(),
                        "topic": topic,
                        "packet_id": publish.packet_id,
                        "payload": BASE64_STANDARD.encode(publish.payload.as_ref()),
                        "pts": publish.create_time,
                        "ts": now.timestamp_millis(),
                        "time": now_time
                    });
                    let body = session.id.to_to_json(body);
                    let body = from.to_from_json(body);
                    Some((Some(topic.clone()), body))
                }
            }

            Parameter::MessageAcked(session, from, publish) => {
                if from.is_system() {
                    None
                } else {
                    let topic = &publish.topic;
                    let body = json!({
                        "dup": publish.dup,
                        "retain": publish.retain,
                        "qos": publish.qos.value(),
                        "topic": topic,
                        "packet_id": publish.packet_id,
                        "payload": BASE64_STANDARD.encode(publish.payload.as_ref()),
                        "pts": publish.create_time,
                        "ts": now.timestamp_millis(),
                        "time": now_time
                    });
                    let body = session.id.to_to_json(body);
                    let body = from.to_from_json(body);
                    Some((Some(topic.clone()), body))
                }
            }

            Parameter::MessageDropped(to, from, publish, reason) => {
                if from.is_system() {
                    None
                } else {
                    let body = json!({
                        "dup": publish.dup,
                        "retain": publish.retain,
                        "qos": publish.qos.value(),
                        "topic": publish.topic,
                        "packet_id": publish.packet_id,
                        "payload": BASE64_STANDARD.encode(publish.payload.as_ref()),
                        "reason": reason.to_string(),
                        "pts": publish.create_time,
                        "ts": now.timestamp_millis(),
                        "time": now_time
                    });
                    let mut body = from.to_from_json(body);
                    if let Some(to) = to {
                        body = to.to_to_json(body);
                    }
                    Some((None, body))
                }
            }
            _ => {
                log::error!("parameter is: {:?}", param);
                None
            }
        };

        log::debug!("bodys: {:?}", bodys);

        if let Some((topic, body)) = bodys {
            let tx = self.tx.read().await.clone();
            if let Err(e) = tx.send((typ, topic, body)).await {
                log::warn!("web-hook send error, typ: {:?}, {:?}", typ, e);
            } else {
                self.chan_queue_count.fetch_add(1, Ordering::SeqCst);
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
    fn new(file: ByteString) -> Self {
        Self { file_name: file.to_string(), file: None }
    }

    #[inline]
    pub async fn log(&mut self, msg: &[u8]) -> std::result::Result<(), Box<dyn std::error::Error>> {
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
    async fn create_dirs(path: &Path) -> std::result::Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            // If the parent directory does not exist, create it recursively.
            if !parent.exists() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        Ok(())
    }
}

#[inline]
fn init_task_exec_queue(workers: usize, queue_max: usize) -> TaskExecQueue {
    let (exec, task_runner) = Builder::default().workers(workers).queue_max(queue_max).build();

    tokio::spawn(async move {
        task_runner.await;
    });

    exec
}

//Failure count
#[inline]
pub(crate) fn fails() -> &'static Counter {
    static INSTANCE: OnceCell<Counter> = OnceCell::new();
    INSTANCE.get_or_init(Counter::new)
}
