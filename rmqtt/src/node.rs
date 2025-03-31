use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use systemstat::Platform;

use crate::context::ServerContext;
#[cfg(feature = "grpc")]
use crate::grpc::{GrpcClient, GrpcServer};
use crate::utils::timestamp_millis;
use crate::{NodeId, TimestampMillis};

#[allow(dead_code)]
mod version {
    include!(concat!(env!("OUT_DIR"), "/version.rs"));
}

#[derive(Debug)]
pub struct Node {
    pub id: NodeId,
    pub start_time: chrono::DateTime<chrono::Local>,
    max_busy_loadavg: f32,
    max_busy_cpuloadavg: f32,
    busy_update_interval: Duration,
    cpuload: AtomicI64,
}

impl Default for Node {
    fn default() -> Self {
        Self::new(0, 80.0, 90.0, Duration::from_secs(2))
    }
}

impl Node {
    pub fn new(
        id: NodeId,
        max_busy_loadavg: f32,
        max_busy_cpuloadavg: f32,
        busy_update_interval: Duration,
    ) -> Self {
        Self {
            id,
            start_time: chrono::Local::now(),
            max_busy_loadavg,
            max_busy_cpuloadavg,
            busy_update_interval,
            cpuload: AtomicI64::new(0),
        }
    }

    #[inline]
    pub fn id(&self) -> NodeId {
        self.id
    }

    #[inline]
    pub async fn name(&self, scx: &ServerContext, id: NodeId) -> String {
        scx.extends.shared().await.node_name(id)
    }

    #[cfg(feature = "grpc")]
    #[inline]
    pub async fn new_grpc_client(
        &self,
        remote_addr: &str,
        client_timeout: Duration,
        client_concurrency_limit: usize,
        _batch_size: usize,
    ) -> crate::Result<GrpcClient> {
        let c = GrpcClient::new(remote_addr, client_timeout, client_concurrency_limit).await?;
        // c.start_ping();
        Ok(c)
    }

    #[cfg(feature = "grpc")]
    pub fn start_grpc_server(
        &self,
        scx: ServerContext,
        server_addr: std::net::SocketAddr,
        worker_threads: usize,
        reuseaddr: bool,
        reuseport: bool,
    ) {
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(worker_threads)
                .thread_name("grpc-server-worker")
                .thread_stack_size(4 * 1024 * 1024)
                .build()
                .expect("tokio runtime build failed");
            let runner = async {
                if let Err(e) = GrpcServer::new(scx).listen_and_serve(server_addr, reuseaddr, reuseport).await
                {
                    log::error!("listen and serve failure, {:?}, laddr: {:?}", e, server_addr);
                }
            };
            rt.block_on(runner)
        });
    }

    #[inline]
    pub async fn status(&self, scx: &ServerContext) -> NodeStatus {
        match scx.extends.shared().await.health_status().await {
            Ok(status) => {
                if status.running {
                    NodeStatus::Running(1)
                } else {
                    NodeStatus::Stop
                }
            }
            Err(e) => NodeStatus::Error(e.to_string()),
        }
    }

    #[inline]
    fn uptime(&self) -> String {
        to_uptime((chrono::Local::now() - self.start_time).num_seconds())
    }

    #[inline]
    pub async fn broker_info(&self, scx: &ServerContext) -> BrokerInfo {
        let node_id = self.id;
        BrokerInfo {
            version: version::VERSION.to_string(),
            rustc_version: version::RUSTC_VERSION.to_string(),
            uptime: self.uptime(),
            sysdescr: "RMQTT Broker".into(),
            node_status: self.status(scx).await,
            node_id,
            node_name: self.name(scx, node_id).await, //Runtime::instance().extends.shared().await.node_name(node_id),
            datetime: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        }
    }

    #[inline]
    pub async fn node_info(&self, scx: &ServerContext) -> NodeInfo {
        let node_id = self.id;

        let sys = systemstat::System::new();
        let boottime = sys.boot_time().map(|t| t.to_string()).unwrap_or_default();
        let loadavg = sys.load_average();
        let mem_info = sys.memory();

        let (disk_total, disk_free) = if let Ok(mounts) = sys.mounts() {
            let total = mounts.iter().map(|m| m.total.as_u64()).sum();
            let free = mounts.iter().map(|m| m.free.as_u64()).sum();
            (total, free)
        } else {
            (0, 0)
        };

        NodeInfo {
            connections: scx.connections.count(),
            boottime,
            load1: loadavg.as_ref().map(|l| l.one).unwrap_or_default(),
            load5: loadavg.as_ref().map(|l| l.five).unwrap_or_default(),
            load15: loadavg.as_ref().map(|l| l.fifteen).unwrap_or_default(),
            memory_total: mem_info.as_ref().map(|m| m.total.as_u64()).unwrap_or_default(),
            memory_free: mem_info.as_ref().map(|m| m.free.as_u64()).unwrap_or_default(),
            memory_used: mem_info
                .as_ref()
                .map(|m| systemstat::saturating_sub_bytes(m.total, m.free).as_u64())
                .unwrap_or_default(),
            disk_total,
            disk_free,
            node_status: self.status(scx).await,
            node_id,
            node_name: self.name(scx, node_id).await, //Runtime::instance().extends.shared().await.node_name(node_id),
            uptime: self.uptime(),
            version: version::VERSION.to_string(),
            rustc_version: version::RUSTC_VERSION.to_string(),
        }
    }

    #[inline]
    fn _is_busy(&self) -> bool {
        let sys = systemstat::System::new();
        let cpuload = self.cpuload();

        let loadavg = sys.load_average();
        let load1 = loadavg.as_ref().map(|l| l.one).unwrap_or_default();

        load1 > self.max_busy_loadavg || cpuload > self.max_busy_cpuloadavg
    }

    #[inline]
    pub fn sys_is_busy(&self) -> bool {
        static CACHED_BUSY: AtomicBool = AtomicBool::new(false);
        static CACHED_TIME: AtomicI64 = AtomicI64::new(0);

        let now = timestamp_millis();

        let last_update = CACHED_TIME.load(Ordering::Relaxed);

        if now - last_update < self.busy_update_interval.as_millis() as TimestampMillis {
            return CACHED_BUSY.load(Ordering::Relaxed);
        }

        let busy = self._is_busy();

        CACHED_BUSY.store(busy, Ordering::Relaxed);
        CACHED_TIME.store(now, Ordering::Relaxed);

        busy
    }

    #[inline]
    pub fn cpuload(&self) -> f32 {
        //0.0 - 100.0
        self.cpuload.load(Ordering::SeqCst) as f32 / 100.0
    }

    pub async fn update_cpuload(&self) {
        let sys = systemstat::System::new();
        let cpuload_aggr = sys.cpu_load_aggregate().ok();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let cpuload_aggr = cpuload_aggr.and_then(|dm| dm.done().ok());
        let cpuload = cpuload_aggr
            .map(|cpuload_aggr| {
                let aggregate1 =
                    cpuload_aggr.user + cpuload_aggr.nice + cpuload_aggr.system + cpuload_aggr.interrupt;
                let aggregate2 = aggregate1 + cpuload_aggr.idle;
                if aggregate2 <= 0.0 {
                    1.0
                } else {
                    aggregate2
                };
                aggregate1 / aggregate2 * 10_000.0
            })
            .unwrap_or_default();

        self.cpuload.store(cpuload as i64, Ordering::SeqCst);
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BrokerInfo {
    pub version: String,
    pub rustc_version: String,
    pub uptime: String,
    pub sysdescr: String,
    pub node_status: NodeStatus,
    pub node_id: NodeId,
    pub node_name: String,
    pub datetime: String,
}

impl BrokerInfo {
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "version": self.version,
            "rustc_version": self.rustc_version,
            "uptime": self.uptime,
            "sysdescr": self.sysdescr,
            "running": self.node_status.is_running(),
            "node_id": self.node_id,
            "node_name": self.node_name,
            "datetime": self.datetime
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NodeInfo {
    pub connections: isize,
    pub boottime: String,
    pub load1: f32,
    pub load5: f32,
    pub load15: f32,
    // pub max_fds: usize,
    // pub cpu_num: String,
    // pub cpu_speed: String,
    pub memory_total: u64,
    pub memory_used: u64,
    pub memory_free: u64,
    pub disk_total: u64,
    pub disk_free: u64,
    // pub os_release: String,
    // pub os_type: String,
    // pub proc_total: String,
    pub node_status: NodeStatus,
    pub node_id: NodeId,
    pub node_name: String,
    pub uptime: String,
    pub version: String,
    pub rustc_version: String,
}

impl NodeInfo {
    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "connections":  self.connections,
            "boottime":  self.boottime,
            "load1":  self.load1,
            "load5":  self.load5,
            "load15":  self.load15,
            // "max_fds":  self.max_fds,
            // "cpu_num":  self.cpu_num,
            // "cpu_speed":  self.cpu_speed,
            "memory_total":  self.memory_total,
            "memory_used":  self.memory_used,
            "memory_free":  self.memory_free,
            "disk_total":  self.disk_total,
            "disk_free":  self.disk_free,
            // "os_release":  self.os_release,
            // "os_type":  self.os_type,
            // "proc_total":  self.proc_total,
            "running":  self.node_status.is_running(),
            "node_id":  self.node_id,
            "node_name":  self.node_name,
            "uptime":  self.uptime,
            "version":  self.version,
            "rustc_version": self.rustc_version,
        })
    }

    #[inline]
    pub fn add(&mut self, other: &NodeInfo) {
        self.load1 += other.load1;
        self.load5 += other.load5;
        self.load15 += other.load15;
        self.memory_total += other.memory_total;
        self.memory_used += other.memory_used;
        self.memory_free += other.memory_free;
        self.disk_total += other.disk_total;
        self.disk_free += other.disk_free;
        self.node_status = {
            let c = match (&self.node_status, &other.node_status) {
                (NodeStatus::Running(c1), NodeStatus::Running(c2)) => *c1 + *c2,
                (NodeStatus::Running(c1), _) => *c1,
                (_, NodeStatus::Running(c2)) => *c2,
                (_, _) => 0,
            };
            NodeStatus::Running(c)
        };
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "lowercase")]
pub enum NodeStatus {
    Running(usize),
    Stop,
    Error(String),
}

impl NodeStatus {
    #[inline]
    pub fn running(&self) -> usize {
        if let NodeStatus::Running(c) = self {
            *c
        } else {
            0
        }
    }

    #[inline]
    pub fn is_running(&self) -> bool {
        matches!(self, NodeStatus::Running(_))
    }
}

impl Default for NodeStatus {
    #[inline]
    fn default() -> Self {
        NodeStatus::Stop
    }
}

#[inline]
pub fn to_uptime(uptime: i64) -> String {
    let uptime_secs = uptime % 60;
    let uptime = uptime / 60;
    let uptime_minus = uptime % 60;
    let uptime = uptime / 60;
    let uptime_hours = uptime % 24;
    let uptime_days = uptime / 24;
    format!("{} days {} hours, {} minutes, {} seconds", uptime_days, uptime_hours, uptime_minus, uptime_secs)
}
