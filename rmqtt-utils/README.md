[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-utils

[![crates.io page](https://img.shields.io/crates/v/rmqtt-utils.svg)](https://crates.io/crates/rmqtt-utils)
[![docs.rs page](https://docs.rs/rmqtt-utils/badge.svg)](https://docs.rs/rmqtt-utils/latest/rmqtt_utils)

Common utilities for the RMQTT MQTT broker: byte sizes, durations, timestamps, node addresses, env var expansion, and atomic counters.

## Type aliases

```rust
pub type NodeId = u64;
pub type Addr = ByteString;
pub type Timestamp = i64;
pub type TimestampMillis = i64;
```

## `Bytesize` — human-readable byte size

```rust
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub struct Bytesize(pub usize);

impl Bytesize {
    pub fn as_u32(&self) -> u32;
    pub fn as_u64(&self) -> u64;
    pub fn as_usize(&self) -> usize;
    pub fn string(&self) -> String;     // "3M", "2G1M512K", etc.
}

// From<usize>, TryFrom<&str>, FromStr, Deref<Target=usize>, DerefMut
```

## `NodeAddr` — cluster node address (`ID@host:port`)

```rust
#[derive(Clone, Serialize)]
pub struct NodeAddr {
    pub id: NodeId,       // u64
    pub addr: Addr,       // ByteString — "host:port"
}
// FromStr: "1@127.0.0.1:1883".parse::<NodeAddr>()?
// Serialize, Deserialize
```

## Free functions

```rust
pub fn to_bytesize(text: &str) -> Result<usize, ParseSizeError>;
// Supported suffixes: G, M, K, B. E.g. "2G512K" -> 2148007936

pub fn to_duration(text: &str) -> Duration;
// Supported: ms, s, m, h, d, w, f (fortnight). E.g. "1h30m15s" -> 5415s

pub fn timestamp() -> Duration;              // SystemTime::now().duration_since(UNIX_EPOCH)
pub fn timestamp_secs() -> Timestamp;        // i64 seconds
pub fn timestamp_millis() -> TimestampMillis;// i64 milliseconds

pub fn format_timestamp(t: Timestamp) -> String;              // "%Y-%m-%d %H:%M:%S"
pub fn format_timestamp_now() -> String;
pub fn format_timestamp_millis(t: TimestampMillis) -> String; // "%Y-%m-%d %H:%M:%S%.3f"
pub fn format_timestamp_millis_now() -> String;

pub fn expand_env_vars(value: &str) -> String;
// Expands ${ENV:VAR_NAME} placeholders using regex. Logs warning for unset vars.
```

## Serde helpers

```rust
pub fn deserialize_duration<'de, D>(d) -> Result<Duration, D::Error>;
pub fn deserialize_duration_option<'de, D>(d) -> Result<Option<Duration>, D::Error>;
pub fn deserialize_addr<'de, D>(d) -> Result<SocketAddr, D::Error>;
pub fn deserialize_addr_option<'de, D>(d) -> Result<Option<SocketAddr>, D::Error>;
pub fn deserialize_datetime_option<'de, D>(d) -> Result<Option<Duration>, D::Error>;
pub fn serialize_datetime_option<S>(t: &Option<Duration>, s) -> Result<S::Ok, S::Error>;
pub fn deserialize_expand_env_vars<'de, D>(d) -> Result<String, D::Error>;
pub fn deserialize_expand_env_vars_option<'de, D>(d) -> Result<Option<String>, D::Error>;
```

## `Counter` / `StatsMergeMode`

```rust
pub struct Counter(AtomicIsize, AtomicIsize, StatsMergeMode);
// (current, max, merge_mode)

impl Counter {
    pub fn new() -> Self;                           // (0, 0, None)
    pub fn new_with(c: isize, max: isize, m: StatsMergeMode) -> Self;

    pub fn inc(&self);                              // current += 1, update max
    pub fn incs(&self, c: isize);                   // current += c, update max
    pub fn current_inc(&self);                      // current += 1, no max update
    pub fn current_incs(&self, c: isize);
    pub fn current_set(&self, c: isize);
    pub fn sets(&self, c: isize);                   // set current, update max
    pub fn dec(&self);
    pub fn decs(&self, c: isize);
    pub fn count_min(&self, c: isize);
    pub fn count_max(&self, c: isize);
    pub fn max_max(&self, m: isize);
    pub fn max_min(&self, m: isize);

    pub fn count(&self) -> isize;
    pub fn max(&self) -> isize;
    pub fn add(&self, other: &Self);                // atomic addition
    pub fn set(&self, other: &Self);                // atomic replacement
    pub fn merge(&self, other: &Self);              // merge using StatsMergeMode
    pub fn to_json(&self) -> serde_json::Value;     // {"count":..., "max":...}
}

pub enum StatsMergeMode { None, Sum, Average, Max, Min }
```

## Safety

`#![deny(unsafe_code)]` — zero unsafe code.

## License

MIT OR Apache-2.0
