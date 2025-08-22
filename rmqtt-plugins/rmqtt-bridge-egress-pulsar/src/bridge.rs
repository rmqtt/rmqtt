use std::collections::HashSet;
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use pulsar::{
    authentication::oauth2::OAuth2Authentication, producer, Authentication, Error as PulsarError,
    Producer as PulsarProducer, Pulsar, SerializeMessage, TokioExecutor,
};

use rmqtt::{
    anyhow::anyhow, bytestring::ByteString, log, serde_json, tokio, tokio::sync::mpsc, tokio::sync::RwLock,
    DashMap,
};

use rmqtt::{
    broker::topic::{TopicTree, VecToTopic},
    From, MqttError, NodeId, Publish, QoSEx, Result, Topic,
};

use crate::config::{AuthName, Bridge, Entry, PluginConfig};

struct Message<'a> {
    f: &'a From,
    p: &'a Publish,
    cfg: &'a Entry,
    metadata: &'a BTreeMap<String, String>,
}

impl SerializeMessage for Message<'_> {
    fn serialize_message(input: Self) -> Result<producer::Message, PulsarError> {
        let f = input.f;
        let p = input.p;
        let cfg = &input.cfg.remote;

        //Not required to forward
        let mut properties: HashMap<String, String> = HashMap::new();
        if !input.metadata.is_empty() {
            properties.extend(input.metadata.clone());
        }

        if cfg.forward_all_from {
            properties.insert("from_type".into(), f.typ().as_str().into());
            properties.insert("from_node".into(), f.node().to_string());
            if let Some(addr) = f.remote_addr {
                properties.insert("from_ipaddress".into(), addr.to_string());
            }
            properties.insert("from_clientid".into(), f.client_id.to_string());
            properties.insert("from_username".into(), f.username().into());
        }

        //Not required to forward
        if cfg.forward_all_publish {
            properties.insert("dup".into(), if p.dup() { "true".into() } else { "false".into() });
            properties.insert("retain".into(), if p.retain() { "true".into() } else { "false".into() });
            properties.insert("qos".into(), p.qos().value().to_string());
            if let Some(packet_id) = p.packet_id() {
                properties.insert("packet_id".into(), packet_id.to_string());
            }
        }

        //Must forward
        properties.insert("topic".into(), p.topic().to_string());
        let payload = p.payload.to_vec();
        let event_time = Some(p.create_time() as u64);
        let partition_key = cfg.partition_key.clone();
        let ordering_key = cfg.ordering_key.as_ref().map(|okey| okey.generate(&f.client_id));
        let replicate_to = cfg.replicate_to.clone();
        let schema_version = cfg.schema_version.clone();
        log::debug!("ordering_key: {ordering_key:?}");
        Ok(producer::Message {
            payload,
            properties,
            partition_key,
            ordering_key,
            replicate_to,
            event_time,
            schema_version,
        })
    }
}

#[derive(Debug)]
pub enum Command {
    Start,
    Close,
    Message(From, Publish),
}

pub struct Producer {
    pub(crate) name: String,
    tx: mpsc::Sender<Command>,
}

impl Producer {
    pub(crate) async fn from(
        pulsar: Pulsar<TokioExecutor>,
        cfg: Arc<Bridge>,
        cfg_entry: Entry,
        entry_idx: usize,
        node_id: NodeId,
    ) -> Result<Self> {
        log::debug!("entry_idx: {entry_idx}, node_id: {node_id}");

        let producer_name = if let Some(prefix) = &cfg.producer_name_prefix {
            format!("{}:{}:egress:{}:{}", prefix, cfg.name, node_id, entry_idx)
        } else {
            format!("{}:egress:{}:{}", cfg.name, node_id, entry_idx)
        };
        log::debug!("producer_name: {producer_name}");

        let topic = cfg_entry.remote.topic.as_str();
        log::debug!("topic: {topic}");

        //user defined properties added to all messages
        let metadata = cfg_entry.remote.options.metadata.clone();
        log::debug!("metadata: {metadata:?}");
        let compression = cfg_entry.remote.options.compression.as_ref().map(|c| c.to_pulsar_comp());
        log::debug!("compression: {compression:?}");
        let access_mode = cfg_entry.remote.options.access_mode;
        log::debug!("access_mode: {access_mode:?}");

        let producer = pulsar
            .producer()
            .with_topic(topic)
            .with_name(&producer_name)
            .with_options(producer::ProducerOptions {
                metadata,
                compression,
                access_mode,
                ..Default::default()
            })
            .build()
            .await
            .map_err(|e| anyhow!(e))?;

        producer.check_connection().await.map_err(|e| anyhow!(e))?;
        log::info!("connection ok");

        let (tx, rx) = mpsc::channel(100_000);
        tokio::spawn(async move {
            Self::start(cfg_entry, producer, rx).await;
        });
        Ok(Producer { name: producer_name, tx })
    }

    async fn start(
        entry_cfg: Entry,
        mut producer: PulsarProducer<TokioExecutor>,
        mut rx: mpsc::Receiver<Command>,
    ) {
        let metadata = producer.options().metadata.clone();
        while let Some(cmd) = rx.recv().await {
            match cmd {
                Command::Close => {
                    if let Err(e) = producer.close().await {
                        log::warn!("{e}");
                    }
                    break;
                }
                Command::Start => {}
                Command::Message(f, p) => {
                    match producer
                        .send_non_blocking(Message { f: &f, p: &p, cfg: &entry_cfg, metadata: &metadata })
                        .await
                    {
                        Err(e) => {
                            log::warn!("{e}");
                        }
                        Ok(fut) => match fut.await {
                            Ok(receipt) => {
                                log::debug!(
                            "highest_sequence_id: {:?}, sequence_id: {}, producer_id: {}, message_id.ack_set: {:?}",
                            receipt.highest_sequence_id,
                            receipt.sequence_id,
                            receipt.producer_id,
                            receipt.message_id.map(|m| m.ack_set)
                        );
                            }
                            Err(e) => {
                                log::warn!("{e}");
                            }
                        },
                    }
                }
            }
        }
        log::info!("exit pulsar producer.")
    }

    #[inline]
    pub(crate) async fn send(&self, f: &From, p: &Publish) -> Result<()> {
        self.tx.send(Command::Message(f.clone(), p.clone())).await?;
        Ok(())
    }
}

pub(crate) type BridgeName = ByteString;
type SourceKey = (BridgeName, EntryIndex);

type EntryIndex = usize;

#[derive(Clone)]
pub(crate) struct BridgeManager {
    node_id: NodeId,
    cfg: Arc<RwLock<PluginConfig>>,
    sinks: Arc<DashMap<SourceKey, Producer>>,
    topics: Arc<RwLock<TopicTree<(BridgeName, EntryIndex)>>>,
}

impl BridgeManager {
    pub async fn new(node_id: NodeId, cfg: Arc<RwLock<PluginConfig>>) -> Self {
        Self {
            node_id,
            cfg: cfg.clone(),
            sinks: Arc::new(DashMap::default()),
            topics: Arc::new(RwLock::new(TopicTree::default())),
        }
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

    pub async fn start(&mut self) {
        while let Err(e) = self._start().await {
            log::error!("start bridge-egress-pulsar error, {e:?}");
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

            let pulsar = Self::build_pulsar(b_cfg).await?;

            bridge_names.insert(&b_cfg.name);
            for (entry_idx, entry) in b_cfg.entries.iter().enumerate() {
                log::debug!("entry.local.topic_filter: {}", entry.local.topic_filter);
                topics.insert(
                    &Topic::from_str(entry.local.topic_filter.as_str())?,
                    (b_cfg.name.clone(), entry_idx),
                );

                let producer = Producer::from(
                    pulsar.clone(),
                    Arc::new(b_cfg.clone()),
                    entry.clone(),
                    entry_idx,
                    self.node_id,
                )
                .await?;
                self.sinks.insert((b_cfg.name.clone(), entry_idx), producer);
            }
        }
        Ok(())
    }

    pub async fn stop(&mut self) {
        for mut entry in &mut self.sinks.iter_mut() {
            let ((bridge_name, entry_idx), producer) = entry.pair_mut();
            log::debug!("stop bridge_name: {bridge_name:?}, entry_idx: {entry_idx:?}",);
            if let Err(e) = producer.tx.send(Command::Close).await {
                log::error!("{e:?}");
            }
        }
        self.sinks.clear();
    }

    #[allow(unused)]
    pub(crate) fn sinks(&self) -> &DashMap<SourceKey, Producer> {
        &self.sinks
    }

    #[inline]
    pub(crate) async fn send(&self, f: &From, p: &Publish) -> Result<()> {
        let topic = Topic::from_str(&p.topic)?;
        for (topic_filter, bridge_infos) in self.topics.read().await.matches(&topic).iter() {
            let topic_filter = topic_filter.to_topic_filter();
            log::debug!("topic_filter: {topic_filter:?}");
            log::debug!("bridge_infos: {bridge_infos:?}");
            for (name, entry_idx) in bridge_infos {
                if let Some(producer) = self.sinks.get(&(name.clone(), *entry_idx)) {
                    if let Err(e) = producer.send(f, p).await {
                        log::warn!("{e}");
                    }
                }
            }
        }
        Ok(())
    }
}
