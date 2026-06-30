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

## `CircuitBreaker` — lock‑free fast‑fail degradation

```rust
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    pub enabled: bool,                           // default: false
    pub failure_threshold: usize,                // default: 10
    pub reset_timeout: Duration,                 // default: 15s
    pub half_open_success_threshold: usize,      // default: 3
}

pub enum CircuitState { Closed, Open, HalfOpen }

impl CircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Self;

    /// Returns true if the circuit is OPEN and the caller should skip
    /// the external operation. Side‑effect: transitions OPEN → HALF_OPEN
    /// after reset_timeout has elapsed.
    pub fn is_blocked(&self) -> bool;
    pub fn record_success(&self);
    pub fn record_failure(&self);
    pub fn state(&self) -> CircuitState;
    pub fn reset(&self);
    pub fn config(&self) -> &CircuitBreakerConfig;
}
```

A pure‑atomics, zero‑lock circuit breaker for proactive failure detection.
State machine: `Closed ⇄ Open ⇄ HalfOpen`. Shared safely across threads
via `Arc<CircuitBreaker>`. Used by `rmqtt-message-storage` to avoid
blocking the broker when Redis is unreachable.

| State | Behaviour |
|-------|-----------|
| **Closed** | Normal operation. Counts consecutive failures. |
| **Open** | `is_blocked()` returns `true`. All callers skip the external op. |
| **HalfOpen** | All requests pass through (no rate limiting); closes after `half_open_success_threshold` consecutive successes, or re‑opens on any failure. |

## Safety

`#![deny(unsafe_code)]` — zero unsafe code.

## `RateCounter` — lock‑free throughput & in‑flight tracker

```rust
use std::time::Duration;
use rmqtt_utils::RateCounter;

let rc = RateCounter::new();

// Task arrives: track throughput, in-flight, and peak
rc.incs(42);
assert_eq!(rc.total(), 42);
assert_eq!(rc.current(), 42);
assert_eq!(rc.max(), 42);

// Higher peak
rc.incs(10);
assert_eq!(rc.max(), 52);

// Task completes: in-flight decreases, peak unchanged
rc.decs(20);
assert_eq!(rc.current(), 32);
assert_eq!(rc.max(), 52);

// Compute per-second rate over a 3 s interval
rc.tick(Duration::from_secs(3));
assert!((rc.speed() - 17.333).abs() < 1e-12);
```

A pure‑atomics, zero‑lock rate counter that tracks:
- **`total`**: Cumulative count since construction or reset.
- **`speed`**: Per‑second throughput, computed by calling `tick(interval)` at a known sampling interval.
- **`current`**: Current in‑flight / active count (incremented by `inc`/`incs`, decremented by `dec`/`decs`).
- **`max`**: Historical peak of the `current` field.

`Clone` shares the underlying atomics via `Arc` — ideal for passing into `tokio::spawn`.  
`snapshot()` creates an independent deep copy with the same values but new atomics.  
Serde serialises/deserialises as a snapshot (safe for cross‑node transfer).

| Method | Effect |
|--------|--------|
| `inc()` / `incs(n)` | Increment `total` and `current`; update `max` |
| `dec()` / `decs(n)` | Decrement `current` only |
| `tick(interval)` | Compute `speed = (total - last_total) / interval` |
| `reset()` | Zero all counters |
| `total()` / `speed()` / `current()` / `max()` | Read individual counters |
| `snapshot()` | Create independent deep copy |

## License

MIT OR Apache-2.0
