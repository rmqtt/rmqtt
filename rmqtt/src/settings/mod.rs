use std::fmt;
use std::net::SocketAddr;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::time::Duration;

use chrono::TimeZone;
use config::{Config, ConfigError, File};
use parking_lot::RwLock;
use serde::de::{Deserialize, Deserializer};
use serde::ser::Serializer;
use serde::Serialize;

use crate::{NodeId, Result};

use self::listener::Listeners;
use self::log::Log;

pub mod listener;
pub mod log;

#[derive(Clone)]
pub struct Settings(Arc<Inner>);

#[derive(Debug, Clone, Deserialize)]
pub struct Inner {
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
}

impl Deref for Settings {
    type Target = Inner;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

impl Settings {
    pub fn new() -> Result<Self, ConfigError> {
        let mut s = Config::new();

        if let Ok(cfg_filename) = std::env::var("RMQTT-CONFIG-FILENAME") {
            s.merge(File::with_name(&cfg_filename).required(false))?;
        }
        s.merge(File::with_name("/etc/rmqtt/rmqtt").required(false))?;
        s.merge(File::with_name("/etc/rmqtt").required(false))?;
        s.merge(File::with_name("rmqtt").required(false))?;

        let mut inner: Inner = match s.try_into() {
            Ok(c) => c,
            Err(e) => {
                return Err(e);
            }
        };

        inner.listeners.init();
        if inner.listeners.tcps.is_empty() && inner.listeners.tlss.is_empty() {
            return Err(ConfigError::Message(
                "Settings::new() error, listener.tcp or listener.tls is not exist".into(),
            ));
        }

        Ok(Self(Arc::new(inner)))
    }
}

impl fmt::Debug for Settings {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Settings ...")?;
        Ok(())
    }
}

#[derive(Default, Debug, Clone, Deserialize)]
pub struct Node {
    #[serde(default)]
    pub id: NodeId,
    #[serde(default = "Node::cookie_default")]
    pub cookie: String,
    #[serde(default = "Node::crash_dump_default")]
    pub crash_dump: String,
}

impl Node {
    fn cookie_default() -> String {
        "rmqttsecretcookie".into()
    }
    fn crash_dump_default() -> String {
        "/var/log/rmqtt/crash.dump".into()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rpc {
    #[serde(default = "Rpc::mode_default")]
    pub mode: String, // = "async"

    #[serde(default = "Rpc::server_addr_default", deserialize_with = "deserialize_addr")]
    pub server_addr: SocketAddr, // = "0.0.0.0:5363"

    #[serde(default = "Rpc::server_workers_default")]
    pub server_workers: usize, //4

    #[serde(default = "Rpc::client_concurrency_limit_default")]
    pub client_concurrency_limit: usize, // = 128

    #[serde(default = "Rpc::client_timeout_default", deserialize_with = "deserialize_duration")]
    pub client_timeout: Duration,
    //= "5s"
    //#Maximum number of messages sent in batch
    #[serde(default = "Rpc::batch_size_default")]
    pub batch_size: usize, // = 128
}

impl Default for Rpc {
    #[inline]
    fn default() -> Self {
        Self {
            mode: Self::mode_default(),
            batch_size: Self::batch_size_default(),
            server_addr: Self::server_addr_default(),
            server_workers: Self::server_workers_default(),
            client_concurrency_limit: Self::client_concurrency_limit_default(),
            client_timeout: Self::client_timeout_default(),
        }
    }
}

impl Rpc {
    fn mode_default() -> String {
        "async".into()
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

    pub fn load_config<'de, T: serde::Deserialize<'de>>(&self, name: &str) -> Result<T, ConfigError> {
        let dir = self.dir.trim_end_matches(|c| c == '/' || c == '\\');
        let mut s = Config::new();
        s.merge(File::with_name(&format!("{}/{}", dir, name)).required(true))?;
        s.try_into::<T>()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Mqtt {}

#[derive(Debug, Clone)]
pub struct ValueMut<T>(Arc<RwLock<T>>);

impl<T> ValueMut<T>
    where
        T: Copy,
{
    #[inline]
    pub fn new(v: T) -> Self {
        Self(Arc::new(RwLock::new(v)))
    }

    #[inline]
    pub fn get(&self) -> T {
        *self.0.read()
    }

    #[inline]
    pub fn set(&self, v: T) {
        *self.0.write() = v;
    }
}

impl<'de, T: serde::Deserialize<'de> + Copy> Deserialize<'de> for ValueMut<T> {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
    {
        let v = T::deserialize(deserializer)?;
        Ok(ValueMut::new(v))
    }
}

#[derive(Debug, Clone)]
pub struct Bytesize(usize);

impl Bytesize {
    #[inline]
    pub fn as_u32(&self) -> u32 {
        self.0 as u32
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
    text.split_inclusive(|x| x == 'G' || x == 'M' || x == 'K' || x == 'B')
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
                'K' => v * 1024,
                'M' => v * 1048576,
                'G' => v * 1073741824,
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
        .split_inclusive(|x| x == 's' || x == 'm' || x == 'h' || x == 'd' || x == 'w' || x == 'f' || x == 'Y')
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
        let t = if let Ok(d) = chrono::Local.datetime_from_str(&t_str, "%Y-%m-%d %H:%M:%S") {
            Duration::from_secs(d.timestamp() as u64)
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
