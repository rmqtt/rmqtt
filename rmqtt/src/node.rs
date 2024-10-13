use std::ops::Deref;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use rust_box::std_ext::RwLock;
use systemstat::Platform;

use crate::grpc::client::NodeGrpcClient;
use crate::grpc::server::Server;
use crate::{NodeId, Result, Runtime};

#[allow(dead_code)]
mod version {
    include!(concat!(env!("OUT_DIR"), "/version.rs"));
}

pub struct Node {
    pub start_time: chrono::DateTime<chrono::Local>,
    cpuload: AtomicI64,
}

impl Node {
    pub(crate) fn new() -> Self {
        Self { start_time: chrono::Local::now(), cpuload: AtomicI64::new(0) }
    }

    #[inline]
    pub fn id(&self) -> NodeId {
        Runtime::instance().settings.node.id
    }

    #[inline]
    pub async fn name(&self, id: NodeId) -> String {
        Runtime::instance().extends.shared().await.node_name(id)
    }

    #[inline]
    pub async fn new_grpc_client(&self, remote_addr: &str) -> Result<NodeGrpcClient> {
        NodeGrpcClient::new(remote_addr).await
    }

    pub fn start_grpc_server(&self) {
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(Runtime::instance().settings.rpc.server_workers)
                .thread_name("grpc-server-worker")
                .thread_stack_size(4 * 1024 * 1024)
                .build()
                .expect("tokio runtime build failed");
            let runner = async {
                if let Err(e) = Server::new().listen_and_serve().await {
                    log::error!(
                        "listen and serve failure, {:?}, laddr: {:?}",
                        e,
                        Runtime::instance().settings.rpc.server_addr
                    );
                }
            };
            rt.block_on(runner)
        });
    }

    #[inline]
    pub async fn status(&self) -> NodeStatus {
        NodeStatus::Running(1)
    }

    #[inline]
    fn uptime(&self) -> String {
        to_uptime((chrono::Local::now() - self.start_time).num_seconds())
    }

    #[inline]
    pub async fn broker_info(&self) -> BrokerInfo {
        let node_id = self.id();
        BrokerInfo {
            version: version::VERSION.to_string(),
            uptime: self.uptime(),
            sysdescr: "RMQTT Broker".into(),
            node_status: self.status().await,
            node_id,
            node_name: Runtime::instance().extends.shared().await.node_name(node_id),
            datetime: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        }
    }

    #[inline]
    pub async fn node_info(&self) -> NodeInfo {
        let node_id = self.id();

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
            connections: Runtime::instance().stats.connections.count(),
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
            node_status: self.status().await,
            node_id,
            node_name: Runtime::instance().extends.shared().await.node_name(node_id),
            uptime: self.uptime(),
            version: version::VERSION.to_string(),
        }
    }

    #[inline]
    fn _is_busy(&self) -> bool {
        let sys = systemstat::System::new();
        let cpuload = self.cpuload();

        let loadavg = sys.load_average();
        let load1 = loadavg.as_ref().map(|l| l.one).unwrap_or_default();

        load1 > Runtime::instance().settings.node.busy.loadavg
            || cpuload > Runtime::instance().settings.node.busy.cpuloadavg
    }

    #[inline]
    pub fn sys_is_busy(&self) -> bool {
        static CACHED: Lazy<RwLock<(bool, Instant)>> = Lazy::new(|| RwLock::new((false, Instant::now())));
        {
            let cached = CACHED.read();
            let (busy, inst) = cached.deref();
            if inst.elapsed() < Runtime::instance().settings.node.busy.update_interval {
                return *busy;
            }
        }
        let busy = self._is_busy();
        *CACHED.write() = (busy, Instant::now());
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
            "uptime": self.uptime,
            "sysdescr": self.sysdescr,
            "running": self.node_status.running(),
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
            "running":  self.node_status.running(),
            "node_id":  self.node_id,
            "node_name":  self.node_name,
            "uptime":  self.uptime,
            "version":  self.version
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
