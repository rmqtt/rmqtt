use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use prometheus::{
    register_gauge_vec_with_registry, register_int_gauge_vec_with_registry, Encoder, GaugeVec, IntGaugeVec,
    Registry, TextEncoder,
};

pub(crate) const PROME_MONITOR: &str = "PROME_MONITOR";

use rmqtt::{
    context::ServerContext,
    grpc::MessageType,
    metrics::Metrics,
    node::NodeInfo,
    stats::Stats,
    types::{DashMap, NodeId},
    utils::timestamp_secs,
    Result,
};

use crate::api::{get_metrics_all, get_metrics_one, get_node, get_nodes_all, get_stats_all, get_stats_one};
use crate::types::PrometheusDataType;

#[inline]
pub async fn to_metrics(
    scx: &ServerContext,
    monitor: Monitor,
    message_type: MessageType,
    cache_interval: Duration,
    typ: PrometheusDataType,
) -> Result<Vec<u8>> {
    monitor.refresh_data(scx, message_type, cache_interval, typ).await?;
    monitor.to_metrics(typ)
}

#[derive(Clone)]
struct MonitorData {
    typ: PrometheusDataType,
    reg: Registry,
    //Node Status Data
    nodes_gauge_vec: GaugeVec,
    //Node Status Data
    stats_gauge_vec: IntGaugeVec,
    //Node Metric Data
    metrics_gauge_vec: IntGaugeVec,
}

impl MonitorData {
    fn new(typ: PrometheusDataType) -> Option<Self> {
        let reg = Registry::new();

        let nodes_gauge_vec =
            register_gauge_vec_with_registry!("rmqtt_nodes", "nodes status", &["node", "item"], reg).ok()?;

        let stats_gauge_vec =
            register_int_gauge_vec_with_registry!("rmqtt_stats", "status data", &["node", "item"], reg)
                .ok()?;

        let metrics_gauge_vec =
            register_int_gauge_vec_with_registry!("rmqtt_metrics", "metrics data", &["node", "item"], reg)
                .ok()?;

        Some(Self { typ, reg, nodes_gauge_vec, stats_gauge_vec, metrics_gauge_vec })
    }

    #[inline]
    fn to_metrics(&self) -> Result<Vec<u8>> {
        let mut buffer = vec![];
        let encoder = TextEncoder::new();
        let familiy = self.reg.gather();
        encoder.encode(&familiy, &mut buffer).map_err(|e| anyhow!(e))?;
        Ok(buffer)
    }

    #[inline]
    async fn refresh_data(&self, scx: &ServerContext, message_type: MessageType) -> Result<()> {
        match self.typ {
            PrometheusDataType::All => {
                self.refresh_data_all(scx, message_type, false).await?;
            }
            PrometheusDataType::Sum => {
                self.refresh_data_all(scx, message_type, true).await?;
            }
            PrometheusDataType::Node(node_id) => {
                self.refresh_data_one(scx, message_type, node_id).await?;
            }
        }

        Ok(())
    }

    #[inline]
    async fn refresh_data_one(
        &self,
        scx: &ServerContext,
        message_type: MessageType,
        node_id: NodeId,
    ) -> Result<()> {
        let node = node_id.to_string();

        let node_info = get_node(scx, message_type, node_id)
            .await?
            .ok_or_else(|| anyhow!(format!("node({}) does not exist", node_id)))?;
        self.nodes_gauge_vec_sets(&node, &node_info).await;

        let (_, stats) = get_stats_one(scx, message_type, node_id)
            .await?
            .ok_or_else(|| anyhow!(format!("node({}) does not exist", node_id)))?;
        self.stats_gauge_vec_sets(scx, &node, &stats).await;

        let metrics = get_metrics_one(scx, message_type, node_id)
            .await?
            .ok_or_else(|| anyhow!(format!("node({}) does not exist", node_id)))?;
        metrics.build_prometheus_metrics(&node, &self.metrics_gauge_vec);

        Ok(())
    }

    #[inline]
    async fn refresh_data_all(
        &self,
        scx: &ServerContext,
        message_type: MessageType,
        only_sum: bool,
    ) -> Result<()> {
        let mut node_info_all = NodeInfo::default();
        let mut stats_all = Stats::default();
        let mut metrics_all = Metrics::default();

        for node_info in get_nodes_all(scx, message_type).await?.into_iter().flatten() {
            let node = node_info.node_id.to_string();
            if !only_sum {
                self.nodes_gauge_vec_sets(&node, &node_info).await;
            }
            node_info_all.add(&node_info);
        }

        for (node_id, _, stats) in get_stats_all(scx, message_type).await?.into_iter().flatten() {
            let node = node_id.to_string();
            if !only_sum {
                self.stats_gauge_vec_sets(scx, &node, &stats).await;
            }
            stats_all.add(*stats);
        }

        for (node_id, metrics) in get_metrics_all(scx, message_type).await?.into_iter().flatten() {
            let node = node_id.to_string();
            if !only_sum {
                metrics.build_prometheus_metrics(&node, &self.metrics_gauge_vec);
            }
            metrics_all.add(&metrics);
        }

        self.nodes_gauge_vec_sets("all", &node_info_all).await;
        self.stats_gauge_vec_sets(scx, "all", &stats_all).await;
        metrics_all.build_prometheus_metrics("all", &self.metrics_gauge_vec);

        Ok(())
    }

    #[inline]
    async fn nodes_gauge_vec_sets(&self, label: &str, node_info: &NodeInfo) {
        self.nodes_gauge_vec.with_label_values(&[label, "load1"]).set(node_info.load1 as f64);
        self.nodes_gauge_vec.with_label_values(&[label, "load5"]).set(node_info.load5 as f64);
        self.nodes_gauge_vec.with_label_values(&[label, "load15"]).set(node_info.load15 as f64);

        self.nodes_gauge_vec.with_label_values(&[label, "memory_total"]).set(node_info.memory_total as f64);
        self.nodes_gauge_vec.with_label_values(&[label, "memory_used"]).set(node_info.memory_used as f64);
        self.nodes_gauge_vec.with_label_values(&[label, "memory_free"]).set(node_info.memory_free as f64);

        self.nodes_gauge_vec.with_label_values(&[label, "disk_total"]).set(node_info.disk_total as f64);
        self.nodes_gauge_vec.with_label_values(&[label, "disk_free"]).set(node_info.disk_free as f64);

        self.nodes_gauge_vec
            .with_label_values(&[label, "running"])
            .set(node_info.node_status.running() as f64);
    }

    #[inline]
    async fn stats_gauge_vec_sets(&self, scx: &ServerContext, label: &str, stats: &Stats) {
        self.stats_gauge_vec
            .with_label_values(&[label, "handshakings.count"])
            .set(stats.handshakings.count() as i64);
        self.stats_gauge_vec
            .with_label_values(&[label, "handshakings.max"])
            .set(stats.handshakings.max() as i64);
        self.stats_gauge_vec
            .with_label_values(&[label, "handshakings_active.count"])
            .set(stats.handshakings_active.count() as i64);
        self.stats_gauge_vec
            .with_label_values(&[label, "handshakings_rate.count"])
            .set(stats.handshakings_rate.count() as i64);
        self.stats_gauge_vec
            .with_label_values(&[label, "handshakings_rate.max"])
            .set(stats.handshakings_rate.max() as i64);
        self.stats_gauge_vec
            .with_label_values(&[label, "connections.count"])
            .set(stats.connections.count() as i64);
        self.stats_gauge_vec
            .with_label_values(&[label, "connections.max"])
            .set(stats.connections.max() as i64);
        self.stats_gauge_vec.with_label_values(&[label, "sessions.count"]).set(stats.sessions.count() as i64);
        self.stats_gauge_vec.with_label_values(&[label, "sessions.max"]).set(stats.sessions.max() as i64);

        self.stats_gauge_vec
            .with_label_values(&[label, "subscriptions.count"])
            .set(stats.subscriptions.count() as i64);
        self.stats_gauge_vec
            .with_label_values(&[label, "subscriptions.max"])
            .set(stats.subscriptions.max() as i64);

        self.stats_gauge_vec
            .with_label_values(&[label, "subscriptions_shared.count"])
            .set(stats.subscriptions_shared.count() as i64);
        self.stats_gauge_vec
            .with_label_values(&[label, "subscriptions_shared.max"])
            .set(stats.subscriptions_shared.max() as i64);

        self.stats_gauge_vec
            .with_label_values(&[label, "retaineds.count"])
            .set(stats.retaineds.count() as i64);
        self.stats_gauge_vec.with_label_values(&[label, "retaineds.max"]).set(stats.retaineds.max() as i64);

        self.stats_gauge_vec
            .with_label_values(&[label, "message_queues.count"])
            .set(stats.message_queues.count() as i64);
        self.stats_gauge_vec
            .with_label_values(&[label, "message_queues.max"])
            .set(stats.message_queues.max() as i64);

        self.stats_gauge_vec
            .with_label_values(&[label, "out_inflights.count"])
            .set(stats.out_inflights.count() as i64);
        self.stats_gauge_vec
            .with_label_values(&[label, "out_inflights.max"])
            .set(stats.out_inflights.max() as i64);

        self.stats_gauge_vec
            .with_label_values(&[label, "in_inflights.count"])
            .set(stats.in_inflights.count() as i64);
        self.stats_gauge_vec
            .with_label_values(&[label, "in_inflights.max"])
            .set(stats.in_inflights.max() as i64);

        self.stats_gauge_vec.with_label_values(&[label, "forwards.count"]).set(stats.forwards.count() as i64);
        self.stats_gauge_vec.with_label_values(&[label, "forwards.max"]).set(stats.forwards.max() as i64);

        self.stats_gauge_vec
            .with_label_values(&[label, "message_storages.count"])
            .set(stats.message_storages.count() as i64);
        self.stats_gauge_vec
            .with_label_values(&[label, "message_storages.max"])
            .set(stats.message_storages.max() as i64);

        self.stats_gauge_vec
            .with_label_values(&[label, "delayed_publishs.count"])
            .set(stats.delayed_publishs.count() as i64);
        self.stats_gauge_vec
            .with_label_values(&[label, "delayed_publishs.max"])
            .set(stats.delayed_publishs.max() as i64);

        let router = scx.extends.router().await;
        let topics = router.merge_topics(&stats.topics_map);
        let routes = router.merge_routes(&stats.routes_map);

        self.stats_gauge_vec.with_label_values(&[label, "topics.count"]).set(topics.count() as i64);
        self.stats_gauge_vec.with_label_values(&[label, "topics.max"]).set(topics.max() as i64);

        self.stats_gauge_vec.with_label_values(&[label, "routes.count"]).set(routes.count() as i64);
        self.stats_gauge_vec.with_label_values(&[label, "routes.max"]).set(routes.max() as i64);
    }
}

#[derive(Clone)]
pub struct Monitor {
    last_refresh_time: Arc<AtomicI64>,
    catcheds: Arc<DashMap<PrometheusDataType, Option<MonitorData>>>,
}

impl Monitor {
    pub fn new() -> Monitor {
        Self { last_refresh_time: Arc::new(AtomicI64::new(0)), catcheds: Arc::new(DashMap::default()) }
    }

    #[inline]
    fn to_metrics(&self, typ: PrometheusDataType) -> Result<Vec<u8>> {
        if let Some(md) = self.catcheds.entry(typ).or_insert_with(|| MonitorData::new(typ)).value() {
            md.to_metrics()
        } else {
            Err(anyhow!("monitor data is not initialized"))
        }
    }

    #[inline]
    async fn refresh_data(
        &self,
        scx: &ServerContext,
        message_type: MessageType,
        refresh_interval: Duration,
        typ: PrometheusDataType,
    ) -> Result<()> {
        if !self.refresh_enable(refresh_interval) {
            return Ok(());
        }

        if let Some(md) = self.catcheds.entry(typ).or_insert_with(|| MonitorData::new(typ)).value() {
            md.nodes_gauge_vec.reset();
            md.stats_gauge_vec.reset();
            md.metrics_gauge_vec.reset();
            if let Err(e) = md.refresh_data(scx, message_type).await {
                log::warn!("refresh data error, {e:?}")
            }
        } else {
            return Err(anyhow!("monitor data is not initialized"));
        }

        Ok(())
    }

    #[inline]
    fn refresh_enable(&self, refresh_interval: Duration) -> bool {
        let timestamp_secs = timestamp_secs();
        if timestamp_secs
            > (self.last_refresh_time.load(Ordering::SeqCst) + refresh_interval.as_secs() as i64)
        {
            self.last_refresh_time.store(timestamp_secs, Ordering::SeqCst);
            true
        } else {
            false
        }
    }
}
