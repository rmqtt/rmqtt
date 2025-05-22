#![deny(unsafe_code)]

use std::convert::From as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use serde_json::{self, json};
use tokio::{spawn, sync::RwLock, time::sleep};

use rmqtt::{
    codec::v5::PublishProperties,
    context::ServerContext,
    hook::{Handler, HookResult, Parameter, Register, ReturnType, Type},
    macros::Plugin,
    plugin::{PackageInfo, Plugin},
    register,
    session::SessionState,
    types::{ClientId, From, Id, NodeId, QoS, TopicName, UserName},
    utils::timestamp_millis,
    Result,
};

use config::PluginConfig;

mod config;

register!(SystemTopicPlugin::new);

#[derive(Plugin)]
struct SystemTopicPlugin {
    scx: ServerContext,
    register: Box<dyn Register>,
    cfg: Arc<RwLock<PluginConfig>>,
    running: Arc<AtomicBool>,
}

impl SystemTopicPlugin {
    #[inline]
    async fn new<N: Into<String>>(scx: ServerContext, name: N) -> Result<Self> {
        let name = name.into();
        let cfg = scx.plugins.read_config_default::<PluginConfig>(&name)?;
        log::debug!("{} SystemTopicPlugin cfg: {:?}", name, cfg);
        let register = scx.extends.hook_mgr().register();
        let cfg = Arc::new(RwLock::new(cfg));
        let running = Arc::new(AtomicBool::new(false));
        Ok(Self { scx, register, cfg, running })
    }

    fn start(scx: ServerContext, cfg: Arc<RwLock<PluginConfig>>, running: Arc<AtomicBool>) {
        spawn(async move {
            let min = Duration::from_secs(1);
            loop {
                let (publish_interval, publish_qos, expiry_interval) = {
                    let cfg_rl = cfg.read().await;
                    (cfg_rl.publish_interval, cfg_rl.publish_qos, cfg_rl.message_expiry_interval)
                };

                let publish_interval = if publish_interval < min { min } else { publish_interval };
                sleep(publish_interval).await;
                if running.load(Ordering::SeqCst) {
                    Self::send_stats(&scx, publish_qos, expiry_interval).await;
                    Self::send_metrics(&scx, publish_qos, expiry_interval).await;
                }
            }
        });
    }

    //Statistics
    //$SYS/brokers/${node}/stats
    async fn send_stats(scx: &ServerContext, publish_qos: QoS, expiry_interval: Duration) {
        let payload = scx.stats.clone(scx).await.to_json(scx).await;
        let nodeid = scx.node.id();
        let topic = format!("$SYS/brokers/{}/stats", nodeid);
        sys_publish(scx.clone(), nodeid, topic, publish_qos, payload, expiry_interval).await;
    }

    //Metrics
    //$SYS/brokers/${node}/metrics
    async fn send_metrics(scx: &ServerContext, publish_qos: QoS, expiry_interval: Duration) {
        let payload = scx.metrics.to_json();
        let nodeid = scx.node.id();
        let topic = format!("$SYS/brokers/{}/metrics", nodeid);
        sys_publish(scx.clone(), nodeid, topic, publish_qos, payload, expiry_interval).await;
    }
}

#[async_trait]
impl Plugin for SystemTopicPlugin {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name());
        let cfg = &self.cfg;

        self.register.add(Type::SessionCreated, Box::new(SystemTopicHandler::new(&self.scx, cfg))).await;
        self.register.add(Type::SessionTerminated, Box::new(SystemTopicHandler::new(&self.scx, cfg))).await;
        self.register.add(Type::ClientConnected, Box::new(SystemTopicHandler::new(&self.scx, cfg))).await;
        self.register.add(Type::ClientDisconnected, Box::new(SystemTopicHandler::new(&self.scx, cfg))).await;
        self.register.add(Type::SessionSubscribed, Box::new(SystemTopicHandler::new(&self.scx, cfg))).await;
        self.register.add(Type::SessionUnsubscribed, Box::new(SystemTopicHandler::new(&self.scx, cfg))).await;

        Self::start(self.scx.clone(), self.cfg.clone(), self.running.clone());
        Ok(())
    }

    #[inline]
    async fn get_config(&self) -> Result<serde_json::Value> {
        self.cfg.read().await.to_json()
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        let new_cfg = self.scx.plugins.read_config_default::<PluginConfig>(self.name())?;
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
    scx: ServerContext,
    cfg: Arc<RwLock<PluginConfig>>,
    nodeid: NodeId,
}

impl SystemTopicHandler {
    fn new(scx: &ServerContext, cfg: &Arc<RwLock<PluginConfig>>) -> Self {
        Self { scx: scx.clone(), cfg: cfg.clone(), nodeid: scx.node.id() }
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

            _ => {
                log::error!("unimplemented, {:?}", param);
                None
            }
        } {
            let nodeid = self.nodeid;
            let (publish_qos, expiry_interval) = {
                let cfg_rl = self.cfg.read().await;
                (cfg_rl.publish_qos, cfg_rl.message_expiry_interval)
            };

            let scx = self.scx.clone();
            spawn(sys_publish(scx, nodeid, topic, publish_qos, payload, expiry_interval));
        }
        (true, acc)
    }
}

#[inline]
async fn sys_publish(
    scx: ServerContext,
    nodeid: NodeId,
    topic: String,
    publish_qos: QoS,
    payload: serde_json::Value,
    message_expiry_interval: Duration,
) {
    match serde_json::to_string(&payload) {
        Ok(payload) => {
            let from = From::from_system(Id::new(
                nodeid,
                0,
                None,
                None,
                ClientId::from_static("system"),
                Some(UserName::from("system")),
            ));

            let p = Box::new(rmqtt::codec::types::Publish {
                dup: false,
                retain: false,
                qos: publish_qos,
                topic: TopicName::from(topic),
                packet_id: None,
                payload: Bytes::from(payload),
                properties: Some(PublishProperties::default()),
                delay_interval: None,
                create_time: Some(timestamp_millis()),
            });

            //hook, message_publish
            let p = scx.extends.hook_mgr().message_publish(None, from.clone(), &p).await.unwrap_or(p);

            let storage_available = scx.extends.message_mgr().await.enable();

            if let Err(e) =
                SessionState::forwards(&scx, from, p, storage_available, Some(message_expiry_interval)).await
            {
                log::warn!("{:?}", e);
            }
        }
        Err(e) => {
            log::error!("{:?}", e);
        }
    }
}
