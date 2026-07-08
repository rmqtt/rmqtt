[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-utils

[![crates.io page](https://img.shields.io/crates/v/rmqtt-utils.svg)](https://crates.io/crates/rmqtt-utils)
[![docs.rs page](https://docs.rs/rmqtt-utils/badge.svg)](https://docs.rs/rmqtt-utils/latest/rmqtt_utils)

RMQTT MQTT Broker 通用工具：字节大小、持续时间、时间戳、节点地址、环境变量展开、原子计数器。

## 类型别名

```rust
pub type NodeId = u64;
pub type Addr = ByteString;
pub type Timestamp = i64;
pub type TimestampMillis = i64;
```

## `Bytesize` — 人类可读字节大小

```rust
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub struct Bytesize(pub usize);

impl Bytesize {
    pub fn as_u32(&self) -> u32;
    pub fn as_u64(&self) -> u64;
    pub fn as_usize(&self) -> usize;
    pub fn string(&self) -> String;     // "3M", "2G1M512K"
}

// From<usize>, TryFrom<&str>, FromStr, Deref<Target=usize>, DerefMut
```

## `NodeAddr` — 集群节点地址（`ID@host:port`）

```rust
#[derive(Clone, Serialize)]
pub struct NodeAddr {
    pub id: NodeId,       // u64
    pub addr: Addr,       // ByteString — "host:port"
}
// FromStr: "1@127.0.0.1:1883".parse::<NodeAddr>()?
// Serialize, Deserialize
```

## 函数

```rust
pub fn to_bytesize(text: &str) -> Result<usize, ParseSizeError>;
// 支持后缀: G, M, K, B。如 "2G512K" -> 2148007936

pub fn to_duration(text: &str) -> Duration;
// 支持: ms, s, m, h, d, w, f（两周）。如 "1h30m15s" -> 5415s

pub fn timestamp() -> Duration;              // SystemTime::now().duration_since(UNIX_EPOCH)
pub fn timestamp_secs() -> Timestamp;        // i64 秒
pub fn timestamp_millis() -> TimestampMillis;// i64 毫秒

pub fn format_timestamp(t: Timestamp) -> String;              // "%Y-%m-%d %H:%M:%S"
pub fn format_timestamp_now() -> String;
pub fn format_timestamp_millis(t: TimestampMillis) -> String; // "%Y-%m-%d %H:%M:%S%.3f"
pub fn format_timestamp_millis_now() -> String;

pub fn expand_env_vars(value: &str) -> String;
// 使用正则表达式展开 ${ENV:VAR_NAME} 占位符。未设置的环境变量记录 warning。
```

## Serde 辅助函数

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

    pub fn inc(&self);                              // current += 1, 更新 max
    pub fn incs(&self, c: isize);                   // current += c, 更新 max
    pub fn current_inc(&self);                      // current += 1, 不更新 max
    pub fn current_incs(&self, c: isize);
    pub fn current_set(&self, c: isize);
    pub fn sets(&self, c: isize);                   // 设置 current, 更新 max
    pub fn dec(&self);
    pub fn decs(&self, c: isize);
    pub fn count_min(&self, c: isize);
    pub fn count_max(&self, c: isize);
    pub fn max_max(&self, m: isize);
    pub fn max_min(&self, m: isize);

    pub fn count(&self) -> isize;
    pub fn max(&self) -> isize;
    pub fn add(&self, other: &Self);                // 原子加法
    pub fn set(&self, other: &Self);                // 原子替换
    pub fn merge(&self, other: &Self);              // 按 StatsMergeMode 合并
    pub fn to_json(&self) -> serde_json::Value;     // {"count":..., "max":...}
}

pub enum StatsMergeMode { None, Sum, Average, Max, Min }
```

## `RateCounter` — 无锁吞吐量和并发跟踪器

```rust
use std::time::Duration;
use rmqtt_utils::RateCounter;

let rc = RateCounter::new();

// 任务到达：跟踪吞吐量、并发数和峰值
rc.incs(42);
assert_eq!(rc.total(), 42);
assert_eq!(rc.current(), 42);
assert_eq!(rc.max(), 42);

// 更高峰值
rc.incs(10);
assert_eq!(rc.max(), 52);

// 任务完成：并发数减少，峰值不变
rc.decs(20);
assert_eq!(rc.current(), 32);
assert_eq!(rc.max(), 52);

// 计算 3 秒间隔内的每秒速率
rc.tick(Duration::from_secs(3));
assert!((rc.speed() - 17.333).abs() < 1e-12);
```

纯原子操作、零锁的速率计数器，跟踪以下指标：
- **`total`**：自创建或重置以来的累计计数。
- **`speed`**：每秒吞吐量，通过以已知采样间隔调用 `tick(interval)` 计算。
- **`current`**：当前并发/活跃计数（`inc`/`incs` 增加，`dec`/`decs` 减少）。
- **`max`**：`current` 的历史峰值。

`Clone` 通过 `Arc` 共享底层原子变量 — 非常适合传入 `tokio::spawn`。
`snapshot()` 创建独立的深拷贝（值相同，新原子变量）。
Serde 序列化/反序列化为快照（可安全跨节点传输）。

| 方法 | 作用 |
|------|------|
| `inc()` / `incs(n)` | 增加 `total` 和 `current`；更新 `max` |
| `dec()` / `decs(n)` | 仅减少 `current` |
| `tick(interval)` | 计算 `speed = (total - last_total) / interval` |
| `reset()` | 清零所有计数器 |
| `total()` / `speed()` / `current()` / `max()` | 读取单个计数器值 |
| `snapshot()` | 创建独立的深拷贝 |

## 安全性

`#![deny(unsafe_code)]` — 零 unsafe 代码。

## 许可证

MIT OR Apache-2.0
