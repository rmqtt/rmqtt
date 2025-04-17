//! Utilities module providing essential types and functions for common system operations
//!
//! ## Core Features:
//! - **Byte Size Handling**: Human-readable byte size parsing/formatting with [`Bytesize`]
//! - **Duration Conversion**: String-to-Duration parsing supporting multiple time units
//! - **Timestamp Utilities**: Precise timestamp handling with millisecond resolution
//! - **Network Addressing**: Cluster node address parsing ([`NodeAddr`]) and socket address handling
//! - **Counter Implementation**: Thread-safe counter with merge modes ([`Counter`])
//!
//! ## Key Components:
//! - `Bytesize`: Handles 2G512M-style conversions with serialization support
//! - Time functions: `timestamp_secs()`, `format_timestamp_now()`, and datetime parsing
//! - `NodeAddr`: Cluster node representation (ID@Address) with parser
//! - Network address utilities with proper error handling
//! - Custom serde helpers for duration and address types
//!
//! ## Usage Examples:
//! ```rust
//! use rmqtt_utils::{Bytesize, NodeAddr, to_bytesize, to_duration, format_timestamp_now};
//!
//! // Byte size parsing
//! let size = Bytesize::from("2G512M");
//! assert_eq!(size.as_usize(), 2_684_354_560);
//!
//! // Duration conversion
//! let duration = to_duration("1h30m15s");
//! assert_eq!(duration.as_secs(), 5415);
//!
//! // Node address parsing
//! let node: NodeAddr = "1@mqtt-node:1883".parse().unwrap();
//! assert_eq!(node.id, 1);
//!
//! // Timestamp formatting
//! let now = format_timestamp_now();
//! assert!(now.contains("2025")); // Current year
//! ```
//!
//! ## Safety Guarantees:
//! - Zero `unsafe` code usage (enforced by `#![deny(unsafe_code)]`)
//! - Comprehensive error handling for parsing operations
//! - Platform-agnostic network address handling
//! - Chrono-based timestamp calculations with proper timezone handling
//!
//! Overall usage example:
//!
//! ```
//! use rmqtt_utils::{
//!     Bytesize, NodeAddr,
//!     to_bytesize, to_duration,
//!     timestamp_secs, format_timestamp_now
//! };
//!
//! // Parse byte size from string
//! let size = Bytesize::from("2G512M");
//!
//! // Convert duration string
//! let duration = to_duration("1h30m15s");
//!
//! // Parse node address
//! let node: NodeAddr = "123@127.0.0.1:1883".parse().unwrap();
//!
//! // Get formatted timestamp
//! let now = format_timestamp_now();
//! ```

#![deny(unsafe_code)]

use std::fmt;
use std::net::SocketAddr;
use std::ops::{Deref, DerefMut};
use std::str::FromStr;
use std::time::Duration;

use anyhow::{anyhow, Error};
use bytestring::ByteString;
use chrono::LocalResult;
use serde::{
    de::{self, Deserializer},
    ser::Serializer,
    Deserialize, Serialize,
};

mod counter;

pub use counter::{Counter, StatsMergeMode};

/// Cluster node identifier type (64-bit unsigned integer)
pub type NodeId = u64;

/// Network address storage using efficient ByteString
pub type Addr = ByteString;

/// Timestamp representation in seconds since Unix epoch
pub type Timestamp = i64;

/// Timestamp representation in milliseconds since Unix epoch
pub type TimestampMillis = i64;

const BYTESIZE_K: usize = 1024;
const BYTESIZE_M: usize = 1048576;
const BYTESIZE_G: usize = 1073741824;

/// Human-readable byte size representation with parsing/serialization support
///
/// # Example:
/// ```
/// use rmqtt_utils::Bytesize;
///
/// // Create from string
/// let size = Bytesize::from("2G512M");
/// assert_eq!(size.as_usize(), 2_684_354_560);
///
/// // Create from integer
/// let size = Bytesize::from(1024);
/// assert_eq!(size.string(), "1K");
/// ```
#[derive(Clone, Copy, Default)]
pub struct Bytesize(pub usize);

impl Bytesize {
    /// Convert to u32 (may truncate on 32-bit platforms)
    ///
    /// # Example:
    /// ```
    /// let size = rmqtt_utils::Bytesize(5000);
    /// assert_eq!(size.as_u32(), 5000);
    /// ```
    #[inline]
    pub fn as_u32(&self) -> u32 {
        self.0 as u32
    }

    /// Convert to u64
    ///
    /// # Example:
    /// ```
    /// let size = rmqtt_utils::Bytesize(usize::MAX);
    /// assert_eq!(size.as_u64(), usize::MAX as u64);
    /// ```
    #[inline]
    pub fn as_u64(&self) -> u64 {
        self.0 as u64
    }

    /// Get underlying usize value
    ///
    /// # Example:
    /// ```
    /// let size = rmqtt_utils::Bytesize(1024);
    /// assert_eq!(size.as_usize(), 1024);
    /// ```
    #[inline]
    pub fn as_usize(&self) -> usize {
        self.0
    }

    /// Format bytesize to human-readable string
    ///
    /// # Example:
    /// ```
    /// let size = rmqtt_utils::Bytesize(3145728);
    /// assert_eq!(size.string(), "3M");
    ///
    /// let mixed = rmqtt_utils::Bytesize(2148532224);
    /// assert_eq!(mixed.string(), "2G1M");
    /// ```
    #[inline]
    pub fn string(&self) -> String {
        let mut v = self.0;
        let mut res = String::new();

        let g = v / BYTESIZE_G;
        if g > 0 {
            res.push_str(&format!("{}G", g));
            v %= BYTESIZE_G;
        }

        let m = v / BYTESIZE_M;
        if m > 0 {
            res.push_str(&format!("{}M", m));
            v %= BYTESIZE_M;
        }

        let k = v / BYTESIZE_K;
        if k > 0 {
            res.push_str(&format!("{}K", k));
            v %= BYTESIZE_K;
        }

        if v > 0 {
            res.push_str(&format!("{}B", v));
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
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = to_bytesize(&String::deserialize(deserializer)?);
        Ok(Bytesize(v))
    }
}

/// Parse human-readable byte size string to usize
///
/// # Example:
/// ```
/// let bytes = rmqtt_utils::to_bytesize("2G512K");
/// assert_eq!(bytes, 2148007936);
///
/// let complex = rmqtt_utils::to_bytesize("1G500M256K1024B");
/// assert_eq!(complex, 1598292992);
/// ```
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

/// Deserialize Duration from human-readable string format
#[inline]
pub fn deserialize_duration<'de, D>(deserializer: D) -> std::result::Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let v = String::deserialize(deserializer)?;
    Ok(to_duration(&v))
}

/// Deserialize optional Duration from string
#[inline]
pub fn deserialize_duration_option<'de, D>(deserializer: D) -> std::result::Result<Option<Duration>, D::Error>
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

/// Convert human-readable duration string to Duration
///
/// # Supported units:
/// - Y: milliseconds (e.g. "100Y" = 100ms)
/// - s: seconds
/// - m: minutes
/// - h: hours
/// - d: days
/// - w: weeks
/// - f: fortnight (2 weeks)
///
/// # Example:
/// ```
/// let duration = rmqtt_utils::to_duration("1h30m15s");
/// assert_eq!(duration.as_secs(), 5415);
///
/// let complex = rmqtt_utils::to_duration("2w3d12h");
/// assert_eq!(complex.as_secs(), 1512000);
/// ```
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

/// Deserialize SocketAddr with error handling
#[inline]
pub fn deserialize_addr<'de, D>(deserializer: D) -> std::result::Result<SocketAddr, D::Error>
where
    D: Deserializer<'de>,
{
    let addr = String::deserialize(deserializer)?
        .parse::<std::net::SocketAddr>()
        .map_err(serde::de::Error::custom)?;
    Ok(addr)
}

/// Deserialize optional SocketAddr with port handling
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

/// Deserialize optional datetime from string
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

/// Serialize optional datetime to string
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

/// Internal datetime parsing helper
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

/// Get current timestamp as Duration
///
/// # Example:
/// ```
/// let ts = rmqtt_utils::timestamp();
/// assert!(ts.as_secs() > 0);
/// ```
#[inline]
pub fn timestamp() -> Duration {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_else(|_| {
        let now = chrono::Local::now();
        Duration::new(now.timestamp() as u64, now.timestamp_subsec_nanos())
    })
}

/// Get current timestamp in seconds
///
/// # Example:
/// ```
/// let ts = rmqtt_utils::timestamp_secs();
/// assert!(ts > 0);
/// ```
#[inline]
pub fn timestamp_secs() -> Timestamp {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|t| t.as_secs() as i64)
        .unwrap_or_else(|_| chrono::Local::now().timestamp())
}

/// Get current timestamp in milliseconds
///
/// # Example:
/// ```
/// let ts = rmqtt_utils::timestamp_millis();
/// assert!(ts > 0);
/// ```
#[inline]
pub fn timestamp_millis() -> TimestampMillis {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|t| t.as_millis() as i64)
        .unwrap_or_else(|_| chrono::Local::now().timestamp_millis())
}

/// Format timestamp (seconds) to human-readable string
#[inline]
pub fn format_timestamp(t: Timestamp) -> String {
    if t <= 0 {
        "".into()
    } else {
        use chrono::TimeZone;
        if let chrono::LocalResult::Single(t) = chrono::Local.timestamp_opt(t, 0) {
            t.format("%Y-%m-%d %H:%M:%S").to_string()
        } else {
            "".into()
        }
    }
}

/// Format current timestamp to string
///
/// # Example:
/// ```
/// let now = rmqtt_utils::format_timestamp_now();
/// assert!(!now.is_empty());
/// ```
#[inline]
pub fn format_timestamp_now() -> String {
    format_timestamp(timestamp_secs())
}

/// Format millisecond timestamp to string
#[inline]
pub fn format_timestamp_millis(t: TimestampMillis) -> String {
    if t <= 0 {
        "".into()
    } else {
        use chrono::TimeZone;
        if let chrono::LocalResult::Single(t) = chrono::Local.timestamp_millis_opt(t) {
            t.format("%Y-%m-%d %H:%M:%S%.3f").to_string()
        } else {
            "".into()
        }
    }
}

/// Format current millisecond timestamp to string
///
/// # Example:
/// ```
/// let now = rmqtt_utils::format_timestamp_millis_now();
/// assert!(!now.is_empty());
/// ```
#[inline]
pub fn format_timestamp_millis_now() -> String {
    format_timestamp_millis(timestamp_millis())
}

/// Cluster node address representation (ID@Address)
///
/// # Example:
/// ```
/// use rmqtt_utils::NodeAddr;
///
/// // Parse from string
/// let node: NodeAddr = "123@mqtt.example.com:1883".parse().unwrap();
/// assert_eq!(node.id, 123);
/// assert_eq!(node.addr, "mqtt.example.com:1883");
///
/// // Direct construction
/// let node = NodeAddr {
///     id: 456,
///     addr: rmqtt_utils::Addr::from("localhost:8883")
/// };
/// ```
#[derive(Clone, Serialize)]
pub struct NodeAddr {
    /// Unique node identifier
    pub id: NodeId,

    /// Network address in host:port format
    pub addr: Addr,
}

impl std::fmt::Debug for NodeAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{:?}", self.id, self.addr)
    }
}

impl FromStr for NodeAddr {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('@').collect();
        if parts.len() < 2 {
            return Err(anyhow!(format!("NodeAddr format error, {}", s)));
        }
        let id = NodeId::from_str(parts[0])?;
        let addr = Addr::from(parts[1]);
        Ok(NodeAddr { id, addr })
    }
}

impl<'de> de::Deserialize<'de> for NodeAddr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        NodeAddr::from_str(&String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}
