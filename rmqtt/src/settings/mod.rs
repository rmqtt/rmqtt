use anyhow::anyhow;
use std::fmt;
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::ops::{Deref, DerefMut};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use chrono::LocalResult;
use config::{Config, File, Source};
use once_cell::sync::OnceCell;
use serde::de::{self, Deserialize, Deserializer};
use serde::ser::Serializer;
use serde::Serialize;

use crate::{Addr, MqttError, NodeId, Result};

pub use self::listener::Listener;
use self::listener::Listeners;
use self::log::Log;
pub use self::options::Options;

pub mod acl;
pub mod listener;
pub mod log;
pub mod options;

static SETTINGS: OnceCell<Settings> = OnceCell::new();

#[derive(Clone)]
pub struct Settings(Arc<Inner>);

#[derive(Debug, Clone, Deserialize)]
pub struct Inner {
    #[serde(default)]
    pub task: Task,
    #[serde(default)]
    pub node: Node,
    #[serde(default)]
    pub rpc: Rpc,
    #[serde(default)]
    pub log: Log,
    #[serde(rename = "listener")]
    #[serde(default)]
    pub listeners: Listeners,
    #[serde(default)]
    pub plugins: Plugins,
    #[serde(default)]
    pub mqtt: Mqtt,
    #[serde(default, skip)]
    pub opts: Options,
}

impl Deref for Settings {
    type Target = Inner;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

impl Settings {
    fn new(opts: Options) -> Result<Self> {
        let mut builder = Config::builder()
            .add_source(File::with_name("/etc/rmqtt/rmqtt").required(false))
            .add_source(File::with_name("/etc/rmqtt").required(false))
            .add_source(File::with_name("rmqtt").required(false))
            .add_source(
                config::Environment::with_prefix("rmqtt")
                    .try_parsing(true)
                    .list_separator(" ")
                    .with_list_parse_key("plugins.default_startups"),
            );

        if let Some(cfg) = opts.cfg_name.as_ref() {
            builder = builder.add_source(File::with_name(cfg).required(false));
        }

        let mut inner: Inner = builder.build()?.try_deserialize()?;

        inner.listeners.init();
        if inner.listeners.tcps.is_empty() && inner.listeners.tlss.is_empty() {
            //set default
            inner.listeners.set_default();
        }

        //Command line configuration overriding file configuration
        if let Some(id) = opts.node_id {
            if id > 0 {
                inner.node.id = id;
            }
        }
        if let Some(plugins_default_startups) = opts.plugins_default_startups.as_ref() {
            inner.plugins.default_startups.clone_from(plugins_default_startups)
        }

        inner.opts = opts;
        Ok(Self(Arc::new(inner)))
    }

    #[inline]
    pub fn instance() -> Result<&'static Self> {
        Ok(SETTINGS.get().ok_or_else(|| anyhow!("Settings not initialized"))?)
    }

    #[inline]
    pub fn init(opts: Options) -> Result<&'static Self> {
        SETTINGS.set(Settings::new(opts)?).map_err(|_| anyhow!("Settings init failed"))?;
        Ok(SETTINGS.get().ok_or_else(|| anyhow!("Settings init failed"))?)
    }

    #[inline]
    pub fn logs() -> Result<()> {
        let cfg = Self::instance()?;
        crate::log::debug!("Config info is {:?}", cfg.0);
        crate::log::info!("node_id is {}", cfg.node.id);
        crate::log::info!("exec_workers is {}", cfg.task.exec_workers);
        crate::log::info!("exec_queue_max is {}", cfg.task.exec_queue_max);
        crate::log::info!("local_exec_workers is {}", cfg.task.local_exec_workers);
        crate::log::info!("local_exec_queue_max is {}", cfg.task.local_exec_queue_max);
        crate::log::info!("local_exec_rate_limit is {:?}", cfg.task.local_exec_rate_limit);
        crate::log::info!("node.busy config is: {:?}", cfg.node.busy);

        if cfg.opts.node_grpc_addrs.is_some() {
            crate::log::info!("node_grpc_addrs is {:?}", cfg.opts.node_grpc_addrs);
        }
        if cfg.opts.raft_peer_addrs.is_some() {
            crate::log::info!("raft_peer_addrs is {:?}", cfg.opts.raft_peer_addrs);
        }
        if cfg.opts.raft_leader_id.is_some() {
            crate::log::info!("raft_leader_id is {:?}", cfg.opts.raft_leader_id);
        }
        Ok(())
    }
}

impl fmt::Debug for Settings {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Settings ...")?;
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Task {
    //Concurrent task count for global task executor.
    #[serde(default = "Task::exec_workers_default")]
    pub exec_workers: usize,

    //Queue capacity for global task executor.
    #[serde(default = "Task::exec_queue_max_default")]
    pub exec_queue_max: usize,

    //Concurrent task count for global local task executor, per worker thread.
    #[serde(default = "Task::local_exec_workers_default")]
    pub local_exec_workers: usize,

    //Queue capacity for global local task executor, per worker thread.
    #[serde(default = "Task::local_exec_queue_max_default")]
    pub local_exec_queue_max: usize,

    //The rate at which messages are dequeued from the 'LocalTaskExecQueue' message queue.
    #[serde(
        default = "Task::local_exec_rate_limit_default",
        deserialize_with = "Task::deserialize_local_exec_rate_limit"
    )]
    pub local_exec_rate_limit: (NonZeroU32, Duration),
}

impl Default for Task {
    #[inline]
    fn default() -> Self {
        Self {
            exec_workers: Self::exec_workers_default(),
            exec_queue_max: Self::exec_queue_max_default(),
            local_exec_workers: Self::local_exec_workers_default(),
            local_exec_queue_max: Self::local_exec_queue_max_default(),
            local_exec_rate_limit: Self::local_exec_rate_limit_default(),
        }
    }
}

impl Task {
    fn exec_workers_default() -> usize {
        1000
    }
    fn exec_queue_max_default() -> usize {
        300_000
    }
    fn local_exec_workers_default() -> usize {
        50
    }
    fn local_exec_queue_max_default() -> usize {
        10_000
    }
    fn local_exec_rate_limit_default() -> (NonZeroU32, Duration) {
        (NonZeroU32::MAX, Duration::from_secs(1))
    }

    #[inline]
    fn deserialize_local_exec_rate_limit<'de, D>(deserializer: D) -> Result<(NonZeroU32, Duration), D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = String::deserialize(deserializer)?;
        let pair: Vec<&str> = v.split(',').collect();
        if pair.len() == 2 {
            let burst = NonZeroU32::from_str(pair[0]).map_err(|e| {
                de::Error::custom(format!("local_exec_rate_limit, burst format error, {e:?}"))
            })?;
            let replenish_n_per = to_duration(pair[1]);
            if replenish_n_per.as_millis() == 0 {
                return Err(de::Error::custom(format!(
                    "local_exec_rate_limit, value format error, {}",
                    pair.join(",")
                )));
            }
            Ok((burst, replenish_n_per))
        } else {
            Err(de::Error::custom(format!("local_exec_rate_limit, value format error, {}", pair.join(","))))
        }
    }
}

#[derive(Default, Debug, Clone, Deserialize)]
pub struct Node {
    #[serde(default)]
    pub id: NodeId,
    #[serde(default = "Node::cookie_default")]
    pub cookie: String,
    // #[serde(default = "Node::crash_dump_default")]
    // pub crash_dump: String,
    #[serde(default)]
    pub busy: Busy,
}

impl Node {
    fn cookie_default() -> String {
        "rmqttsecretcookie".into()
    }
    // fn crash_dump_default() -> String {
    //     "/var/log/rmqtt/crash.dump".into()
    // }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Busy {
    //Busy status check switch
    #[serde(default = "Busy::check_enable_default")]
    pub check_enable: bool,
    //Busy status update interval
    #[serde(default = "Busy::update_interval_default", deserialize_with = "deserialize_duration")]
    pub update_interval: Duration,
    //The threshold for the 1-minute average system load used to determine system busyness.
    #[serde(default = "Busy::loadavg_default")]
    pub loadavg: f32, //70.0
    //The threshold for average CPU load used to determine system busyness.
    #[serde(default = "Busy::cpuloadavg_default")]
    pub cpuloadavg: f32, //80.0
    //The threshold for determining high-concurrency connection handshakes in progress.
    #[serde(default)]
    pub handshaking: usize, //0
}

impl Default for Busy {
    #[inline]
    fn default() -> Self {
        Self {
            check_enable: Self::check_enable_default(),
            update_interval: Self::update_interval_default(),
            loadavg: Self::loadavg_default(),
            cpuloadavg: Self::cpuloadavg_default(),
            handshaking: 0,
        }
    }
}

impl Busy {
    fn check_enable_default() -> bool {
        true
    }
    fn update_interval_default() -> Duration {
        Duration::from_secs(2)
    }
    fn loadavg_default() -> f32 {
        80.0
    }

    fn cpuloadavg_default() -> f32 {
        90.0
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rpc {
    #[serde(default = "Rpc::server_addr_default", deserialize_with = "deserialize_addr")]
    pub server_addr: SocketAddr,

    #[serde(default = "Rpc::reuseaddr_default")]
    pub reuseaddr: bool,

    #[serde(default = "Rpc::reuseport_default")]
    pub reuseport: bool,

    #[serde(default = "Rpc::server_workers_default")]
    pub server_workers: usize,

    #[serde(default = "Rpc::client_concurrency_limit_default")]
    pub client_concurrency_limit: usize,

    #[serde(default = "Rpc::client_timeout_default", deserialize_with = "deserialize_duration")]
    pub client_timeout: Duration,

    //#Maximum number of messages sent in batch
    #[serde(default = "Rpc::batch_size_default")]
    pub batch_size: usize,
}

impl Default for Rpc {
    #[inline]
    fn default() -> Self {
        Self {
            reuseaddr: Self::reuseaddr_default(),
            reuseport: Self::reuseport_default(),
            batch_size: Self::batch_size_default(),
            server_addr: Self::server_addr_default(),
            server_workers: Self::server_workers_default(),
            client_concurrency_limit: Self::client_concurrency_limit_default(),
            client_timeout: Self::client_timeout_default(),
        }
    }
}

impl Rpc {
    fn reuseaddr_default() -> bool {
        true
    }
    fn reuseport_default() -> bool {
        false
    }
    fn batch_size_default() -> usize {
        128
    }
    fn server_addr_default() -> SocketAddr {
        ([0, 0, 0, 0], 5363).into()
    }
    fn server_workers_default() -> usize {
        4
    }
    fn client_concurrency_limit_default() -> usize {
        128
    }
    fn client_timeout_default() -> Duration {
        Duration::from_secs(5)
    }
}

#[derive(Default, Debug, Clone, Deserialize)]
pub struct Plugins {
    #[serde(default = "Plugins::dir_default")]
    pub dir: String,
    #[serde(default)]
    pub default_startups: Vec<String>,
}

impl Plugins {
    fn dir_default() -> String {
        "./plugins/".into()
    }

    pub fn load_config<'de, T: serde::Deserialize<'de>>(&self, name: &str) -> Result<T> {
        let (cfg, _) = self.load_config_with_required(name, true, &[])?;
        Ok(cfg)
    }

    pub fn load_config_default<'de, T: serde::Deserialize<'de>>(&self, name: &str) -> Result<T> {
        let (cfg, def) = self.load_config_with_required(name, false, &[])?;
        if def {
            crate::log::warn!(
                "The configuration for plugin '{name}' does not exist, default values will be used!"
            );
        }
        Ok(cfg)
    }

    pub fn load_config_with<'de, T: serde::Deserialize<'de>>(
        &self,
        name: &str,
        env_list_keys: &[&str],
    ) -> Result<T> {
        let (cfg, _) = self.load_config_with_required(name, true, env_list_keys)?;
        Ok(cfg)
    }

    pub fn load_config_default_with<'de, T: serde::Deserialize<'de>>(
        &self,
        name: &str,
        env_list_keys: &[&str],
    ) -> Result<T> {
        let (cfg, def) = self.load_config_with_required(name, false, env_list_keys)?;
        if def {
            crate::log::warn!(
                "The configuration for plugin '{name}' does not exist, default values will be used!"
            );
        }
        Ok(cfg)
    }

    fn load_config_with_required<'de, T: serde::Deserialize<'de>>(
        &self,
        name: &str,
        required: bool,
        env_list_keys: &[&str],
    ) -> Result<(T, bool)> {
        let dir = self.dir.trim_end_matches(['/', '\\']);
        let mut builder =
            Config::builder().add_source(File::with_name(&format!("{dir}/{name}")).required(required));

        let mut env = config::Environment::with_prefix(&format!("rmqtt_plugin_{}", name.replace('-', "_")));
        if !env_list_keys.is_empty() {
            env = env.try_parsing(true).list_separator(" ");
            for key in env_list_keys {
                env = env.with_list_parse_key(key);
            }
        }
        builder = builder.add_source(env);

        let s = builder.build()?;
        let count = s.collect()?.len();
        Ok((s.try_deserialize::<T>()?, count == 0))
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Mqtt {
    #[serde(default = "Mqtt::delayed_publish_max_default")]
    pub delayed_publish_max: usize,
    #[serde(default = "Mqtt::delayed_publish_immediate_default")]
    pub delayed_publish_immediate: bool,
    #[serde(default = "Mqtt::max_sessions_default")]
    pub max_sessions: isize,
}

impl Mqtt {
    fn delayed_publish_max_default() -> usize {
        100_000
    }

    fn delayed_publish_immediate_default() -> bool {
        true
    }

    fn max_sessions_default() -> isize {
        0
    }
}

const BYTESIZE_K: usize = 1024;
const BYTESIZE_M: usize = 1048576;
const BYTESIZE_G: usize = 1073741824;

#[derive(Clone, Copy, Default)]
pub struct Bytesize(usize);

impl Bytesize {
    #[inline]
    pub fn as_u32(&self) -> u32 {
        self.0 as u32
    }

    #[inline]
    pub fn as_u64(&self) -> u64 {
        self.0 as u64
    }

    #[inline]
    pub fn as_usize(&self) -> usize {
        self.0
    }

    #[inline]
    pub fn string(&self) -> String {
        let mut v = self.0;
        let mut res = String::new();

        let g = v / BYTESIZE_G;
        if g > 0 {
            res.push_str(&format!("{g}G"));
            v %= BYTESIZE_G;
        }

        let m = v / BYTESIZE_M;
        if m > 0 {
            res.push_str(&format!("{m}M"));
            v %= BYTESIZE_M;
        }

        let k = v / BYTESIZE_K;
        if k > 0 {
            res.push_str(&format!("{k}K"));
            v %= BYTESIZE_K;
        }

        if v > 0 {
            res.push_str(&format!("{v}B"));
        }

        res
    }
}

impl Deref for Bytesize {
    type Target = usize;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Bytesize {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl From<usize> for Bytesize {
    fn from(v: usize) -> Self {
        Bytesize(v)
    }
}

impl From<&str> for Bytesize {
    fn from(v: &str) -> Self {
        Bytesize(to_bytesize(v))
    }
}

impl fmt::Debug for Bytesize {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.string())?;
        Ok(())
    }
}

impl Serialize for Bytesize {
    #[inline]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Bytesize {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = to_bytesize(&String::deserialize(deserializer)?);
        Ok(Bytesize(v))
    }
}

#[inline]
pub fn to_bytesize(text: &str) -> usize {
    let text = text.to_uppercase().replace("GB", "G").replace("MB", "M").replace("KB", "K");
    text.split_inclusive(['G', 'M', 'K', 'B'])
        .map(|x| {
            let mut chars = x.chars();
            let u = match chars.nth_back(0) {
                None => return 0,
                Some(u) => u,
            };
            let v = match chars.as_str().parse::<usize>() {
                Err(_e) => return 0,
                Ok(v) => v,
            };
            match u {
                'B' => v,
                'K' => v * BYTESIZE_K,
                'M' => v * BYTESIZE_M,
                'G' => v * BYTESIZE_G,
                _ => 0,
            }
        })
        .sum()
}

#[inline]
pub fn deserialize_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let v = String::deserialize(deserializer)?;
    Ok(to_duration(&v))
}

#[inline]
pub fn deserialize_duration_option<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    let v = String::deserialize(deserializer)?;
    if v.is_empty() {
        Ok(None)
    } else {
        Ok(Some(to_duration(&v)))
    }
}

#[inline]
pub fn to_duration(text: &str) -> Duration {
    let text = text.to_lowercase().replace("ms", "Y");
    let ms: u64 = text
        .split_inclusive(['s', 'm', 'h', 'd', 'w', 'f', 'Y'])
        .map(|x| {
            let mut chars = x.chars();
            let u = match chars.nth_back(0) {
                None => return 0,
                Some(u) => u,
            };
            let v = match chars.as_str().parse::<u64>() {
                Err(_e) => return 0,
                Ok(v) => v,
            };
            match u {
                'Y' => v,
                's' => v * 1000,
                'm' => v * 60000,
                'h' => v * 3600000,
                'd' => v * 86400000,
                'w' => v * 604800000,
                'f' => v * 1209600000,
                _ => 0,
            }
        })
        .sum();
    Duration::from_millis(ms)
}

#[inline]
pub fn deserialize_addr<'de, D>(deserializer: D) -> Result<SocketAddr, D::Error>
where
    D: Deserializer<'de>,
{
    let addr = String::deserialize(deserializer)?
        .parse::<std::net::SocketAddr>()
        .map_err(serde::de::Error::custom)?;
    Ok(addr)
}

#[inline]
pub fn deserialize_addr_option<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<std::net::SocketAddr>, D::Error>
where
    D: Deserializer<'de>,
{
    let addr = String::deserialize(deserializer).map(|mut addr| {
        if !addr.contains(':') {
            addr += ":0";
        }
        addr
    })?;
    let addr = addr.parse::<std::net::SocketAddr>().map_err(serde::de::Error::custom)?;
    Ok(Some(addr))
}

#[inline]
pub fn deserialize_datetime_option<'de, D>(deserializer: D) -> std::result::Result<Option<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    let t_str = String::deserialize(deserializer)?;
    if t_str.is_empty() {
        Ok(None)
    } else {
        let t = if let Ok(d) = timestamp_parse_from_str(&t_str, "%Y-%m-%d %H:%M:%S") {
            Duration::from_secs(d as u64)
        } else {
            let d = t_str.parse::<u64>().map_err(serde::de::Error::custom)?;
            Duration::from_secs(d)
        };
        Ok(Some(t))
    }
}

#[inline]
pub fn serialize_datetime_option<S>(t: &Option<Duration>, s: S) -> std::result::Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if let Some(t) = t {
        t.as_secs().to_string().serialize(s)
    } else {
        "".serialize(s)
    }
}

#[derive(Clone, Serialize)]
pub struct NodeAddr {
    pub id: NodeId,
    pub addr: Addr,
}

impl std::fmt::Debug for NodeAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{:?}", self.id, self.addr)
    }
}

impl FromStr for NodeAddr {
    type Err = MqttError;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('@').collect();
        if parts.len() < 2 {
            return Err(MqttError::Msg(format!("NodeAddr format error, {s}")));
        }
        let id = NodeId::from_str(parts[0]).map_err(MqttError::ParseIntError)?;
        //let addr = parts[1].parse().map_err(|e|MqttError::AddrParseError(e))?;
        let addr = Addr::from(parts[1]);
        Ok(NodeAddr { id, addr })
    }
}

impl<'de> de::Deserialize<'de> for NodeAddr {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        NodeAddr::from_str(&String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

#[inline]
fn timestamp_parse_from_str(ts: &str, fmt: &str) -> anyhow::Result<i64> {
    let ndt = chrono::NaiveDateTime::parse_from_str(ts, fmt)?;
    let ndt = ndt.and_local_timezone(*chrono::Local::now().offset());
    match ndt {
        LocalResult::None => Err(anyhow::Error::msg("Impossible")),
        LocalResult::Single(d) => Ok(d.timestamp()),
        LocalResult::Ambiguous(d, _tz) => Ok(d.timestamp()),
    }
}
