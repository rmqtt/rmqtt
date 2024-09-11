#![deny(unsafe_code)]
#[macro_use]
extern crate serde;

#[macro_use]
extern crate rmqtt_macros;

use config::PluginConfig;
use rmqtt::{
    async_trait::async_trait,
    base64::prelude::{Engine, BASE64_STANDARD},
    bytes::Bytes,
    chrono, log,
    serde_json::{self, json},
    tokio::spawn,
    tokio::sync::RwLock,
    tokio::time::sleep,
};
use rmqtt::{
    broker::hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    broker::types::{From, Id, QoSEx},
    plugin::{PackageInfo, Plugin},
    register, timestamp_millis, ClientId, NodeId, Publish, PublishProperties, QoS, Result, Runtime,
    SessionState, TopicName, UserName,
};
use std::convert::From as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

mod config;

register!(SystemTopicPlugin::new);

#[derive(Plugin)]
struct SystemTopicPlugin {
    runtime: &'static Runtime,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
    running: Arc<AtomicBool>,
}

impl SystemTopicPlugin {
    #[inline]
    async fn new<N: Into<String>>(runtime: &'static Runtime, name: N) -> Result<Self> {
        let name = name.into();
        let cfg = runtime.settings.plugins.load_config_default::<PluginConfig>(&name)?;
        log::debug!("{} SystemTopicPlugin cfg: {:?}", name, cfg);
        let register = runtime.extends.hook_mgr().await.register();
        let cfg = Arc::new(RwLock::new(cfg));
        let running = Arc::new(AtomicBool::new(false));
        Ok(Self { runtime, register, cfg, running })
    }

    fn start(runtime: &'static Runtime, cfg: Arc<RwLock<PluginConfig>>, running: Arc<AtomicBool>) {
        spawn(async move {
            let min = Duration::from_secs(1);
            loop {
                let (publish_interval, publish_qos, retain_available, expiry_interval) = {
                    let cfg_rl = cfg.read().await;
                    (
                        cfg_rl.publish_interval,
                        cfg_rl.publish_qos,
                        cfg_rl.message_retain_available,
                        cfg_rl.message_expiry_interval,
                    )
                };

                let publish_interval = if publish_interval < min { min } else { publish_interval };
                sleep(publish_interval).await;
                if running.load(Ordering::SeqCst) {
                    Self::send_stats(runtime, publish_qos, retain_available, expiry_interval).await;
                    Self::send_metrics(runtime, publish_qos, retain_available, expiry_interval).await;
                }
            }
        });
    }

    //Statistics
    //$SYS/brokers/${node}/stats
    async fn send_stats(
        runtime: &'static Runtime,
        publish_qos: QoS,
        retain_available: bool,
        expiry_interval: Duration,
    ) {
        let payload = runtime.stats.clone().await.to_json().await;
        let nodeid = runtime.node.id();
        let topic = format!("$SYS/brokers/{}/stats", nodeid);
        sys_publish(nodeid, topic, publish_qos, payload, retain_available, expiry_interval).await;
    }

    //Metrics
    //$SYS/brokers/${node}/metrics
    async fn send_metrics(
        runtime: &'static Runtime,
        publish_qos: QoS,
        retain_available: bool,
        expiry_interval: Duration,
    ) {
        let payload = Runtime::instance().metrics.to_json();
        let nodeid = runtime.node.id();
        let topic = format!("$SYS/brokers/{}/metrics", nodeid);
        sys_publish(nodeid, topic, publish_qos, payload, retain_available, expiry_interval).await;
    }
}

#[async_trait]
impl Plugin for SystemTopicPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        let cfg = &self.cfg;

        self.register.add(Type::SessionCreated, Box::new(SystemTopicHandler::new(cfg))).await;
        self.register.add(Type::SessionTerminated, Box::new(SystemTopicHandler::new(cfg))).await;
        self.register.add(Type::ClientConnected, Box::new(SystemTopicHandler::new(cfg))).await;
        self.register.add(Type::ClientDisconnected, Box::new(SystemTopicHandler::new(cfg))).await;
        self.register.add(Type::SessionSubscribed, Box::new(SystemTopicHandler::new(cfg))).await;
        self.register.add(Type::SessionUnsubscribed, Box::new(SystemTopicHandler::new(cfg))).await;
        self.register.add(Type::MessageDropped, Box::new(SystemTopicHandler::new(cfg))).await;

        Self::start(self.runtime, self.cfg.clone(), self.running.clone());
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
        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::info!("{} stop", self.name());
        self.register.stop().await;
        self.running.store(false, Ordering::SeqCst);
        Ok(false)
    }
}

struct SystemTopicHandler {
    cfg: Arc<RwLock<PluginConfig>>,
    //    message_type: MessageType,
    nodeid: NodeId,
}

impl SystemTopicHandler {
    fn new(cfg: &Arc<RwLock<PluginConfig>>) -> Self {
        let nodeid = Runtime::instance().node.id();
        Self { cfg: cfg.clone(), nodeid }
    }
}

#[async_trait]
impl Handler for SystemTopicHandler {
    async fn hook(&self, param: &Parameter, acc: Option<HookResult>) -> ReturnType {
        log::debug!("param: {:?}, acc: {:?}", param, acc);
        let now = chrono::Local::now();
        let now_time = now.format("%Y-%m-%d %H:%M:%S%.3f").to_string();
        if let Some((topic, payload)) = match param {
            Parameter::SessionCreated(session) => {
                let body = json!({
                    "node": session.id.node(),
                    "ipaddress": session.id.remote_addr,
                    "clientid": session.id.client_id,
                    "username": session.id.username_ref(),
                    "created_at": session.created_at().await.unwrap_or_default(),
                    "time": now_time
                });
                let topic = format!("$SYS/brokers/{}/session/{}/created", self.nodeid, session.id.client_id);
                Some((topic, body))
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
                let topic =
                    format!("$SYS/brokers/{}/session/{}/terminated", self.nodeid, session.id.client_id);
                Some((topic, body))
            }
            Parameter::ClientConnected(session) => {
                let mut body = session
                    .connect_info()
                    .await
                    .map(|connect_info| connect_info.to_hook_body())
                    .unwrap_or_default();
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
                let topic =
                    format!("$SYS/brokers/{}/clients/{}/connected", self.nodeid, session.id.client_id);
                Some((topic, body))
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
                let topic =
                    format!("$SYS/brokers/{}/clients/{}/disconnected", self.nodeid, session.id.client_id);
                Some((topic, body))
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
                let topic =
                    format!("$SYS/brokers/{}/session/{}/subscribed", self.nodeid, session.id.client_id);
                Some((topic, body))
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
                let topic =
                    format!("$SYS/brokers/{}/session/{}/unsubscribed", self.nodeid, session.id.client_id);
                Some((topic, body))
            }

            Parameter::MessageDropped(to, from, publish, reason) => {
                let body = json!({
                    "dup": publish.dup(),
                    "retain": publish.retain(),
                    "qos": publish.qos().value(),
                    "topic": publish.topic(),
                    "packet_id": publish.packet_id(),
                    "payload": BASE64_STANDARD.encode(publish.payload()),
                    "reason": reason.to_string(),
                    "pts": publish.create_time(),
                    "ts": now.timestamp_millis(),
                    "time": now_time
                });
                let mut body = from.to_from_json(body);
                if let Some(to) = to {
                    body = to.to_to_json(body);
                }
                let topic = format!("$SYS/brokers/{}/message/dropped", self.nodeid);
                Some((topic, body))
            }

            _ => {
                log::error!("unimplemented, {:?}", param);
                None
            }
        } {
            let nodeid = self.nodeid;
            let (publish_qos, retain_available, expiry_interval) = {
                let cfg_rl = self.cfg.read().await;
                (cfg_rl.publish_qos, cfg_rl.message_retain_available, cfg_rl.message_expiry_interval)
            };

            spawn(sys_publish(nodeid, topic, publish_qos, payload, retain_available, expiry_interval));
        }
        (true, acc)
    }
}

#[inline]
async fn sys_publish(
    nodeid: NodeId,
    topic: String,
    publish_qos: QoS,
    payload: serde_json::Value,
    retain_available: bool,
    message_expiry_interval: Duration,
) {
    match serde_json::to_string(&payload) {
        Ok(payload) => {
            let from = From::from_system(Id::new(
                nodeid,
                None,
                None,
                ClientId::from_static("system"),
                Some(UserName::from("system")),
            ));

            let p = Publish {
                dup: false,
                retain: false,
                qos: publish_qos,
                topic: TopicName::from(topic),
                packet_id: None,
                payload: Bytes::from(payload),
                properties: PublishProperties::default(),
                delay_interval: None,
                create_time: timestamp_millis(),
            };

            //hook, message_publish
            let p = Runtime::instance()
                .extends
                .hook_mgr()
                .await
                .message_publish(None, from.clone(), &p)
                .await
                .unwrap_or(p);

            let storage_available = Runtime::instance().extends.message_mgr().await.enable();

            if let Err(e) = SessionState::forwards(
                from,
                p,
                retain_available,
                storage_available,
                Some(message_expiry_interval),
            )
            .await
            {
                log::warn!("{:?}", e);
            }
        }
        Err(e) => {
            log::error!("{:?}", e);
        }
    }
}
