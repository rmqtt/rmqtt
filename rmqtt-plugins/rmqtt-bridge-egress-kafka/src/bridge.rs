use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;

use rdkafka::config::{ClientConfig as KafkaClientConfig, RDKafkaLogLevel};
use rdkafka::message::{Header, OwnedHeaders};
use rdkafka::producer::{FutureProducer, FutureRecord};

use rmqtt::anyhow::anyhow;
use rmqtt::bytestring::ByteString;
use rmqtt::rust_box::task_exec_queue::SpawnExt;
use rmqtt::{
    broker::topic::{TopicTree, VecToTopic},
    timestamp_millis, From, MqttError, NodeId, Publish, QoSEx, Result, Topic,
};
use rmqtt::{
    itoa, log, rand,
    rust_box::task_exec_queue::{Builder, TaskExecQueue},
    tokio,
    tokio::sync::RwLock,
    DashMap,
};

use crate::config::{Bridge, Entry, PluginConfig};

#[derive(Debug)]
pub enum Command {
    Start,
    Close,
}

#[derive(Clone)]
pub struct Producer {
    pub(crate) client_id: Option<ByteString>,
    pub(crate) cfg: Arc<Bridge>,
    pub(crate) cfg_entry: Entry,
    pub(crate) producer: FutureProducer,
}

impl Producer {
    pub(crate) fn from(
        cfg: Arc<Bridge>,
        cfg_entry: Entry,
        entry_idx: usize,
        node_id: NodeId,
        client_no: usize,
    ) -> Result<Self> {
        let mut client_cfg = KafkaClientConfig::new();

        client_cfg.set("bootstrap.servers", cfg.servers.as_str());

        let client_id = if let Some(client_id_prefix) = &cfg.client_id_prefix {
            let client_id =
                format!("{}:{}:egress:{}:{}:{}", client_id_prefix, cfg.name, node_id, entry_idx, client_no);
            log::info!("client: {}", client_id.as_str());
            client_cfg.set("client.id", client_id.as_str());
            Some(ByteString::from(client_id))
        } else {
            None
        };

        for (key, val) in &cfg.properties {
            if !key.trim_start().starts_with('#') {
                log::info!("{}={}", key, val);
                client_cfg.set(key, val);
            }
        }

        client_cfg.set_log_level(RDKafkaLogLevel::Info);
        let producer: FutureProducer = client_cfg.create().map_err(|e| anyhow!(e))?;

        Ok(Producer { client_id, cfg, cfg_entry, producer })
    }

    #[inline]
    pub(crate) async fn send(&self, exec: &TaskExecQueue, f: &From, p: &Publish) -> Result<()> {
        let mut headers = OwnedHeaders::new();
        headers = headers.insert(Header { key: "from_type", value: Some(f.typ().as_str()) });
        headers =
            headers.insert(Header { key: "from_node", value: Some(itoa::Buffer::new().format(f.node())) });
        headers = headers
            .insert(Header { key: "from_ipaddress", value: f.remote_addr.map(|a| a.to_string()).as_ref() });
        headers = headers.insert(Header { key: "from_clientid", value: Some(f.client_id.as_str()) });
        headers = headers.insert(Header { key: "from_username", value: Some(f.username_ref()) });

        headers = headers.insert(Header { key: "dup", value: Some(p.dup().as_str()) });
        headers = headers.insert(Header { key: "retain", value: Some(p.retain().as_str()) });
        headers =
            headers.insert(Header { key: "qos", value: Some(itoa::Buffer::new().format(p.qos().value())) });
        if let Some(packet_id) = p.packet_id() {
            headers = headers
                .insert(Header { key: "packet_id", value: Some(itoa::Buffer::new().format(packet_id)) });
        }
        headers =
            headers.insert(Header { key: "ts", value: Some(itoa::Buffer::new().format(p.create_time())) });
        headers = headers
            .insert(Header { key: "time", value: Some(itoa::Buffer::new().format(timestamp_millis())) });
        headers = headers.insert(Header { key: "topic", value: Some(p.topic().as_str()) });

        let topic = self.cfg_entry.remote.make_topic(&p.topic);
        let payload = p.payload().clone();
        let queue_timeout = self.cfg_entry.remote.queue_timeout;
        let partition = self.cfg_entry.remote.partition;
        let name = self.cfg.name.clone();
        let producer = self.producer.clone();
        if let Err(e) = async move {
            let mut frecord: FutureRecord<(), _> =
                FutureRecord::to(&topic).payload(payload.as_ref()).headers(headers);

            if let Some(part) = partition {
                frecord = frecord.partition(part);
            }
            frecord = frecord.timestamp(timestamp_millis());

            let delivery_status = producer.send(frecord, queue_timeout).await;
            match delivery_status {
                Ok((partition, offset)) => {
                    log::debug!("{} delivery ok, partition: {}, offset: {}", name, partition, offset);
                }
                Err((e, msg)) => {
                    log::error!("{} delivery error: {:?}, message: {:?}", name, e, msg);
                }
            };
        }
        .spawn(exec)
        .await
        {
            log::error!("{} task exec error, {}", self.cfg.name, e.to_string());
        }
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
    sinks: Arc<DashMap<SourceKey, Vec<Producer>>>,
    topics: Arc<RwLock<TopicTree<(BridgeName, EntryIndex)>>>,
    pub(crate) exec: TaskExecQueue,
}

impl BridgeManager {
    pub async fn new(node_id: NodeId, cfg: Arc<RwLock<PluginConfig>>) -> Self {
        Self {
            node_id,
            cfg: cfg.clone(),
            sinks: Arc::new(DashMap::default()),
            topics: Arc::new(RwLock::new(TopicTree::default())),
            exec: Self::init_task_exec_queue(
                cfg.read().await.task_concurrency_limit,
                cfg.read().await.task_queue_capacity,
            ),
        }
    }

    #[inline]
    fn init_task_exec_queue(workers: usize, queue_max: usize) -> TaskExecQueue {
        let (exec, task_runner) = Builder::default().workers(workers).queue_max(queue_max).build();

        tokio::spawn(async move {
            task_runner.await;
        });

        exec
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
                log::info!("entry.local.topic_filter: {}", entry.local.topic_filter);
                topics.insert(
                    &Topic::from_str(entry.local.topic_filter.as_str())?,
                    (b_cfg.name.clone(), entry_idx),
                );

                for client_no in 0..b_cfg.concurrent_client_limit {
                    let producer = Producer::from(
                        Arc::new(b_cfg.clone()),
                        entry.clone(),
                        entry_idx,
                        self.node_id,
                        client_no,
                    )?;
                    self.sinks.entry((b_cfg.name.clone(), entry_idx)).or_default().push(producer);
                }
            }
        }
        Ok(())
    }

    pub async fn stop(&mut self) {
        for mut entry in &mut self.sinks.iter_mut() {
            let ((bridge_name, entry_idx), producers) = entry.pair_mut();
            for (client_no, _producer) in producers.iter_mut().enumerate() {
                log::debug!(
                    "stop bridge_name: {:?}, entry_idx: {:?}, client_no: {:?}",
                    bridge_name,
                    entry_idx,
                    client_no
                );
            }
        }
        self.sinks.clear();
    }

    #[allow(unused)]
    pub(crate) fn sinks(&self) -> &DashMap<SourceKey, Vec<Producer>> {
        &self.sinks
    }

    #[inline]
    pub(crate) async fn send(&self, f: &From, p: &Publish) -> Result<()> {
        let topic = Topic::from_str(&p.topic)?;
        let rnd = rand::random::<u64>() as usize;
        for (topic_filter, bridge_infos) in { self.topics.read().await.matches(&topic) }.iter() {
            let topic_filter = topic_filter.to_topic_filter();
            log::debug!("topic_filter: {:?}", topic_filter);
            log::debug!("bridge_infos: {:?}", bridge_infos);
            for (name, entry_idx) in bridge_infos {
                if let Some(producers) = self.sinks.get(&(name.clone(), *entry_idx)) {
                    let client_no = rnd % producers.len();
                    if let Some(producer) = producers.get(client_no) {
                        if let Err(e) = producer.send(&self.exec, f, p).await {
                            log::warn!("{}", e);
                        }
                    }
                }
            }
        }
        Ok(())
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
