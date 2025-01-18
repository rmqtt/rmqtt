use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use ntex_mqtt::v3::codec::Publish as PublishV3;
use ntex_mqtt::v5::codec::Publish as PublishV5;

use ntex::connect::rustls::TlsConnector;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::CertificateDer;
use rustls::{ClientConfig, RootCertStore};

use rmqtt::anyhow::anyhow;
use rmqtt::bytestring::ByteString;
use rmqtt::futures::channel::mpsc;
use rmqtt::futures::SinkExt;
use rmqtt::{
    broker::topic::{TopicTree, VecToTopic},
    rand, ClientId, From, MqttError, NodeId, Publish, PublishProperties, Result, Topic,
};
use rmqtt::{log, rustls, tokio, tokio::sync::RwLock, DashMap};

use rmqtt::ntex_mqtt::types::{MQTT_LEVEL_31, MQTT_LEVEL_311, MQTT_LEVEL_5};

use crate::config::{Bridge, Entry, PluginConfig};
use crate::v4::Client as ClientV4;
use crate::v5::Client as ClientV5;

#[derive(Debug)]
pub enum Command {
    Connect,
    Publish(BridgePublish),
    Close,
}

#[derive(Clone)]
pub struct CommandMailbox {
    pub(crate) cfg: Arc<Bridge>,
    pub(crate) client_id: ClientId,
    cmd_tx: mpsc::Sender<Command>,
}

impl CommandMailbox {
    pub(crate) fn new(cfg: Arc<Bridge>, client_id: ClientId, cmd_tx: mpsc::Sender<Command>) -> Self {
        CommandMailbox { cfg, client_id, cmd_tx }
    }

    #[inline]
    pub(crate) async fn send(&self, cmd: Command) -> Result<()> {
        self.cmd_tx.clone().send(cmd).await.map_err(|e| anyhow!(e))?;
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

pub(crate) type BridgeName = ByteString;
type SourceKey = (BridgeName, EntryIndex);

type EntryIndex = usize;

type MqttVer = u8;

#[derive(Clone)]
pub(crate) struct BridgeManager {
    node_id: NodeId,
    cfg: Arc<RwLock<PluginConfig>>,
    sinks: Arc<DashMap<SourceKey, Vec<CommandMailbox>>>,
    topics: Arc<RwLock<TopicTree<(BridgeName, EntryIndex, MqttVer)>>>,
}

impl BridgeManager {
    pub fn new(node_id: NodeId, cfg: Arc<RwLock<PluginConfig>>) -> Self {
        Self {
            node_id,
            cfg,
            sinks: Arc::new(DashMap::default()),
            topics: Arc::new(RwLock::new(TopicTree::default())),
        }
    }

    pub async fn start(&mut self) {
        while let Err(e) = self._start().await {
            log::error!("start bridge-egress-mqtt error, {:?}", e);
            self.stop().await;
            tokio::time::sleep(Duration::from_millis(3000)).await;
        }
    }

    async fn _start(&mut self) -> Result<()> {
        let mut topics = self.topics.write().await;
        let bridges = self.cfg.read().await.bridges.clone();
        let mut bridge_names: HashSet<&str> = HashSet::default();
        for b_cfg in &bridges {
            if !b_cfg.enable {
                continue;
            }
            if bridge_names.contains(&b_cfg.name as &str) {
                return Err(MqttError::from(format!("The bridge name already exists! {:?}", b_cfg.name)));
            }

            bridge_names.insert(&b_cfg.name);
            for (entry_idx, entry) in b_cfg.entries.iter().enumerate() {
                log::debug!("entry.local.topic_filter: {}", entry.local.topic_filter);
                topics.insert(
                    &Topic::from_str(entry.local.topic_filter.as_str())?,
                    (b_cfg.name.clone(), entry_idx, b_cfg.mqtt_ver.level()),
                );
                for client_no in 0..b_cfg.concurrent_client_limit {
                    match b_cfg.mqtt_ver.level() {
                        MQTT_LEVEL_311 => {
                            let mailbox =
                                ClientV4::connect(b_cfg.clone(), entry_idx, self.node_id, client_no)?;
                            self.sinks.entry((b_cfg.name.clone(), entry_idx)).or_default().push(mailbox);
                        }
                        MQTT_LEVEL_5 => {
                            let mailbox =
                                ClientV5::connect(b_cfg.clone(), entry_idx, self.node_id, client_no)?;
                            self.sinks.entry((b_cfg.name.clone(), entry_idx)).or_default().push(mailbox);
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

    pub async fn stop(&mut self) {
        for mut entry in &mut self.sinks.iter_mut() {
            let ((bridge_name, entry_idx), mailboxs) = entry.pair_mut();
            for (client_no, mailbox) in mailboxs.iter_mut().enumerate() {
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
        }
        self.sinks.clear();
    }

    pub(crate) fn sinks(&self) -> &DashMap<SourceKey, Vec<CommandMailbox>> {
        &self.sinks
    }

    #[inline]
    pub(crate) async fn send(&self, _f: &From, p: &Publish) -> Result<()> {
        let topic = Topic::from_str(&p.topic)?;
        let rnd = rand::random::<usize>();
        for (topic_filter, bridge_infos) in { self.topics.read().await.matches(&topic) }.iter() {
            let topic_filter = topic_filter.to_topic_filter();
            log::debug!("topic_filter: {:?}", topic_filter);
            log::debug!("bridge_infos: {:?}", bridge_infos);
            for (name, entry_idx, mqtt_ver) in bridge_infos {
                if let Some(mailboxs) = self.sinks.get(&(name.clone(), *entry_idx)) {
                    let client_no = rnd % mailboxs.len();
                    if let Some(mailbox) = mailboxs.get(client_no) {
                        let entry = if let Some(entry) = mailbox.cfg.entries.get(*entry_idx) {
                            entry
                        } else {
                            log::error!("unreachable!(), entry_idx: {}", *entry_idx);
                            continue;
                        };
                        match *mqtt_ver {
                            MQTT_LEVEL_311 => {
                                if let Err(e) = mailbox
                                    .send(Command::Publish(BridgePublish::V3(self.to_v3_publish(entry, p))))
                                    .await
                                {
                                    log::warn!("{}", e);
                                }
                            }
                            MQTT_LEVEL_5 => {
                                if let Err(e) = mailbox
                                    .send(Command::Publish(BridgePublish::V5(self.to_v5_publish(entry, p))))
                                    .await
                                {
                                    log::warn!("{}", e);
                                }
                            }
                            MQTT_LEVEL_31 => {
                                log::warn!("Connection to MQTT 3.1 broker not implemented!")
                            }
                            _ => {
                                log::error!("Wrong MQTT version, {}", *mqtt_ver)
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    #[inline]
    fn to_v3_publish(&self, cfg_entry: &Entry, p: &Publish) -> PublishV3 {
        PublishV3 {
            dup: false,
            retain: cfg_entry.remote.make_retain(p.retain),
            qos: cfg_entry.remote.make_qos(p.qos),
            topic: cfg_entry.remote.make_topic(&p.topic),
            packet_id: None,
            payload: ntex::util::Bytes::from(p.payload.to_vec()), //@TODO ...
        }
    }

    #[inline]
    fn to_v5_publish(&self, cfg_entry: &Entry, p: &Publish) -> PublishV5 {
        PublishV5 {
            dup: false,
            retain: cfg_entry.remote.make_retain(p.retain),
            qos: cfg_entry.remote.make_qos(p.qos),
            topic: cfg_entry.remote.make_topic(&p.topic),
            packet_id: None,
            payload: ntex::util::Bytes::from(p.payload.to_vec()), //@TODO ...
            properties: to_properties(&p.properties),
        }
    }
}

#[inline]
fn to_properties(props: &PublishProperties) -> ntex_mqtt::v5::codec::PublishProperties {
    let user_properties: ntex_mqtt::v5::codec::UserProperties = props
        .user_properties
        .iter()
        .map(|(k, v)| (ntex::util::ByteString::from(k.as_ref()), ntex::util::ByteString::from(v.as_ref())))
        .collect();
    ntex_mqtt::v5::codec::PublishProperties {
        topic_alias: props.topic_alias,
        correlation_data: props.correlation_data.as_ref().map(|data| ntex::util::Bytes::from(data.to_vec())),
        message_expiry_interval: props.message_expiry_interval,
        content_type: props.content_type.as_ref().map(|data| ntex::util::ByteString::from(data.as_ref())),
        user_properties,
        is_utf8_payload: props.is_utf8_payload.unwrap_or_default(),
        response_topic: props.response_topic.as_ref().map(|data| ntex::util::ByteString::from(data.as_ref())),
        subscription_ids: props.subscription_ids.clone().unwrap_or_default(),
    }
}

pub(crate) fn build_tls_connector(cfg: &super::config::Bridge) -> Result<TlsConnector<String>> {
    let mut root_store = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.into() };

    if let Some(c) = &cfg.root_cert {
        root_store.add_parsable_certificates(
            CertificateDer::pem_file_iter(c)
                .map_err(|e| anyhow!(e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| anyhow!(e))?,
        );
    }

    let config = ClientConfig::builder().with_root_certificates(root_store);
    let config = if let (Some(client_key), Some(client_cert)) = (&cfg.client_key, &cfg.client_cert) {
        let c_certs = CertificateDer::pem_file_iter(client_cert)
            .map_err(|e| anyhow!(e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow!(e))?;
        let c_key = rustls::pki_types::PrivateKeyDer::from_pem_file(client_key).map_err(|e| anyhow!(e))?;
        config.with_client_auth_cert(c_certs, c_key).map_err(|e| MqttError::Anyhow(e.into()))?
    } else {
        config.with_no_client_auth()
    };
    Ok(TlsConnector::new(config))
}
