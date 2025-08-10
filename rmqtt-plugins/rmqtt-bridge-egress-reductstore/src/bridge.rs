use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::SystemTime;

use reduct_rs::{Bucket, QuotaType, ReductClient};

use rmqtt::{
    anyhow::anyhow, bytestring::ByteString, log, reqwest::Url, tokio, tokio::sync::mpsc, tokio::sync::RwLock,
    DashMap,
};

use rmqtt::{
    broker::topic::{TopicTree, VecToTopic},
    From, MqttError, NodeId, Publish, QoSEx, Result, Topic,
};

use crate::config::{Bridge, Entry, PluginConfig};

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

        let producer = Self::build_reductstore(&cfg).await?;

        let mut builder =
            producer.create_bucket(cfg_entry.remote.bucket.as_str()).quota_type(QuotaType::FIFO);
        if let Some(quota_size) = cfg_entry.remote.quota_size {
            builder = builder.quota_size(quota_size);
        }
        if let Some(exist_ok) = cfg_entry.remote.exist_ok {
            builder = builder.exist_ok(exist_ok);
        }

        let bucket = builder.send().await.map_err(|e| format!("Failed to create Bucket, {e}"))?;

        let (tx, rx) = mpsc::channel(100_000);
        tokio::spawn(async move {
            Self::start(cfg_entry, bucket, rx).await;
        });
        Ok(Producer { name: producer_name, tx })
    }

    #[inline]
    async fn build_reductstore(cfg: &Bridge) -> Result<ReductClient> {
        let _ = Url::parse(cfg.servers.as_str()).map_err(|e| anyhow!(e))?;
        let mut builder = ReductClient::builder().url(cfg.servers.as_str());
        if let Some(api_token) = cfg.api_token.as_ref() {
            builder = builder.api_token(api_token);
        }
        if let Some(verify_ssl) = cfg.verify_ssl {
            builder = builder.verify_ssl(verify_ssl);
        }
        if let Some(timeout) = cfg.timeout {
            builder = builder.timeout(timeout);
        }

        Ok(builder.try_build().map_err(|e| format!("Failed to connect to Reductstore, {e}"))?)
    }

    async fn start(entry_cfg: Entry, bucket: Bucket, mut rx: mpsc::Receiver<Command>) {
        let forward_all_from = entry_cfg.remote.forward_all_from;
        let forward_all_publish = entry_cfg.remote.forward_all_publish;

        while let Some(cmd) = rx.recv().await {
            match cmd {
                Command::Close => {
                    break;
                }
                Command::Start => {}
                Command::Message(f, p) => {
                    let start = SystemTime::now();
                    let mut sender = bucket.write_record(entry_cfg.remote.entry.as_str()).timestamp(start);

                    //Not required to forward
                    if forward_all_from {
                        sender = sender.add_label("from_type", f.typ().as_str());
                        sender = sender.add_label("from_node", f.node().to_string());
                        if let Some(addr) = f.remote_addr {
                            sender = sender.add_label("from_ipaddress", addr.to_string());
                        }
                        sender = sender.add_label("from_clientid", f.client_id.clone());
                        sender = sender.add_label("from_username", f.username());
                    }

                    //Not required to forward
                    if forward_all_publish {
                        sender = sender.add_label("dup", if p.dup() { "true" } else { "false" });
                        sender = sender.add_label("retain", if p.retain() { "true" } else { "false" });
                        sender = sender.add_label("qos", p.qos().value().to_string());
                        if let Some(packet_id) = p.packet_id() {
                            sender = sender.add_label("packet_id", packet_id.to_string());
                        }
                    }

                    //Must forward
                    sender = sender.add_label("topic", p.topic());

                    if let Err(e) = sender.data(p.payload).send().await {
                        log::warn!("{e}");
                    }
                }
            }
        }
        log::info!("exit reductstore producer.")
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

    pub async fn start(&mut self) -> Result<()> {
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
                    (b_cfg.name.clone(), entry_idx),
                );

                let producer =
                    Producer::from(Arc::new(b_cfg.clone()), entry.clone(), entry_idx, self.node_id).await?;
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
        for (topic_filter, bridge_infos) in { self.topics.read().await.matches(&topic) }.iter() {
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
