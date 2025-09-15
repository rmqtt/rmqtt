#![deny(unsafe_code)]

use std::fmt;
use std::net::SocketAddr;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use config::{Config, File, Source};
use once_cell::sync::OnceCell;
use serde::Deserialize;

use rmqtt_net::Result;
use rmqtt_utils::*;

use self::listener::Listeners;
use self::logging::Log;

pub use self::listener::Listener;
pub use self::options::Options;

pub mod listener;
pub mod logging;
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
            inner.node.id = id;
        }
        if let Some(plugins_default_startups) = opts.plugins_default_startups.as_ref() {
            inner.plugins.default_startups.clone_from(plugins_default_startups)
        }

        inner.opts = opts;
        Ok(Self(Arc::new(inner)))
    }

    #[inline]
    pub fn instance() -> &'static Self {
        match SETTINGS.get() {
            Some(c) => c,
            None => {
                unreachable!("Settings not initialized");
            }
        }
    }

    #[inline]
    pub fn init(opts: Options) -> Result<&'static Self> {
        SETTINGS.set(Settings::new(opts)?).map_err(|_| anyhow!("Settings init failed"))?;
        SETTINGS.get().ok_or_else(|| anyhow!("Settings init failed"))
    }

    #[inline]
    pub fn logs() -> Result<()> {
        let cfg = Self::instance();
        log::debug!("Config info is {:?}", cfg.0);
        log::info!("node_id is {}", cfg.node.id);
        log::info!("exec_workers is {}", cfg.task.exec_workers);
        log::info!("exec_queue_max is {}", cfg.task.exec_queue_max);
        log::info!("node.busy config is: {:?}", cfg.node.busy);
        log::info!("node.rpc config is: {:?}", cfg.rpc);

        if cfg.opts.node_grpc_addrs.is_some() {
            log::info!("node_grpc_addrs is {:?}", cfg.opts.node_grpc_addrs);
        }
        if cfg.opts.raft_peer_addrs.is_some() {
            log::info!("raft_peer_addrs is {:?}", cfg.opts.raft_peer_addrs);
        }
        if cfg.opts.raft_leader_id.is_some() {
            log::info!("raft_leader_id is {:?}", cfg.opts.raft_leader_id);
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
}

impl Default for Task {
    #[inline]
    fn default() -> Self {
        Self { exec_workers: Self::exec_workers_default(), exec_queue_max: Self::exec_queue_max_default() }
    }
}

impl Task {
    fn exec_workers_default() -> usize {
        1000
    }
    fn exec_queue_max_default() -> usize {
        300_000
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
    pub handshaking: isize, //0
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
}

impl Default for Rpc {
    #[inline]
    fn default() -> Self {
        Self {
            reuseaddr: Self::reuseaddr_default(),
            reuseport: Self::reuseport_default(),
            server_addr: Self::server_addr_default(),
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
    fn server_addr_default() -> SocketAddr {
        ([0, 0, 0, 0], 5363).into()
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
            log::warn!("The configuration for plugin '{name}' does not exist, default values will be used!");
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
            log::warn!("The configuration for plugin '{name}' does not exist, default values will be used!");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_id_values() {
        let test_cases = [
            (Some(0), 0),
            (Some(1), 1),
            (Some(2), 2),
            (Some(1000), 1000),
            (None, 0), // Default when None
        ];

        for (node_id, expected) in &test_cases {
            let opts = Options { node_id: *node_id, ..Default::default() };

            let settings = Settings::new(opts).expect("Settings creation failed");
            assert_eq!(settings.node.id, *expected, "Expected node ID {}", expected);
        }
    }

    #[test]
    fn test_raft_leader_id_integration() {
        // Ensure node ID and raft leader ID can both use 0 without conflicts
        let test_cases = [
            (Some(0), None, 0, None),
            (Some(0), Some(0), 0, Some(0)),
            (Some(1), Some(0), 1, Some(0)),
            (Some(0), Some(1), 0, Some(1)),
        ];

        for (node_id, raft_leader_id, expected_node, expected_leader) in &test_cases {
            let opts = Options { node_id: *node_id, raft_leader_id: *raft_leader_id, ..Default::default() };

            let settings = Settings::new(opts).expect("Settings creation failed");
            assert_eq!(settings.node.id, *expected_node, "Expected node ID {}", expected_node);
            assert_eq!(
                settings.opts.raft_leader_id, *expected_leader,
                "Expected raft leader {:?}",
                expected_leader
            );
        }
    }
}
