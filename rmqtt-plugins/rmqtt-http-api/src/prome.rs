use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rmqtt::prometheus::{
    register_gauge_vec_with_registry, register_int_gauge_vec_with_registry, Encoder, GaugeVec, IntGaugeVec,
    Registry, Result as PromeResult, TextEncoder,
};

use rmqtt::metrics::Metrics;
use rmqtt::{anyhow::anyhow, once_cell::sync::OnceCell};
use rmqtt::{grpc::MessageType, node::NodeInfo, stats::Stats, timestamp_secs, Result, Runtime};

use crate::api::{get_metrics_all, get_nodes_all, get_stats_all};

#[inline]
pub async fn to_metrics(message_type: MessageType, cache_interval: Duration) -> Result<Vec<u8>> {
    static INSTANCE: OnceCell<PromeResult<Monitor>> = OnceCell::new();
    let monitor = INSTANCE.get_or_init(Monitor::new).as_ref().map_err(|e| anyhow!(format!("{:?}", e)))?;
    monitor.refresh_data(message_type, cache_interval).await?;
    monitor.to_metrics()
}

#[derive(Clone)]
pub struct Monitor {
    reg: Registry,
    last_refresh_time: Arc<AtomicI64>,
    //Node Status Data
    nodes_gauge_vec: GaugeVec,
    //Node Status Data
    stats_gauge_vec: IntGaugeVec,
    //Node Metric Data
    metrics_gauge_vec: IntGaugeVec,
}

impl Monitor {
    fn new() -> PromeResult<Monitor> {
        let reg = Registry::new();

        let nodes_gauge_vec =
            register_gauge_vec_with_registry!("rmqtt_nodes", "All nodes status", &["node", "item"], reg)?;

        let stats_gauge_vec =
            register_int_gauge_vec_with_registry!("rmqtt_stats", "All status data", &["node", "item"], reg)?;

        let metrics_gauge_vec = register_int_gauge_vec_with_registry!(
            "rmqtt_metrics",
            "All metrics data",
            &["node", "item"],
            reg
        )?;

        Ok(Self {
            reg,
            last_refresh_time: Arc::new(AtomicI64::new(0)),
            nodes_gauge_vec,
            stats_gauge_vec,
            metrics_gauge_vec,
        })
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
    async fn refresh_data(&self, message_type: MessageType, refresh_interval: Duration) -> Result<()> {
        // if !Runtime::instance().node.status().await.env.is_started() {
        //     return;
        // }
        if !self.refresh_enable(refresh_interval) {
            return Ok(());
        }

        let mut node_info_all = NodeInfo::default();
        let mut stats_all = Stats::default();
        let mut metrics_all = Metrics::default();

        for node_info in get_nodes_all(message_type).await?.into_iter().flatten() {
            let node = node_info.node_id.to_string();
            self.nodes_gauge_vec_sets(&node, &node_info).await;
            node_info_all.add(&node_info);
        }

        for (node_id, _, stats) in get_stats_all(message_type).await?.into_iter().flatten() {
            let node = node_id.to_string();
            self.stats_gauge_vec_sets(&node, &stats).await;
            stats_all.add(*stats);
        }

        for (node_id, metrics) in get_metrics_all(message_type).await?.into_iter().flatten() {
            let node = node_id.to_string();
            metrics.build_prometheus_metrics(&node, &self.metrics_gauge_vec);
            metrics_all.add(&metrics);
        }

        self.nodes_gauge_vec_sets("all", &node_info_all).await;
        self.stats_gauge_vec_sets("all", &stats_all).await;
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
    async fn stats_gauge_vec_sets(&self, label: &str, stats: &Stats) {
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

        let router = Runtime::instance().extends.router().await;
        let topics = router.merge_topics(&stats.topics_map);
        let routes = router.merge_routes(&stats.routes_map);

        self.stats_gauge_vec.with_label_values(&[label, "topics.count"]).set(topics.count() as i64);
        self.stats_gauge_vec.with_label_values(&[label, "topics.max"]).set(topics.max() as i64);

        self.stats_gauge_vec.with_label_values(&[label, "routes.count"]).set(routes.count() as i64);
        self.stats_gauge_vec.with_label_values(&[label, "routes.max"]).set(routes.max() as i64);
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
