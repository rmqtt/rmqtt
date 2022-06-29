use std::net::SocketAddr;
use systemstat::Platform;

use crate::{NodeId, Result, Runtime};
use crate::grpc::client::NodeGrpcClient;
use crate::grpc::server::Server;

#[allow(dead_code)]
mod version {
    include!(concat!(env!("OUT_DIR"), "/version.rs"));
}

pub struct Node {
    pub start_time: chrono::DateTime<chrono::Local>,
}

impl Node {
    pub(crate) fn new() -> Self {
        Self {
            start_time: chrono::Local::now(),
        }
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
    pub async fn new_grpc_client(&self, remote_addr: &SocketAddr) -> Result<NodeGrpcClient> {
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
                .unwrap();
            let runner = async {
                if let Err(e) = Server::new().listen_and_serve().await {
                    log::error!("listen and serve failure, {:?}", e);
                }
            };
            rt.block_on(runner)
        });
    }

    #[inline]
    pub async fn status(&self) -> NodeStatus{
        NodeStatus::Running
    }

    #[inline]
    fn uptime(&self) -> String{
        to_uptime((chrono::Local::now() - self.start_time).num_seconds())
    }

    #[inline]
    pub async fn broker_info(&self) -> BrokerInfo {
        let node_id = self.id();
        BrokerInfo{
            version: version::VERSION.to_string(),
            uptime: self.uptime(),
            sysdescr: "RMQTT Broker".into(),
            node_status: self.status().await,
            node_id,
            node_name: format!("{}@{}", node_id, "127.0.0.1"),
            datetime: self.start_time.format("%Y-%m-%d %H:%M:%S").to_string()
        }
    }

    #[inline]
    pub async fn node_info(&self) -> NodeInfo {
        let node_id = self.id();

        let sys = systemstat::System::new();
        let boottime = sys.boot_time().map(|t|t.to_string()).unwrap_or_default();
        let loadavg = sys.load_average();
        let mem_info = sys.memory();

        let (disk_total, disk_free) = if let Ok(mounts) = sys.mounts(){
            let total = mounts.iter().map(|m| m.total.as_u64()).sum();
            let free = mounts.iter().map(|m| m.free.as_u64()).sum();
            (total, free)
        }else{
            (0, 0)
        };

        NodeInfo{
            connections: Runtime::instance().extends.shared().await.clients().await,
            boottime,
            load1: loadavg.as_ref().map(|l|l.one).unwrap_or_default(),
            load5: loadavg.as_ref().map(|l|l.five).unwrap_or_default(),
            load15: loadavg.as_ref().map(|l|l.fifteen).unwrap_or_default(),
            memory_total: mem_info.as_ref().map(|m| m.total.as_u64()).unwrap_or_default(),
            memory_free: mem_info.as_ref().map(|m| m.free.as_u64()).unwrap_or_default(),
            memory_used: mem_info.as_ref().map(|m| systemstat::saturating_sub_bytes(m.total, m.free).as_u64()).unwrap_or_default(),
            disk_total,
            disk_free,
            node_status: self.status().await,
            node_id,
            node_name: format!("{}@{}", node_id, "127.0.0.1"),
            uptime: self.uptime(),
            version: version::VERSION.to_string(),
            ..Default::default()
        }
    }

}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BrokerInfo{
    pub version: String,
    pub uptime: String,
    pub sysdescr: String,
    pub node_status: NodeStatus,
    pub node_id: NodeId,
    pub node_name: String,
    pub datetime: String,
}

impl BrokerInfo {
    pub fn to_json(&self) -> serde_json::Value{
        json!({
            "version": self.version,
            "uptime": self.uptime,
            "sysdescr": self.sysdescr,
            "node_status": self.node_status,
            "node_id": self.node_id,
            "node_name": self.node_name,
            "datetime": self.datetime
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct NodeInfo{
    pub connections: usize,
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
    pub fn to_json(&self) -> serde_json::Value{
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
            "node_status":  self.node_status,
            "node_id":  self.node_id,
            "node_name":  self.node_name,
            "uptime":  self.uptime,
            "version":  self.version
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum NodeStatus{
    Running,
    Stop,
    Error(String)
}

impl Default for NodeStatus{
    fn default() -> Self{
        NodeStatus::Running
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
    format!(
        "{} days {} hours, {} minutes, {} seconds",
        uptime_days, uptime_hours, uptime_minus, uptime_secs
    )
}