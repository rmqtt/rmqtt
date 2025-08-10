use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use event_notify::Event;

use rdkafka::client::ClientContext as KafkaClientContext;
use rdkafka::config::{ClientConfig as KafkaClientConfig, RDKafkaLogLevel};
use rdkafka::consumer::stream_consumer::StreamConsumer as KafkaStreamConsumer;
use rdkafka::consumer::{
    BaseConsumer, CommitMode, Consumer as KafkaConsumer, ConsumerContext as KafkaConsumerContext, Rebalance,
};
use rdkafka::error::KafkaResult;
use rdkafka::message::{BorrowedMessage, Headers, Message};
use rdkafka::topic_partition_list::TopicPartitionList;

use rmqtt::{
    anyhow::anyhow, bytes::Bytes, bytestring::ByteString, log, tokio, tokio::sync::mpsc, tokio::sync::RwLock,
    DashMap, UserProperties,
};
use rmqtt::{
    timestamp_millis, ClientId, From, Id, MqttError, NodeId, Publish, PublishProperties, QoS, Result,
    Runtime, SessionState, UserName,
};

use crate::config::{Bridge, Entry, PluginConfig, MESSAGE_KEY, PARTITION_UNASSIGNED};

type ExpiryInterval = Duration;

pub type MessageType = (From, Publish, ExpiryInterval);
pub type OnMessageEvent = Arc<Event<MessageType, ()>>;

#[derive(Debug)]
pub enum Command {
    Start,
    Close,
}

#[derive(Clone)]
pub struct CommandMailbox {
    pub(crate) client_id: ClientId,
    cmd_tx: mpsc::Sender<Command>,
}

impl CommandMailbox {
    pub(crate) fn new(cmd_tx: mpsc::Sender<Command>, client_id: ClientId) -> Self {
        CommandMailbox { cmd_tx, client_id }
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

struct SourceContext {}

impl KafkaClientContext for SourceContext {}
impl KafkaConsumerContext for SourceContext {
    fn pre_rebalance(&self, _base_consumer: &BaseConsumer<Self>, rebalance: &Rebalance<'_>) {
        log::debug!("Pre rebalance {rebalance:?}");
    }
    fn post_rebalance(&self, _base_consumer: &BaseConsumer<Self>, rebalance: &Rebalance<'_>) {
        log::debug!("Post rebalance {rebalance:?}");
    }
    fn commit_callback(&self, result: KafkaResult<()>, _offsets: &TopicPartitionList) {
        match result {
            Ok(_) => log::debug!("Offsets committed successfully"),
            Err(e) => log::warn!("Error while committing offsets: {e}"),
        }
    }
}

pub struct Consumer {
    pub(crate) client_id: ByteString,
    pub(crate) cfg: Arc<Bridge>,
    pub(crate) cfg_entry: Entry,
    enable_auto_commit: bool,
}

impl Consumer {
    pub(crate) async fn connect(
        cfg: Arc<Bridge>,
        cfg_entry: Entry,
        entry_idx: usize,
        node_id: NodeId,
        on_message: OnMessageEvent,
    ) -> Result<CommandMailbox> {
        let mut topics = Vec::new();
        let mut topic_parts = TopicPartitionList::new();
        let mut client_cfg = KafkaClientConfig::new();

        client_cfg.set("bootstrap.servers", cfg.servers.as_str());

        let client_id = if let Some(client_id_prefix) = &cfg.client_id_prefix {
            let client_id = format!("{}:{}:ingress:{}:{}", client_id_prefix, cfg.name, node_id, entry_idx);
            log::debug!("client: {}", client_id.as_str());
            client_cfg.set("client.id", client_id.as_str());
            ByteString::from(client_id)
        } else {
            ByteString::default()
        };

        let mut enable_auto_commit = true;
        for (key, val) in &cfg.properties {
            if !key.trim_start().starts_with('#') {
                log::info!("{key}={val}");
                if key == "enable.auto.commit" && val == "false" {
                    enable_auto_commit = false;
                }
                client_cfg.set(key, val);
            }
        }

        let t = cfg_entry.remote.topic.as_str();
        let start_partition = cfg_entry.remote.start_partition;
        let stop_partition = cfg_entry.remote.stop_partition;
        let offset = cfg_entry.remote.offset;
        if cfg_entry.remote.start_partition == cfg_entry.remote.stop_partition
            || (cfg_entry.remote.stop_partition == PARTITION_UNASSIGNED)
        {
            topics.push(t);
            topic_parts.add_partition(t, start_partition);
            if let Some(offset) = offset {
                topic_parts.set_partition_offset(t, start_partition, offset).map_err(|e| anyhow!(e))?;
            }
        } else if start_partition < stop_partition {
            topics.push(t);
            topic_parts.add_partition_range(t, start_partition, stop_partition);
            if let Some(offset) = offset {
                for p in start_partition..stop_partition + 1 {
                    log::debug!("{start_partition}..{stop_partition}, partition: {p}");
                    topic_parts.set_partition_offset(t, p, offset).map_err(|e| anyhow!(e))?;
                }
            }
        } else {
            log::error!(
                "Kafka topic config error, start_partition({start_partition}) > stop_partition({stop_partition}), {t:?}"
            );
        }

        client_cfg.set("group.id", cfg_entry.remote.group_id.as_str());

        log::info!("topic_parts: {topic_parts:?}");
        client_cfg.set_log_level(RDKafkaLogLevel::Info);

        log::info!("client_cfg: {client_cfg:?}");

        let consumer: KafkaStreamConsumer<SourceContext> =
            client_cfg.create_with_context(SourceContext {}).map_err(|e| anyhow!(e))?;

        consumer.subscribe(&topics).map_err(|e| anyhow!(e))?;
        consumer.assign(&topic_parts).map_err(|e| anyhow!(e))?;

        let (cmd_tx, cmd_rx) = mpsc::channel(100_000);
        Self { client_id: client_id.clone(), cfg, cfg_entry, enable_auto_commit }
            .start(consumer, cmd_rx, on_message)
            .await?;
        Ok(CommandMailbox::new(cmd_tx, client_id))
    }

    async fn start(
        self,
        consumer: KafkaStreamConsumer<SourceContext>,
        cmd_rx: mpsc::Receiver<Command>,
        on_message: OnMessageEvent,
    ) -> Result<()> {
        let enable_auto_commit = self.enable_auto_commit;
        log::info!(
            "{} enable_auto_commit: {}, client_id: {}",
            self.cfg.name,
            enable_auto_commit,
            self.client_id,
        );
        tokio::spawn(async move {
            self.ev_loop(consumer, cmd_rx, on_message).await;
        });
        Ok(())
    }

    async fn ev_loop(
        self,
        consumer: KafkaStreamConsumer<SourceContext>,
        mut cmd_rx: mpsc::Receiver<Command>,
        on_message: OnMessageEvent,
    ) {
        let cfg = self.cfg.clone();
        let entry = self.cfg_entry.clone();
        let name = self.cfg.name.as_str();
        let client_id = self.client_id;
        let enable_auto_commit = self.enable_auto_commit;
        log::info!("{name}/{client_id} start kafka recv loop");
        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    match cmd{
                        Some(Command::Close) => {
                            break
                        }
                        Some(Command::Start) => {}
                        None => {
                            log::error!("{name}/{client_id} Command(None) received");
                            break;
                        }
                    }
                },
                msg = consumer.recv() => {
                    match msg{
                        Err(e) => {
                            log::error!("{name}/{client_id} Kafka error: {e:?}");
                            tokio::time::sleep(Duration::from_millis(1000)).await;
                        },
                        Ok(m) => {
                            let m: BorrowedMessage = m;

                            log::debug!("{:?} {}/{} key: {:?}, payload: {:?}({}), topic: {}, partition: {}, offset: {}, timestamp: {:?}",
                                  std::thread::current().id(), name, client_id, m.key().map(|v| String::from_utf8_lossy(v)), m.payload().map(|v|String::from_utf8_lossy(v)), m.payload().map(|v| v.len()).unwrap_or_default(), m.topic(), m.partition(), m.offset(), m.timestamp());

                            Self::process_message(cfg.as_ref(), &entry, name, client_id.clone(), &m, &on_message);

                            if !enable_auto_commit {
                                if let Err(e) = consumer.commit_message(&m, CommitMode::Async){
                                    log::error!("{name}/{client_id} Kafka commit_message error: {e:?}");
                                }
                            }
                        }
                    }
                }
            }
        }
        log::info!("{name}/{client_id} Kafka exit event loop");
    }

    fn process_message(
        cfg: &Bridge,
        entry: &Entry,
        name: &str,
        client_id: ClientId,
        m: &BorrowedMessage,
        on_message: &OnMessageEvent,
    ) {
        let mut user_properties = UserProperties::default();
        let mut remote_addr = None;
        let mut from_clientid = None;
        let mut from_username = None;
        let mut qos = None;
        let mut retain = None;
        // let mut topic = None;
        if let Some(headers) = m.headers() {
            for i in 0..headers.count() {
                let h = headers.get(i);
                let key = h.key;
                let val = h.value.map(|v| String::from_utf8_lossy(v));
                log::debug!("{name}/{client_id} Header {key:#?}: {val:?}");

                match (key, val) {
                    ("from_ipaddress", Some(addr)) => match addr.parse::<SocketAddr>() {
                        Ok(addr) => {
                            remote_addr = Some(addr);
                        }
                        Err(e) => {
                            log::warn!(
                                "{}/{} Illegal IP address, from_ipaddress({}) {:?}",
                                name,
                                client_id,
                                addr.as_ref(),
                                e
                            );
                        }
                    },
                    ("from_clientid", Some(cid)) => {
                        from_clientid = Some(ClientId::from(cid.as_ref()));
                    }
                    ("from_username", Some(username)) => {
                        from_username = Some(UserName::from(username.as_ref()));
                    }
                    ("qos", Some(q)) => match q.as_ref() {
                        "0" => {
                            qos = Some(QoS::AtMostOnce);
                        }
                        "1" => {
                            qos = Some(QoS::AtLeastOnce);
                        }
                        "2" => {
                            qos = Some(QoS::ExactlyOnce);
                        }
                        _ => {
                            log::warn!("{}/{} Illegal QoS, qos({})", name, client_id, q.as_ref());
                        }
                    },
                    ("retain", Some(r)) => match r.as_ref() {
                        "true" => {
                            retain = Some(true);
                        }
                        "false" => {
                            retain = Some(false);
                        }
                        _ => {
                            log::warn!("{}/{} Illegal Retain, retain({})", name, client_id, r.as_ref());
                        }
                    },
                    (key, Some(val)) => {
                        user_properties.push((ByteString::from(key), ByteString::from(val.as_ref())));
                    }
                    _ => {}
                }
            }
        }

        let key = m.key().map(|v| String::from_utf8_lossy(v));
        if let Some(ref key) = key {
            user_properties.push((MESSAGE_KEY.into(), ByteString::from(key.as_ref())))
        }

        let payload = m.payload().map(|payload| Bytes::from(payload.to_vec())).unwrap_or_default();

        let from = From::from_bridge(Id::new(
            Runtime::instance().node.id(),
            None,
            remote_addr,
            from_clientid.unwrap_or(client_id),
            from_username,
        ));

        let properties = PublishProperties::from(user_properties);
        let p = Publish {
            dup: false,
            retain: entry.local.make_retain(retain),
            qos: entry.local.make_qos(qos),
            topic: entry.local.make_topic(key),
            packet_id: None,
            payload,
            properties,
            delay_interval: None,
            create_time: timestamp_millis(),
        };

        on_message.fire((from, p, cfg.expiry_interval));
    }
}

pub(crate) type BridgeName = ByteString;
type SourceKey = (BridgeName, EntryIndex);

type EntryIndex = usize;

#[derive(Clone)]
pub(crate) struct BridgeManager {
    node_id: NodeId,
    cfg: Arc<RwLock<PluginConfig>>,
    sources: Arc<DashMap<SourceKey, CommandMailbox>>,
}

impl BridgeManager {
    pub async fn new(node_id: NodeId, cfg: Arc<RwLock<PluginConfig>>) -> Self {
        Self { node_id, cfg: cfg.clone(), sources: Arc::new(DashMap::default()) }
    }

    pub async fn start(&mut self) -> Result<()> {
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
                let mailbox = Consumer::connect(
                    Arc::new(b_cfg.clone()),
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
        Arc::new(
            Event::listen(|(f, p, expiry_interval): MessageType, _next| {
                tokio::spawn(async move {
                    send_publish(f, p, expiry_interval).await;
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
                log::error!(
                    "stop BridgeKafkaIngressPlugin error, bridge_name: {bridge_name}, entry_idx: {entry_idx}, {e:?}"
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

async fn send_publish(from: From, msg: Publish, expiry_interval: Duration) {
    log::debug!("from {from:?}, message: {msg:?}");

    let expiry_interval = msg
        .properties
        .message_expiry_interval
        .map(|interval| Duration::from_secs(interval.get() as u64))
        .unwrap_or(expiry_interval);

    //hook, message_publish
    let msg = Runtime::instance()
        .extends
        .hook_mgr()
        .await
        .message_publish(None, from.clone(), &msg)
        .await
        .unwrap_or(msg);

    let storage_available = Runtime::instance().extends.message_mgr().await.enable();

    if let Err(e) = SessionState::forwards(from, msg, storage_available, Some(expiry_interval)).await {
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
