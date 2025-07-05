use std::collections::HashSet;
use std::convert::From as _;
use std::str::FromStr;
use std::sync::Arc;

use async_nats::{connect_with_options, header::HeaderMap, Client, ConnectOptions, ServerAddr, Subject};

use anyhow::anyhow;
use bytestring::ByteString;
use futures::SinkExt;
use tokio::sync::mpsc;
use tokio::sync::RwLock;

use rmqtt::{
    trie::{TopicTree, VecToTopic},
    types::{DashMap, From, NodeId, Publish, Topic},
    Result,
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
        // producer: Client,
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

        let producer = Self::build_nats(&cfg, &producer_name).await?;

        let (tx, rx) = mpsc::channel(100_000);
        tokio::spawn(async move {
            Self::start(cfg_entry, producer, rx).await;
        });
        Ok(Producer { name: producer_name, tx })
    }

    #[inline]
    async fn build_nats(cfg: &Bridge, producer_name: &str) -> Result<Client> {
        let addrs = cfg
            .servers
            .as_str()
            .split(',')
            .map(|url| url.parse())
            .collect::<std::result::Result<Vec<ServerAddr>, _>>()?;
        let mut opts = ConnectOptions::new();
        opts = opts.name(producer_name);

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

    async fn start(entry_cfg: Entry, mut producer: Client, mut rx: mpsc::Receiver<Command>) {
        let topic = Subject::from(entry_cfg.remote.topic.as_str());
        let forward_all_from = entry_cfg.remote.forward_all_from;
        let forward_all_publish = entry_cfg.remote.forward_all_publish;

        let jetstream = async_nats::jetstream::new(producer.clone());

        while let Some(cmd) = rx.recv().await {
            match cmd {
                Command::Close => {
                    if let Err(e) = producer.flush().await {
                        log::warn!("{e}");
                    }
                    if let Err(e) = producer.close().await {
                        log::warn!("{e}");
                    }
                    break;
                }
                Command::Start => {}
                Command::Message(f, p) => {
                    let mut properties = HeaderMap::new();

                    //Not required to forward
                    if forward_all_from {
                        properties.insert("from_type", f.typ().as_str());
                        properties.insert("from_node", f.node().to_string());
                        if let Some(addr) = f.remote_addr {
                            properties.insert("from_ipaddress", addr.to_string());
                        }
                        properties.insert("from_clientid", f.client_id.to_string());
                        properties.insert("from_username", f.username().as_ref());
                    }

                    //Not required to forward
                    if forward_all_publish {
                        properties.insert("dup", if p.dup { "true" } else { "false" });
                        properties.insert("retain", if p.retain { "true" } else { "false" });
                        properties.insert("qos", p.qos.value().to_string());
                        if let Some(packet_id) = p.packet_id {
                            properties.insert("packet_id", packet_id.to_string());
                        }
                    }

                    //Must forward
                    properties.insert("topic", p.topic.as_ref());

                    if let Err(e) = jetstream.publish_with_headers(topic.clone(), properties, p.payload).await
                    {
                        log::warn!("{e}");
                    }
                }
            }
        }
        log::info!("exit nats producer.")
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
