use std::convert::From as _;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use event_notify::Event;

use ntex_mqtt::v3::codec::Publish as PublishV3;
use ntex_mqtt::v5::codec::Publish as PublishV5;

use ntex::connect::rustls::TlsConnector;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::CertificateDer;
use rustls::{ClientConfig, RootCertStore};

use anyhow::anyhow;
use bytes::Bytes;
use bytestring::ByteString;
use futures::channel::mpsc;
use futures::SinkExt;
use tokio::sync::RwLock;

use rmqtt::codec::types::{MQTT_LEVEL_31, MQTT_LEVEL_311, MQTT_LEVEL_5};
use rmqtt::codec::v5::{PublishProperties, UserProperties};
use rmqtt::context::ServerContext;
use rmqtt::utils::timestamp_millis;
use rmqtt::{
    session::SessionState,
    types::{ClientId, DashMap, From, Id, NodeId, UserName},
    Result,
};

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
    fn cfg(&self) -> Arc<Bridge> {
        match self {
            BridgeClient::V4(c) => c.cfg.clone(),
            BridgeClient::V5(c) => c.cfg.clone(),
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
    Rc<Event<(BridgeClient, Option<SocketAddr>, Option<SocketAddr>, BridgePublish), ()>>;

type SourceKey = (String, usize, usize); //BridgeName, entry_idx, client_no

#[derive(Clone)]
pub(crate) struct BridgeManager {
    scx: ServerContext,
    node_id: NodeId,
    cfg: Arc<RwLock<PluginConfig>>,
    sources: Arc<DashMap<SourceKey, CommandMailbox>>,
}

impl BridgeManager {
    pub fn new(scx: ServerContext, cfg: Arc<RwLock<PluginConfig>>) -> Self {
        let node_id = scx.node.id();
        Self { scx, node_id, cfg, sources: Arc::new(DashMap::default()) }
    }

    pub async fn start(&mut self) {
        while let Err(e) = self._start().await {
            log::error!("start bridge-ingress-mqtt error, {e:?}");
            self.stop().await;
            tokio::time::sleep(Duration::from_millis(3000)).await;
        }
    }

    async fn _start(&mut self) -> Result<()> {
        let bridges = self.cfg.read().await.bridges.clone();
        for b_cfg in &bridges {
            if !b_cfg.enable {
                continue;
            }
            for (entry_idx, entry) in b_cfg.entries.iter().enumerate() {
                let concurrent_client_limit =
                    if entry.remote.topic.starts_with("$share/") { b_cfg.concurrent_client_limit } else { 1 };
                log::debug!("concurrent_client_limit: {concurrent_client_limit}");

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
        let scx = self.scx.clone();
        Rc::new(
            Event::listen(
                move |(c, remote_addr, local_laddr, p): (
                    BridgeClient,
                    Option<SocketAddr>,
                    Option<SocketAddr>,
                    BridgePublish,
                ),
                      _next| {
                    let scx = scx.clone();

                    let f = From::from_bridge(Id::new(
                        scx.node.id(),
                        local_laddr.map(|a| a.port()).unwrap_or_default(),
                        local_laddr,
                        remote_addr,
                        c.client_id(),
                        Some(c.username()),
                    ));
                    let cfg = c.cfg();
                    let idx = c.entry_idx();
                    ntex::rt::spawn(async move {
                        send_publish(scx, cfg, idx, f, p).await;
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
                "stop bridge_name: {bridge_name:?}, entry_idx: {entry_idx:?}, client_no: {client_no:?}"
            );
            if let Err(e) = mailbox.stop().await {
                log::error!(
                    "stop BridgeMqttIngressPlugin error, bridge_name: {bridge_name}, entry_idx: {entry_idx}, client_no: {client_no}, {e:?}"
                );
            }
        }
        self.sources.clear();
    }

    pub(crate) fn sources(&self) -> &DashMap<SourceKey, CommandMailbox> {
        &self.sources
    }
}

async fn send_publish(scx: ServerContext, cfg: Arc<Bridge>, entry_idx: usize, f: From, p: BridgePublish) {
    log::debug!("from {f:?}, message: {p:?}");
    let entry = if let Some(entry) = cfg.entries.get(entry_idx) { entry } else { unreachable!() };
    let msg = match p {
        BridgePublish::V3(p) => rmqtt::codec::types::Publish {
            dup: false,
            retain: entry.local.make_retain(p.retain),
            qos: entry.local.make_qos(p.qos),
            topic: entry.local.make_topic(p.topic.as_str()),
            packet_id: None,
            payload: Bytes::from(p.payload.to_vec()), //@TODO ...
            properties: Some(PublishProperties::default()),
            delay_interval: None,
            create_time: Some(timestamp_millis()),
        },
        BridgePublish::V5(p) => rmqtt::codec::types::Publish {
            dup: false,
            retain: entry.local.make_retain(p.retain),
            qos: entry.local.make_qos(p.qos),
            topic: entry.local.make_topic(p.topic.as_str()),
            packet_id: None,
            payload: Bytes::from(p.payload.to_vec()),      //@TODO ...
            properties: Some(to_properties(p.properties)), //@TODO ...
            delay_interval: None,
            create_time: Some(timestamp_millis()),
        },
    };

    log::debug!("msg: {msg:?}");

    let expiry_interval = msg
        .properties
        .as_ref()
        .and_then(|p| p.message_expiry_interval.map(|interval| Duration::from_secs(interval.get() as u64)))
        .unwrap_or(cfg.expiry_interval);

    let msg = Box::new(msg);

    //hook, message_publish
    let msg = scx.extends.hook_mgr().message_publish(None, f.clone(), &msg).await.unwrap_or(msg);

    let storage_available = scx.extends.message_mgr().await.enable();

    if let Err(e) = SessionState::forwards(&scx, f, msg, storage_available, Some(expiry_interval)).await {
        log::warn!("{e:?}");
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
        is_utf8_payload: props.is_utf8_payload,
        response_topic: props.response_topic.map(|data| ByteString::from(data.as_str())),
        subscription_ids: props.subscription_ids.clone(),
    }
}

pub(crate) fn build_tls_connector(cfg: &super::config::Bridge) -> Result<TlsConnector<String>> {
    let mut root_store = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.into() };

    if let Some(c) = &cfg.root_cert {
        root_store.add_parsable_certificates(
            CertificateDer::pem_file_iter(c)
                .map_err(|e| anyhow!(e))?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|e| anyhow!(e))?,
        );
    }

    let config = ClientConfig::builder().with_root_certificates(root_store);
    let config = if let (Some(client_key), Some(client_cert)) = (&cfg.client_key, &cfg.client_cert) {
        let c_certs = CertificateDer::pem_file_iter(client_cert)
            .map_err(|e| anyhow!(e))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| anyhow!(e))?;
        let c_key = rustls::pki_types::PrivateKeyDer::from_pem_file(client_key).map_err(|e| anyhow!(e))?;
        config.with_client_auth_cert(c_certs, c_key).map_err(|e| anyhow!(e))?
    } else {
        config.with_no_client_auth()
    };
    Ok(TlsConnector::new(config))
}
