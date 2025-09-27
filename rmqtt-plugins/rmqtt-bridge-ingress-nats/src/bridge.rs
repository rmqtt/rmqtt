use std::borrow::Cow;
use std::collections::HashSet;
use std::convert::From as _;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use async_nats::{
    connect_with_options, Client, ConnectOptions, Message as NatsMessage, ServerAddr, Subscriber,
};
use bytes::Bytes;
use bytestring::ByteString;
use event_notify::Event;
use futures::SinkExt;
use futures::StreamExt;
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

use crate::config::{Bridge, Entry, PluginConfig};

type ExpiryInterval = Duration;

pub type MessageType = (From, Publish, ExpiryInterval);
pub type OnMessageEvent = Arc<Event<MessageType, ()>>;

#[derive(Debug)]
struct Message {
    f: From,
    payload: Bytes,
    topic: TopicName,
    retain: bool,
    qos: QoS,
    properties: PublishProperties,
    delay_interval_s: Option<u32>,
    create_time: TimestampMillis,
}

impl Message {
    fn deserialize_message(node_id: NodeId, msg: NatsMessage, cfg_entry: &Entry) -> Result<Message> {
        let mut from_node = None;
        let mut from_ipaddress = None;
        let mut from_clientid = None;
        let mut from_username = None;
        let mut retain = None;
        let mut qos = None;

        let mut delay_interval_s = None;
        let mut user_props = UserProperties::default();
        if let Some(headers) = msg.headers {
            for (key, vals) in headers.iter() {
                match key.as_ref() {
                    "from_node" => {
                        from_node = vals.first().map(|v| v.as_str().parse::<NodeId>()).transpose()?;
                    }
                    "from_ipaddress" => {
                        from_ipaddress =
                            vals.first().map(|v| v.as_str().parse::<SocketAddr>()).transpose()?;
                    }
                    "from_clientid" => {
                        from_clientid = vals.first().map(|v| ClientId::from(v.as_str()));
                    }
                    "from_username" => {
                        from_username = vals.first().map(|v| UserName::from(v.as_str()));
                    }

                    "retain" => {
                        retain = {
                            let retain_str =
                                vals.first().map(|v| v.as_str().to_lowercase()).unwrap_or_default();
                            match retain_str.as_str() {
                                "true" => Some(true),
                                "false" => Some(false),
                                _ => return Err(anyhow!(format!("Invalid Retain, {retain_str:?}"))),
                            }
                        }
                    }

                    "qos" => {
                        qos = {
                            let qos_str = vals.first().map(|v| v.as_str());
                            Some(match qos_str {
                                Some("0") => QoS::AtMostOnce,
                                Some("1") => QoS::AtLeastOnce,
                                Some("2") => QoS::ExactlyOnce,
                                _ => return Err(anyhow!(format!("Invalid QoS, {qos_str:?}"))),
                            })
                        }
                    }
                    "delay_interval" => {
                        delay_interval_s = vals.first().map(|v| v.as_str().parse::<u32>()).transpose()?;
                    }
                    _ => {
                        let val = vals.first().map(|v| v.as_str());
                        if let Some(val) = val {
                            user_props.push((ByteString::from(key.as_ref()), ByteString::from(val)));
                        }
                    }
                }
            }
        }

        let f = From::from_bridge(Id::new(
            from_node.unwrap_or(node_id),
            0,
            None,
            from_ipaddress,
            from_clientid.unwrap_or_default(),
            from_username,
        ));

        let properties = PublishProperties::from(user_props);
        let payload = msg.payload;
        let create_time = timestamp_millis();
        let qos = cfg_entry.local.make_qos(qos);
        let retain = cfg_entry.local.make_retain(retain);
        let topic = cfg_entry.local.make_topic(Some(Cow::Borrowed(msg.subject.as_str())));

        Ok(Message { f, payload, topic, retain, qos, properties, delay_interval_s, create_time })
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

        let consumer = Self::build_nats(&cfg, &consumer_name).await?;

        let subscriber = if let Some(group) = cfg_entry.remote.group.as_ref() {
            consumer.queue_subscribe(cfg_entry.remote.topic, group.into()).await?
        } else {
            consumer.subscribe(cfg_entry.remote.topic).await?
        };

        let (cmd_tx, cmd_rx) = mpsc::channel(100_000);
        Self { scx, consumer_name: consumer_name.clone(), cfg, cfg_entry: cfg_entry1 }
            .start(sys_cmd_tx, consumer, subscriber, cmd_rx, on_message)
            .await?;
        Ok(CommandMailbox::new(cmd_tx, consumer_name))
    }

    #[inline]
    async fn build_nats(cfg: &Bridge, consumer_name: &str) -> Result<Client> {
        let addrs = cfg
            .servers
            .as_str()
            .split(',')
            .map(|url| url.parse())
            .collect::<std::result::Result<Vec<ServerAddr>, _>>()?;
        let mut opts = ConnectOptions::new();
        opts = opts.name(consumer_name);

        if let Some(true) = cfg.no_echo {
            opts = opts.no_echo();
        }
        if let Some(max_reconnects) = cfg.max_reconnects {
            opts = opts.max_reconnects(max_reconnects);
        }
        if let Some(ping_interval) = cfg.ping_interval {
            opts = opts.ping_interval(ping_interval);
        }
        if let Some(connection_timeout) = cfg.connection_timeout {
            opts = opts.connection_timeout(connection_timeout);
        }
        if let Some(tls_required) = cfg.tls_required {
            opts = opts.require_tls(tls_required);
        }
        if let Some(true) = cfg.tls_first {
            opts = opts.tls_first();
        }
        if let Some(root_certificates) = cfg.root_certificates.as_ref() {
            opts = opts.add_root_certificates(root_certificates.clone());
        }
        if let (Some(client_cert), Some(client_key)) = (cfg.client_cert.as_ref(), cfg.client_key.as_ref()) {
            opts = opts.add_client_certificate(client_cert.clone(), client_key.clone());
        }
        if let Some(sender_capacity) = cfg.sender_capacity {
            opts = opts.client_capacity(sender_capacity);
        }
        opts = opts.request_timeout(cfg.request_timeout);
        if cfg.retry_on_initial_connect {
            opts = opts.retry_on_initial_connect();
        }
        if cfg.ignore_discovered_servers {
            opts = opts.ignore_discovered_servers();
        }
        if cfg.retain_servers_order {
            opts = opts.retain_servers_order();
        }
        if let Some(read_buffer_capacity) = cfg.read_buffer_capacity {
            opts = opts.read_buffer_capacity(read_buffer_capacity);
        }

        if let (Some(jwt), Some(seed)) = (cfg.auth.jwt.as_ref(), cfg.auth.jwt_seed.as_ref()) {
            let key_pair = Arc::new(nkeys::KeyPair::from_seed(seed).map_err(|e| anyhow!(e))?);
            opts = opts.jwt(jwt.into(), move |nonce| {
                let key_pair = key_pair.clone();
                async move { key_pair.sign(&nonce).map_err(async_nats::AuthError::new) }
            })
        }
        if let Some(nkey) = cfg.auth.nkey.as_ref() {
            opts = opts.nkey(nkey.into());
        }
        if let (Some(username), Some(password)) = (cfg.auth.username.as_ref(), cfg.auth.password.as_ref()) {
            opts = opts.user_and_password(username.into(), password.into());
        }
        if let Some(token) = cfg.auth.token.as_ref() {
            opts = opts.token(token.into());
        }

        log::debug!("ConnectOptions: {opts:?}");
        let client = connect_with_options(addrs, opts)
            .await
            .map_err(|e| anyhow!(format!("Failed to connect to NATS, {}", e)))?;
        Ok(client)
    }

    async fn start(
        self,
        sys_cmd_tx: mpsc::Sender<SystemCommand>,
        consumer: Client,
        subscriber: Subscriber,
        cmd_rx: mpsc::Receiver<Command>,
        on_message: OnMessageEvent,
    ) -> Result<()> {
        tokio::spawn(async move {
            self.ev_loop(sys_cmd_tx, consumer, subscriber, cmd_rx, on_message).await;
        });
        Ok(())
    }

    async fn ev_loop(
        self,
        sys_cmd_tx: mpsc::Sender<SystemCommand>,
        mut consumer: Client,
        mut subscriber: Subscriber,
        mut cmd_rx: mpsc::Receiver<Command>,
        on_message: OnMessageEvent,
    ) {
        let cfg = self.cfg.clone();
        let entry = self.cfg_entry.clone();
        let name = self.cfg.name.as_str();
        let consumer_name = self.consumer_name;

        log::info!("{name}/{consumer_name} start nats recv loop");
        loop {
            tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd{
                    Some(Command::Close) => {
                        if let Err(e) = consumer.close().await {
                            log::warn!("{name}/{consumer_name} consumer close error, {e:?}");
                        }
                        log::info!("{name}/{consumer_name} Command(Close) nats exit event loop");
                        return;
                    }
                    None => {
                        log::error!("{name}/{consumer_name} Command(None) received");
                        break;
                    }
                }
            },
            msg = subscriber.next() => {
                match msg {
                    None => {
                        log::warn!("{name}/{consumer_name} Message(None) received");
                        break
                    }
                    Some(data) => {
                        let msg = match Message::deserialize_message(self.scx.node.id(), data, &entry) {
                            Err(e) => {
                                log::warn!("{name}/{consumer_name} nats consumer deserialize message error: {e:?}");
                                continue
                            },
                            Ok(msg) => msg
                        };

                        log::debug!("{:?} {}/{} msg: {:?}", std::thread::current().id(), name, consumer_name, msg);

                        if let Err(e) = Self::process_message(&cfg, &entry, msg, &on_message) {
                            log::warn!("{name}/{consumer_name} nats consumer process message error: {e:?}");
                        }

                    }
                }
                }
            }
        }
        if let Err(e) = consumer.close().await {
            log::warn!("{name}/{consumer_name} consumer close error, {e:?}");
        }
        log::info!("{name}/{consumer_name} nats exit event loop");
        if let Err(e) = sys_cmd_tx.send(SystemCommand::Restart).await {
            log::warn!("{name}/{consumer_name} consumer Send(SystemCommand::Restart) error, {e:?}");
        }
    }

    #[inline]
    fn process_message(cfg: &Bridge, entry: &Entry, m: Message, on_message: &OnMessageEvent) -> Result<()> {
        let payload = m.payload;
        if !entry.local.allow_empty_forward && payload.is_empty() {
            return Ok(());
        }

        if m.topic.is_empty() {
            log::warn!("{} ignored forwarding some messages because the topic was empty.", cfg.name);
        } else {
            let p = CodecPublish {
                dup: false,
                retain: m.retain,
                qos: m.qos,
                topic: m.topic,
                packet_id: None,
                payload,
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

    pub async fn start(&mut self, sys_cmd_tx: mpsc::Sender<SystemCommand>) {
        while let Err(e) = self._start(sys_cmd_tx.clone()).await {
            log::error!("start bridge-ingress-nats error, {e:?}");
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

            bridge_names.insert(&b_cfg.name);
            for (entry_idx, entry) in b_cfg.entries.iter().enumerate() {
                let mailbox = Consumer::connect(
                    self.scx.clone(),
                    sys_cmd_tx.clone(),
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
                    "stop BridgeNatsIngressPlugin error, bridge_name: {bridge_name}, entry_idx: {entry_idx}, {e:?}"
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
