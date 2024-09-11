use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use event_notify::Event;

use ntex_mqtt::v3::codec::Publish as PublishV3;
use ntex_mqtt::v5::codec::Publish as PublishV5;

use rmqtt::anyhow::anyhow;
use rmqtt::bytestring::ByteString;
use rmqtt::futures::channel::mpsc;
use rmqtt::futures::SinkExt;
use rmqtt::{bytes::Bytes, log, timestamp_millis, tokio::sync::RwLock, ClientId, DashMap, UserName};
use rmqtt::{From, Id, NodeId, Publish, PublishProperties, Result, Runtime, SessionState, UserProperties};

use rmqtt::ntex_mqtt::types::{MQTT_LEVEL_31, MQTT_LEVEL_311, MQTT_LEVEL_5};

use crate::config::{Bridge, PluginConfig};
use crate::v4::Client as ClientV4;
use crate::v5::Client as ClientV5;

#[derive(Debug)]
pub enum Command {
    Connect,
    Close,
}

#[derive(Clone)]
pub struct CommandMailbox {
    pub(crate) client_id: ClientId,
    cmd_tx: mpsc::Sender<Command>,
}

impl CommandMailbox {
    pub(crate) fn new(client_id: ClientId, cmd_tx: mpsc::Sender<Command>) -> Self {
        CommandMailbox { client_id, cmd_tx }
    }

    #[inline]
    pub(crate) async fn send(&mut self, cmd: Command) -> Result<()> {
        self.cmd_tx.send(cmd).await.map_err(|e| anyhow!(e))?;
        Ok(())
    }

    #[inline]
    pub(crate) async fn stop(&mut self) -> Result<()> {
        self.send(Command::Close).await
    }
}

#[derive(Debug)]
pub enum BridgePublish {
    V3(PublishV3),
    V5(PublishV5),
}

#[derive(Clone)]
pub enum BridgeClient {
    V4(ClientV4),
    V5(ClientV5),
}

impl BridgeClient {
    fn cfg(&self) -> &Bridge {
        match self {
            BridgeClient::V4(c) => c.cfg.as_ref(),
            BridgeClient::V5(c) => c.cfg.as_ref(),
        }
    }

    fn entry_idx(&self) -> usize {
        match self {
            BridgeClient::V4(c) => c.entry_idx,
            BridgeClient::V5(c) => c.entry_idx,
        }
    }

    fn client_id(&self) -> ClientId {
        match self {
            BridgeClient::V4(c) => c.client_id.clone(),
            BridgeClient::V5(c) => c.client_id.clone(),
        }
    }

    fn username(&self) -> UserName {
        match self {
            BridgeClient::V4(c) => c.username.clone(),
            BridgeClient::V5(c) => c.username.clone(),
        }
    }
}

pub type OnMessageEvent =
    Arc<Event<(BridgeClient, Option<SocketAddr>, Option<SocketAddr>, BridgePublish), ()>>;

type SourceKey = (String, usize, usize); //BridgeName, entry_idx, client_no

#[derive(Clone)]
pub(crate) struct BridgeManager {
    node_id: NodeId,
    cfg: Arc<RwLock<PluginConfig>>,
    sources: Arc<DashMap<SourceKey, CommandMailbox>>,
}

impl BridgeManager {
    pub fn new(node_id: NodeId, cfg: Arc<RwLock<PluginConfig>>) -> Self {
        Self { node_id, cfg, sources: Arc::new(DashMap::default()) }
    }

    pub async fn start(&mut self) -> Result<()> {
        let bridges = self.cfg.read().await.bridges.clone();
        for b_cfg in &bridges {
            if !b_cfg.enable {
                continue;
            }
            for (entry_idx, entry) in b_cfg.entries.iter().enumerate() {
                let concurrent_client_limit =
                    if entry.remote.topic.starts_with("$share/") { b_cfg.concurrent_client_limit } else { 1 };
                log::debug!("concurrent_client_limit: {}", concurrent_client_limit);

                for client_no in 0..concurrent_client_limit {
                    match b_cfg.mqtt_ver.level() {
                        MQTT_LEVEL_311 => {
                            let mailbox = ClientV4::connect(
                                b_cfg.clone(),
                                entry_idx,
                                self.node_id,
                                client_no,
                                self.on_message(),
                            )?;
                            self.sources.insert((b_cfg.name.clone(), entry_idx, client_no), mailbox);
                        }
                        MQTT_LEVEL_5 => {
                            let mailbox = ClientV5::connect(
                                b_cfg.clone(),
                                entry_idx,
                                self.node_id,
                                client_no,
                                self.on_message(),
                            )?;
                            self.sources.insert((b_cfg.name.clone(), entry_idx, client_no), mailbox);
                        }
                        MQTT_LEVEL_31 => {
                            log::warn!("Connection to MQTT 3.1 broker not implemented!")
                        }
                        _ => {
                            log::error!("Wrong MQTT version, {}", b_cfg.mqtt_ver.level())
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn on_message(&self) -> OnMessageEvent {
        Arc::new(
            Event::listen(
                |(c, remote_addr, local_laddr, p): (
                    BridgeClient,
                    Option<SocketAddr>,
                    Option<SocketAddr>,
                    BridgePublish,
                ),
                 _next| {
                    ntex::rt::spawn(async move {
                        send_publish(c, p, remote_addr, local_laddr).await;
                    });
                },
            )
            .finish(),
        )
    }

    pub async fn stop(&mut self) {
        for mut entry in &mut self.sources.iter_mut() {
            let ((bridge_name, entry_idx, client_no), mailbox) = entry.pair_mut();
            log::debug!(
                "stop bridge_name: {:?}, entry_idx: {:?}, client_no: {:?}",
                bridge_name,
                entry_idx,
                client_no
            );
            if let Err(e) = mailbox.stop().await {
                log::error!(
                    "stop BridgeMqttIngressPlugin error, bridge_name: {}, entry_idx: {}, client_no: {}, {:?}",
                    bridge_name,
                    entry_idx,
                    client_no,
                    e
                );
            }
        }
        self.sources.clear();
    }

    pub(crate) fn sources(&self) -> &DashMap<SourceKey, CommandMailbox> {
        &self.sources
    }
}

async fn send_publish(
    c: BridgeClient,
    p: BridgePublish,
    remote_addr: Option<SocketAddr>,
    local_laddr: Option<SocketAddr>,
) {
    let from = From::from_bridge(Id::new(
        Runtime::instance().node.id(),
        local_laddr,
        remote_addr,
        c.client_id(),
        Some(c.username()),
    ));
    log::debug!("from {:?}, message: {:?}", from, p);
    let cfg = c.cfg();
    let entry = if let Some(entry) = cfg.entries.get(c.entry_idx()) { entry } else { unreachable!() };
    let msg = match p {
        BridgePublish::V3(p) => Publish {
            dup: false,
            retain: entry.local.make_retain(p.retain),
            qos: entry.local.make_qos(p.qos),
            topic: entry.local.make_topic(p.topic.as_str()),
            packet_id: None,
            payload: Bytes::from(p.payload.to_vec()), //@TODO ...
            properties: PublishProperties::default(),
            delay_interval: None,
            create_time: timestamp_millis(),
        },
        BridgePublish::V5(p) => Publish {
            dup: false,
            retain: entry.local.make_retain(p.retain),
            qos: entry.local.make_qos(p.qos),
            topic: entry.local.make_topic(p.topic.as_str()),
            packet_id: None,
            payload: Bytes::from(p.payload.to_vec()), //@TODO ...
            properties: to_properties(p.properties),  //@TODO ...
            delay_interval: None,
            create_time: timestamp_millis(),
        },
    };

    log::debug!("msg: {:?}", msg);

    let expiry_interval = msg
        .properties
        .message_expiry_interval
        .map(|interval| Duration::from_secs(interval.get() as u64))
        .unwrap_or(cfg.expiry_interval);

    //hook, message_publish
    let msg = Runtime::instance()
        .extends
        .hook_mgr()
        .await
        .message_publish(None, from.clone(), &msg)
        .await
        .unwrap_or(msg);

    let storage_available = Runtime::instance().extends.message_mgr().await.enable();

    if let Err(e) =
        SessionState::forwards(from, msg, cfg.retain_available, storage_available, Some(expiry_interval))
            .await
    {
        log::warn!("{:?}", e);
    }
}

#[inline]
fn to_properties(props: ntex_mqtt::v5::codec::PublishProperties) -> PublishProperties {
    let user_properties: UserProperties = props
        .user_properties
        .into_iter()
        .map(|(k, v)| (ByteString::from(k.as_str()), ByteString::from(v.as_str())))
        .collect();
    PublishProperties {
        topic_alias: props.topic_alias,
        correlation_data: props.correlation_data.map(|data| Bytes::from(data.to_vec())),
        message_expiry_interval: props.message_expiry_interval,
        content_type: props.content_type.map(|data| ByteString::from(data.as_str())),
        user_properties,
        is_utf8_payload: Some(props.is_utf8_payload),
        response_topic: props.response_topic.map(|data| ByteString::from(data.as_str())),
        subscription_ids: Some(props.subscription_ids.clone()),
    }
}
