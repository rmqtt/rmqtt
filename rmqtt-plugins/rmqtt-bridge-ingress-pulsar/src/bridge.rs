use std::collections::{BTreeMap, HashSet};
use std::convert::From as _;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use bytestring::ByteString;
use event_notify::Event;
use futures::StreamExt;
use itertools::Itertools;
use pulsar::{authentication::oauth2::OAuth2Authentication, consumer, Authentication, Pulsar, TokioExecutor};
use tokio::sync::mpsc;
use tokio::sync::RwLock;

use rmqtt::{
    codec::v5::{PublishProperties, UserProperties},
    context::ServerContext,
    session::SessionState,
    types::{
        ClientId, CodecPublish, DashMap, From, Id, NodeId, Publish, QoS, TimestampMillis, TopicName, UserName,
    },
    utils::timestamp_millis,
    Result,
};

use crate::config::{AuthName, Bridge, Entry, PayloadData, PayloadFormat, PluginConfig};

type ExpiryInterval = Duration;

pub type MessageType = (From, Publish, ExpiryInterval);
pub type OnMessageEvent = Arc<Event<MessageType, ()>>;

#[derive(Debug)]
struct Message {
    f: From,
    payload: PayloadData,
    topics: Vec<TopicName>,
    retain: bool,
    qos: QoS,
    properties: PublishProperties,
    delay_interval_s: Option<u32>,
    create_time: TimestampMillis,
}

impl Message {
    fn deserialize_message(
        node_id: NodeId,
        msg: &consumer::Message<Vec<u8>>,
        cfg_entry: &Entry,
    ) -> Result<Message> {
        let payload = &msg.payload;
        let mut from_node = None;
        let mut from_ipaddress = None;
        let mut from_clientid = None;
        let mut from_username = None;
        let mut retain = None;
        let mut qos = None;
        let mut props_topics: BTreeMap<_, _> = BTreeMap::default();
        let mut delay_interval_s = None;
        let mut user_props = UserProperties::default();
        for kv in payload.metadata.properties.iter() {
            match kv.key.as_str() {
                "from_node" => from_node = Some(kv.value.parse::<NodeId>()?),
                "from_ipaddress" => from_ipaddress = Some(kv.value.parse::<SocketAddr>()?),
                "from_clientid" => from_clientid = Some(ClientId::from(kv.value.as_str())),
                "from_username" => from_username = Some(UserName::from(kv.value.as_str())),

                "retain" => {
                    retain = {
                        let retain_str = kv.value.to_lowercase();
                        match retain_str.as_str() {
                            "true" => Some(true),
                            "false" => Some(false),
                            _ => return Err(anyhow!(format!("Invalid Retain, {}", retain_str))),
                        }
                    }
                }
                "qos" => {
                    qos = {
                        let qos_str = kv.value.as_str();
                        Some(match qos_str {
                            "0" => QoS::AtMostOnce,
                            "1" => QoS::AtLeastOnce,
                            "2" => QoS::ExactlyOnce,
                            _ => return Err(anyhow!(format!("Invalid QoS, {}", qos_str))),
                        })
                    }
                }
                "delay_interval" => delay_interval_s = Some(kv.value.parse::<u32>()?),
                _ => {
                    if let Some(placeholder) = cfg_entry.local.props_topic_names.get(kv.key.as_str()) {
                        props_topics.insert(placeholder.as_str(), kv.value.as_str());
                    } else {
                        user_props
                            .push((ByteString::from(kv.key.as_str()), ByteString::from(kv.value.as_str())));
                    }
                }
            }
        }

        let create_time =
            payload.metadata.event_time.map(|t| t as TimestampMillis).unwrap_or_else(timestamp_millis);

        let f = From::from_bridge(Id::new(
            from_node.unwrap_or(node_id),
            0,
            None,
            from_ipaddress,
            from_clientid.unwrap_or_default(),
            from_username,
        ));

        log::debug!("props_topic_names: {:?}", cfg_entry.local.props_topic_names);
        log::debug!("props_topics: {props_topics:?}");

        let properties = PublishProperties::from(user_props);

        let (payload, topics) = match cfg_entry.remote.payload_format {
            PayloadFormat::Bytes => {
                let topics = cfg_entry.local.make_topics(&msg.topic, props_topics, None);

                (PayloadData::Bytes(payload.data.to_vec().into()), topics)
            }
            PayloadFormat::Json => {
                let payload_json: serde_json::Value =
                    serde_json::from_slice(payload.data.as_slice()).map_err(|e| anyhow!(e))?;
                log::debug!("payload_topic_names: {:?}", &cfg_entry.local.payload_topic_names);
                let payload_topics = cfg_entry
                    .local
                    .payload_topic_names
                    .iter()
                    .filter_map(|(path, placeholder)| {
                        payload_json.pointer(path).and_then(|topic| {
                            if let Some(t) = topic.as_str() {
                                Some((placeholder.as_str(), vec![t]))
                            } else if let Some(arr) = topic.as_array() {
                                let ts = arr.iter().filter_map(|t| t.as_str()).collect_vec();
                                Some((placeholder.as_str(), ts))
                            } else {
                                None
                            }
                        })
                    })
                    .collect::<BTreeMap<_, _>>();

                log::debug!("payload_topics: {payload_topics:?}");
                let topics = cfg_entry.local.make_topics(&msg.topic, props_topics, Some(payload_topics));

                let qos1 = payload_json
                    .as_object()
                    .and_then(|obj| {
                        obj.get("qos").and_then(|qos| match qos.as_u64() {
                            Some(0) => Some(Ok(QoS::AtMostOnce)),
                            Some(1) => Some(Ok(QoS::AtLeastOnce)),
                            Some(2) => Some(Ok(QoS::ExactlyOnce)),
                            Some(_) => Some(Err(anyhow!(format!("Invalid QoS, {:?}", qos)))),
                            None => None,
                        })
                    })
                    .transpose()?;

                qos = match (qos, qos1) {
                    (Some(q1), Some(q2)) => Some(q1.less_value(q2)),
                    (Some(q1), None) => Some(q1),
                    (None, Some(q2)) => Some(q2),
                    (None, None) => None,
                };

                if let Some(retain1) =
                    payload_json.as_object().and_then(|obj| obj.get("retain").and_then(|qos| qos.as_bool()))
                {
                    retain = Some(retain1);
                }

                (PayloadData::Json(payload_json), topics)
            }
        };

        let qos = cfg_entry.local.make_qos(qos);
        let retain = cfg_entry.local.make_retain(retain);

        Ok(Message { f, payload, topics, retain, qos, properties, delay_interval_s, create_time })
    }
}

#[derive(Debug)]
pub enum SystemCommand {
    Start,
    Restart,
    Close,
}

#[derive(Debug)]
pub enum Command {
    Close,
}

#[derive(Clone)]
pub struct CommandMailbox {
    pub(crate) consumer_name: ByteString,
    cmd_tx: mpsc::Sender<Command>,
}

impl CommandMailbox {
    pub(crate) fn new(cmd_tx: mpsc::Sender<Command>, consumer_name: ByteString) -> Self {
        CommandMailbox { cmd_tx, consumer_name }
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

pub struct Consumer {
    scx: ServerContext,
    pub(crate) consumer_name: ByteString,
    pub(crate) cfg: Bridge,
    pub(crate) cfg_entry: Entry,
}

impl Consumer {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn connect(
        scx: ServerContext,
        sys_cmd_tx: mpsc::Sender<SystemCommand>,
        pulsar: Pulsar<TokioExecutor>,
        cfg: Bridge,
        cfg_entry: Entry,
        entry_idx: usize,
        node_id: NodeId,
        on_message: OnMessageEvent,
    ) -> Result<CommandMailbox> {
        let cfg_entry1 = cfg_entry.clone();
        let consumer_name = if let Some(prefix) = &cfg.consumer_name_prefix {
            format!("{}:{}:ingress:{}:{}", prefix, cfg.name, node_id, entry_idx)
        } else {
            format!("{}:ingress:{}:{}", cfg.name, node_id, entry_idx)
        };
        let consumer_name = ByteString::from(consumer_name);
        log::debug!("consumer_name: {consumer_name}");

        let mut consumer = pulsar.consumer();
        if let Some(ref topic) = cfg_entry.remote.topic {
            consumer = consumer.with_topic(topic);
        }
        if let Some(topic_regex) = cfg_entry.remote.topic_regex {
            consumer = consumer.with_topic_regex(regex::Regex::new(&topic_regex).map_err(|e| anyhow!(e))?);
        }
        if let Some(sub_type) = cfg_entry.remote.subscription_type {
            consumer = consumer.with_subscription_type(sub_type);
        }
        if let Some(subscription) = cfg_entry.remote.subscription {
            consumer = consumer.with_subscription(subscription);
        }
        if let Some(lookup_namespace) = cfg_entry.remote.lookup_namespace {
            consumer = consumer.with_lookup_namespace(lookup_namespace);
        }
        if let Some(topic_refresh_interval) = cfg_entry.remote.topic_refresh_interval {
            consumer = consumer.with_topic_refresh(topic_refresh_interval);
        }
        if let Some(consumer_id) = cfg_entry.remote.consumer_id {
            consumer = consumer.with_consumer_id(consumer_id);
        }
        if let Some(batch_size) = cfg_entry.remote.batch_size {
            consumer = consumer.with_batch_size(batch_size);
        }

        if let Some(dead_letter_policy) = cfg_entry.remote.dead_letter_policy {
            consumer = consumer.with_dead_letter_policy(dead_letter_policy);
        }

        let mut consumer = consumer
            .with_topics(cfg_entry.remote.topics)
            .with_consumer_name(consumer_name.to_string())
            .with_unacked_message_resend_delay(cfg_entry.remote.unacked_message_resend_delay)
            .with_options(cfg_entry.remote.options)
            .build::<Vec<u8>>()
            .await
            .map_err(|e| anyhow!(e))?;

        consumer.check_connection().await.map_err(|e| anyhow!(e))?;
        log::info!("connection ok");

        let (cmd_tx, cmd_rx) = mpsc::channel(100_000);
        Self { scx, consumer_name: consumer_name.clone(), cfg, cfg_entry: cfg_entry1 }
            .start(sys_cmd_tx, consumer, cmd_rx, on_message)
            .await?;
        Ok(CommandMailbox::new(cmd_tx, consumer_name))
    }

    async fn start(
        self,
        sys_cmd_tx: mpsc::Sender<SystemCommand>,
        consumer: consumer::Consumer<Vec<u8>, TokioExecutor>,
        cmd_rx: mpsc::Receiver<Command>,
        on_message: OnMessageEvent,
    ) -> Result<()> {
        tokio::spawn(async move {
            self.ev_loop(sys_cmd_tx, consumer, cmd_rx, on_message).await;
        });
        Ok(())
    }

    async fn ev_loop(
        self,
        sys_cmd_tx: mpsc::Sender<SystemCommand>,
        mut consumer: consumer::Consumer<Vec<u8>, TokioExecutor>,
        mut cmd_rx: mpsc::Receiver<Command>,
        on_message: OnMessageEvent,
    ) {
        let cfg = self.cfg.clone();
        let entry = self.cfg_entry.clone();
        let name = self.cfg.name.as_str();
        let consumer_name = self.consumer_name;
        log::info!("{name}/{consumer_name} start pulsar recv loop");
        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    match cmd{
                        Some(Command::Close) => {
                            if let Err(e) = consumer.close().await {
                                log::warn!("{name}/{consumer_name} consumer close error, {e:?}");
                            }
                            log::info!("{name}/{consumer_name} Command(Close) pulsar exit event loop");
                            return;
                        }
                        None => {
                            log::error!("{name}/{consumer_name} Command(None) received");
                            break;
                        }
                    }
                },
                msg = consumer.next() => {
                    match msg {
                        None => {
                            log::warn!("{name}/{consumer_name} Message(None) received");
                            break
                        }
                        Some(Err(e)) => {
                            log::error!("{name}/{consumer_name} pulsar consumer recv error: {e:?}");
                            break
                        },
                        Some(Ok(data)) => {
                            let data: consumer::Message<Vec<u8>> = data;

                            if let Err(e) = consumer.ack(&data).await {
                                log::error!("{name}/{consumer_name} pulsar consumer recv message error: {e:?}");
                                break
                            }

                            let msg = match Message::deserialize_message(self.scx.node.id(), &data, &entry) {
                                Err(e) => {
                                    log::warn!("{name}/{consumer_name} pulsar consumer deserialize message error: {e:?}");
                                    continue
                                },
                                Ok(msg) => msg
                            };

                            log::debug!("{:?} {}/{} msg: {:?}", std::thread::current().id(), name, consumer_name, msg);

                            if let Err(e) = Self::process_message(&cfg, &entry, msg, &on_message) {
                                log::warn!("{name}/{consumer_name} pulsar consumer process message error: {e:?}");
                            }
                        }
                    }
                }
            }
        }
        if let Err(e) = consumer.close().await {
            log::warn!("{name}/{consumer_name} consumer close error, {e:?}");
        }
        log::info!("{name}/{consumer_name} pulsar exit event loop");
        if let Err(e) = sys_cmd_tx.send(SystemCommand::Restart).await {
            log::warn!("{name}/{consumer_name} consumer Send(SystemCommand::Restart) error, {e:?}");
        }
    }

    #[inline]
    fn process_message(cfg: &Bridge, entry: &Entry, m: Message, on_message: &OnMessageEvent) -> Result<()> {
        let payload = match entry.remote.payload_format {
            PayloadFormat::Bytes => m.payload.bytes()?,
            PayloadFormat::Json => {
                m.payload.extract(entry.remote.payload_path.as_deref())?.unwrap_or_default()
            }
        };
        if !entry.local.allow_empty_forward && payload.is_empty() {
            return Ok(());
        }
        for topic in m.topics {
            if topic.is_empty() {
                log::warn!("{} ignored forwarding some messages because the topic was empty.", cfg.name);
                continue;
            }
            let p = CodecPublish {
                dup: false,
                retain: m.retain,
                qos: m.qos,
                topic,
                packet_id: None,
                payload: payload.clone(),
                properties: Some(m.properties.clone()),
            };
            let p = <CodecPublish as Into<Publish>>::into(p).create_time(m.create_time);
            let p = if let Some(delay_interval_s) = m.delay_interval_s {
                p.delay_interval(delay_interval_s)
            } else {
                p
            };
            on_message.fire((m.f.clone(), p, cfg.expiry_interval));
        }
        Ok(())
    }
}

pub(crate) type BridgeName = ByteString;
type SourceKey = (BridgeName, EntryIndex);

type EntryIndex = usize;

#[derive(Clone)]
pub(crate) struct BridgeManager {
    scx: ServerContext,
    node_id: NodeId,
    cfg: Arc<RwLock<PluginConfig>>,
    sources: Arc<DashMap<SourceKey, CommandMailbox>>,
}

impl BridgeManager {
    pub async fn new(scx: ServerContext, node_id: NodeId, cfg: Arc<RwLock<PluginConfig>>) -> Self {
        Self { scx, node_id, cfg: cfg.clone(), sources: Arc::new(DashMap::default()) }
    }

    #[inline]
    async fn build_pulsar(cfg: &Bridge) -> Result<Pulsar<TokioExecutor>> {
        let mut builder = Pulsar::builder(cfg.servers.as_str(), TokioExecutor);

        match &cfg.auth.name {
            Some(AuthName::Token) => {
                let token = cfg.auth.data.clone().ok_or(anyhow!("token config is not exist"))?;
                let auth = Authentication { name: "token".to_string(), data: token.into_bytes() };
                builder = builder.with_auth(auth);
            }
            Some(AuthName::OAuth2) => {
                let oauth2_cfg = cfg.auth.data.clone().ok_or(anyhow!("oauth2 config is not exist"))?;
                let oauth2_json = serde_json::from_str(oauth2_cfg.as_str()).map_err(|e| {
                    anyhow!(format!("invalid oauth2 config [{}], {:?}", oauth2_cfg.as_str(), e))
                })?;
                builder = builder.with_auth_provider(OAuth2Authentication::client_credentials(oauth2_json));
            }
            None => {}
        }

        if let Some(cert_chain_file) = cfg.cert_chain_file.as_ref() {
            builder = builder.with_certificate_chain_file(cert_chain_file)?;
        }
        builder = builder.with_tls_hostname_verification_enabled(cfg.tls_hostname_verification_enabled);
        builder = builder.with_allow_insecure_connection(cfg.allow_insecure_connection);

        let pulsar: Pulsar<_> = builder.build().await.map_err(|e| anyhow!(e))?;
        Ok(pulsar)
    }

    pub async fn start(&mut self, sys_cmd_tx: mpsc::Sender<SystemCommand>) {
        while let Err(e) = self._start(sys_cmd_tx.clone()).await {
            log::error!("start bridge-ingress-pulsar error, {e:?}");
            self.stop().await;
            tokio::time::sleep(Duration::from_millis(3000)).await;
        }
    }

    async fn _start(&mut self, sys_cmd_tx: mpsc::Sender<SystemCommand>) -> Result<()> {
        let bridges = self.cfg.read().await.bridges.clone();
        let mut bridge_names: HashSet<&str> = HashSet::default();
        for b_cfg in &bridges {
            if !b_cfg.enable {
                continue;
            }
            if bridge_names.contains(&b_cfg.name as &str) {
                return Err(anyhow!(format!("The bridge name already exists! {:?}", b_cfg.name)));
            }

            let pulsar = Self::build_pulsar(b_cfg).await?;

            bridge_names.insert(&b_cfg.name);
            for (entry_idx, entry) in b_cfg.entries.iter().enumerate() {
                let mailbox = Consumer::connect(
                    self.scx.clone(),
                    sys_cmd_tx.clone(),
                    pulsar.clone(),
                    b_cfg.clone(),
                    entry.clone(),
                    entry_idx,
                    self.node_id,
                    self.on_message(),
                )
                .await?;
                self.sources.insert((b_cfg.name.clone(), entry_idx), mailbox);
            }
        }
        Ok(())
    }

    fn on_message(&self) -> OnMessageEvent {
        let scx = self.scx.clone();
        Arc::new(
            Event::listen(move |(f, p, expiry_interval): MessageType, _next| {
                let scx = scx.clone();
                tokio::spawn(async move {
                    send_publish(scx, f, p, expiry_interval).await;
                });
            })
            .finish(),
        )
    }

    pub async fn stop(&mut self) {
        for mut entry in &mut self.sources.iter_mut() {
            let ((bridge_name, entry_idx), mailbox) = entry.pair_mut();
            log::debug!("stop bridge_name: {bridge_name:?}, entry_idx: {entry_idx:?}",);
            if let Err(e) = mailbox.stop().await {
                log::warn!(
                    "stop BridgePulsarIngressPlugin error, bridge_name: {bridge_name}, entry_idx: {entry_idx}, {e:?}"
                );
            }
        }
        self.sources.clear();
    }

    #[allow(unused)]
    pub(crate) fn sources(&self) -> &DashMap<SourceKey, CommandMailbox> {
        &self.sources
    }
}

async fn send_publish(scx: ServerContext, from: From, msg: Publish, expiry_interval: Duration) {
    log::debug!("from {from:?}, message: {msg:?}");

    // let expiry_interval = msg
    //     .properties
    //     .message_expiry_interval
    //     .map(|interval| Duration::from_secs(interval.get() as u64))
    //     .unwrap_or(expiry_interval);

    let expiry_interval = msg
        .properties
        .as_ref()
        .and_then(|p| p.message_expiry_interval.map(|interval| Duration::from_secs(interval.get() as u64)))
        .unwrap_or(expiry_interval);

    //hook, message_publish
    let msg = scx.extends.hook_mgr().message_publish(None, from.clone(), &msg).await.unwrap_or(msg);

    let storage_available = scx.extends.message_mgr().await.enable();

    if let Err(e) = SessionState::forwards(&scx, from, msg, storage_available, Some(expiry_interval)).await {
        log::warn!("{e:?}");
    }
}

pub trait AsStr {
    fn as_str(&self) -> &str;
}

impl AsStr for bool {
    fn as_str(&self) -> &str {
        if *self {
            "true"
        } else {
            "false"
        }
    }
}

impl AsStr for ByteString {
    fn as_str(&self) -> &str {
        self.as_ref()
    }
}
