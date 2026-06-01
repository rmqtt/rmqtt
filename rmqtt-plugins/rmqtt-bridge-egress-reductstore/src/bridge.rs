//! Egress bridge to ReductStore.
//!
//! Routes MQTT publish messages to ReductStore as time-series records.
//! Manages ReductStore client connections, bucket creation, topic matching
//! via a trie, and stores MQTT metadata as record labels.

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::SystemTime;

use reduct_rs::{Bucket, QuotaType, ReductClient};

use anyhow::anyhow;
use bytestring::ByteString;
use reqwest::Url;
use tokio::sync::mpsc;
use tokio::sync::RwLock;

use rmqtt::{
    trie::{TopicTree, VecToTopic},
    types::{DashMap, From, NodeId, Publish, Topic},
    Result,
};

use crate::config::{Bridge, Entry, PluginConfig};

/// Commands for controlling a ReductStore producer.
#[derive(Debug)]
pub enum Command {
    Start,
    Close,
    Message(From, Publish),
}

/// A ReductStore producer that writes MQTT messages as time-series records.
pub struct Producer {
    pub(crate) name: String,
    cfg_entry: Entry,
    tx: mpsc::Sender<Command>,
}

impl Producer {
    /// Creates a new ReductStore producer and spawns its processing loop.
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

        let bucket = builder.send().await.map_err(|e| anyhow!(format!("Failed to create Bucket, {}", e)))?;

        let cfg_entry1 = cfg_entry.clone();
        let (tx, rx) = mpsc::channel(100_000);
        tokio::spawn(async move {
            Self::start(cfg_entry1, bucket, rx).await;
        });
        Ok(Producer { name: producer_name, cfg_entry, tx })
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

        builder.try_build().map_err(|e| anyhow!(format!("Failed to connect to Reductstore, {}", e)))
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
                        sender = sender.add_label("dup", if p.dup { "true" } else { "false" });
                        sender = sender.add_label("retain", if p.retain { "true" } else { "false" });
                        sender = sender.add_label("qos", p.qos.value().to_string());
                        if let Some(packet_id) = p.packet_id {
                            sender = sender.add_label("packet_id", packet_id.to_string());
                        }
                    }

                    //Must forward
                    sender = sender.add_label("topic", p.topic.clone());
                    if let Err(e) = sender.data(p.take_payload()).send().await {
                        log::warn!("{e}");
                    }
                }
            }
        }
        log::info!("exit reductstore producer.")
    }

    /// Sends an MQTT publish to the ReductStore producer, applying skip-levels.
    #[inline]
    pub(crate) async fn send(&self, f: &From, p: &Publish, topic: &Topic) -> Result<()> {
        let p = if self.cfg_entry.remote.skip_levels > 0 {
            let mut p = p.clone();
            p.topic = ByteString::from(topic.to_string_skip(self.cfg_entry.remote.skip_levels));
            p
        } else {
            p.clone()
        };
        log::debug!("new_local_topic: {}, skip_levels: {}", p.topic, self.cfg_entry.remote.skip_levels);
        self.tx.send(Command::Message(f.clone(), p)).await?;
        Ok(())
    }
}

/// A named bridge instance identifier.
pub(crate) type BridgeName = ByteString;
type SourceKey = (BridgeName, EntryIndex);

type EntryIndex = usize;

/// Manages ReductStore producers and topic routing for bridge entries.
#[derive(Clone)]
pub(crate) struct BridgeManager {
    node_id: NodeId,
    cfg: Arc<RwLock<PluginConfig>>,
    sinks: Arc<DashMap<SourceKey, Producer>>,
    topics: Arc<RwLock<TopicTree<(BridgeName, EntryIndex)>>>,
}

impl BridgeManager {
    /// Creates a new `BridgeManager` with the given node ID and configuration.
    pub async fn new(node_id: NodeId, cfg: Arc<RwLock<PluginConfig>>) -> Self {
        Self {
            node_id,
            cfg: cfg.clone(),
            sinks: Arc::new(DashMap::default()),
            topics: Arc::new(RwLock::new(TopicTree::default())),
        }
    }

    /// Starts all configured bridge entries by creating ReductStore producers.
    pub async fn start(&mut self) -> Result<()> {
        let mut topics = self.topics.write().await;
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

    /// Stops all bridge entries by sending close commands.
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

    /// Returns a reference to the internal producer map.
    #[allow(unused)]
    pub(crate) fn sinks(&self) -> &DashMap<SourceKey, Producer> {
        &self.sinks
    }

    /// Routes an MQTT publish to matching ReductStore producers.
    #[inline]
    pub(crate) async fn send(&self, f: &From, p: &Publish) -> Result<()> {
        let topic = Topic::from_str(&p.topic)?;
        for (topic_filter, bridge_infos) in { self.topics.read().await.matches(&topic) }.iter() {
            let topic_filter = topic_filter.to_topic_filter();
            log::debug!("topic_filter: {topic_filter:?}");
            log::debug!("bridge_infos: {bridge_infos:?}");
            for (name, entry_idx) in bridge_infos {
                if let Some(producer) = self.sinks.get(&(name.clone(), *entry_idx)) {
                    if let Err(e) = producer.send(f, p, &topic).await {
                        log::warn!("{e}");
                    }
                }
            }
        }
        Ok(())
    }
}
