[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-conf

[![crates.io page](https://img.shields.io/crates/v/rmqtt-conf.svg)](https://crates.io/crates/rmqtt-conf)
[![docs.rs page](https://docs.rs/rmqtt-conf/badge.svg)](https://docs.rs/rmqtt-conf/latest/rmqtt_conf)

RMQTT MQTT Broker 的集中式配置管理系统。

## 公开 API

### `Settings` — 单例（包装 `Arc<Inner>`）

```rust
impl Settings {
    pub fn init(opts: Options) -> Result<&'static Self>;  // 必须先调用一次
    pub fn instance() -> &'static Self;                    // 未 init 会 panic
    pub fn logs() -> Result<()>;                           // INFO 级别打印配置
}
```

### `Inner` — 配置结构体（从 TOML 反序列化）

```rust
pub struct Inner {
    pub task: Task,             // [task] 节
    pub node: Node,             // [node] 节
    pub rpc: Rpc,               // [rpc] 节
    pub log: Log,               // [log] 节
    pub listeners: Listeners,   // [listener] 节
    pub plugins: Plugins,       // [plugins] 节
    pub mqtt: Mqtt,             // [mqtt] 节
    pub opts: Options,          // CLI 覆盖（反序列化时跳过）
}
```

### `Options` — CLI 参数（`clap::Parser`）

| clap 属性 | 字段 | 类型 | 默认值 |
|---|---|---|---|
| `-f, --config` | `cfg_name` | `Option<String>` | `None` |
| `-V, --version` | `version` | `bool` | `false` |
| `--id` | `node_id` | `Option<NodeId>` (u64) | `None` |
| `--plugins-default-startups` | `plugins_default_startups` | `Option<Vec<String>>` | `None` |
| `--node-grpc-addrs` | `node_grpc_addrs` | `Option<Vec<NodeAddr>>` | `None` |
| `--raft-peer-addrs` | `raft_peer_addrs` | `Option<Vec<NodeAddr>>` | `None` |
| `--raft-leader-id` | `raft_leader_id` | `Option<NodeId>` (u64) | `None` |

### `Task` — 执行器配置

| 字段 | 类型 | 默认值 | 说明 |
|-------|------|---------|------|
| `exec_workers` | `usize` | `1000` | 并发任务数 |
| `exec_queue_max` | `usize` | `300_000` | 最大队列容量 |

### `Node` — 集群节点配置

| 字段 | 类型 | 默认值 | 说明 |
|-------|------|---------|------|
| `id` | `NodeId` (u64) | `0` | 节点标识 |
| `cookie` | `String` | `"rmqttsecretcookie"` | 集群认证 cookie |
| `busy` | `Busy` | （见下方） | 繁忙检测设置 |

`Busy` 字段：
- `check_enable: bool` (默认 `true`)
- `update_interval: Duration` (默认 `2s`)
- `loadavg: f32` (默认 `80.0`)
- `cpuloadavg: f32` (默认 `90.0`)
- `handshaking: isize` (默认 `0`)

### `Rpc` — gRPC 节点间通信配置

| 字段 | 类型 | 默认值 |
|-------|------|---------|
| `server_addr` | `SocketAddr` | `0.0.0.0:5363` |
| `reuseaddr` | `bool` | `true` |
| `reuseport` | `bool` | `false` |

### `Log` — 日志配置

| 字段 | 类型 | 默认值 | 说明 |
|-------|------|---------|------|
| `to` | `To` 枚举 | `Console` | `off` / `console` / `file` / `both` |
| `level` | `Level` 枚举 | `Info` | `trace` / `debug` / `info` / `warn` / `error` |
| `dir` | `String` | `"/var/log/rmqtt"` | 日志目录 |
| `file` | `String` | `"rmqtt.log"` | 日志文件名 |

`Log` 还提供 `fn filename(&self) -> String` 方法拼接路径。

### `Listeners` — 网络监听器集合

从 TOML `[listener.<protocol>.<name>]` 解析，按端口 key 存储到各协议 HashMap：

```rust
impl Listeners {
    pub fn tcp(&self, port: u16) -> Option<Listener>;
    pub fn tls(&self, port: u16) -> Option<Listener>;
    pub fn ws(&self, port: u16) -> Option<Listener>;
    pub fn wss(&self, port: u16) -> Option<Listener>;
    pub fn quic(&self, port: u16) -> Option<Listener>;
    pub fn get(&self, port: u16) -> Option<Listener>;  // 搜索所有协议
}
```

### `Listener` — 单监听器配置（包装 `Arc<ListenerInner>`）

通过 `Deref<Target = ListenerInner>` 访问字段：

| 字段 | 类型 | 默认值 | 说明 |
|-------|------|---------|------|
| `name` | `String` | `"external/tcp"` | 监听器名称 |
| `enable` | `bool` | `true` | 启用该监听器 |
| `addr` | `SocketAddr` | `0.0.0.0:1883` | 绑定地址 |
| `max_connections` | `usize` | `1_024_000` | 最大并发连接数 |
| `max_handshaking_limit` | `usize` | `500` | 最大并发握手数 |
| `max_packet_size` | `Bytesize` | `1M` | 最大 MQTT 包大小 |
| `backlog` | `i32` | `1024` | TCP 监听 backlog |
| `nodelay` | `bool` | `false` | TCP_NODELAY |
| `reuseaddr` | `Option<bool>` | `Some(true)` | SO_REUSEADDR |
| `reuseport` | `Option<bool>` | `None` | SO_REUSEPORT |
| `allow_anonymous` | `bool` | `false` | 允许匿名登录 |
| `min_keepalive` | `u16` | `0` | 最小保活间隔（秒） |
| `max_keepalive` | `u16` | `65535` | 最大保活间隔（秒） |
| `allow_zero_keepalive` | `bool` | `true` | 允许 keepalive=0 |
| `keepalive_backoff` | `f32` | `0.75` | 保活退避因子 |
| `max_inflight` | `NonZeroU16` | `16` | 最大 in-flight 消息数 |
| `handshake_timeout` | `Duration` | `15s` | MQTT 握手超时 |
| `max_mqueue_len` | `usize` | `1000` | 最大消息队列长度 |
| `mqueue_rate_limit` | `(NonZeroU32, Duration)` | `(MAX, 1s)` | 队列速率（突发量，周期） |
| `max_clientid_len` | `usize` | `65535` | 最大 ClientId 长度 |
| `max_qos_allowed` | `QoS` | `2` (ExactlyOnce) | 允许的最大 QoS |
| `max_topic_levels` | `usize` | `0`（不限） | 最大主题层级数 |
| `session_expiry_interval` | `Duration` | `7200s` | 会话过期时间 |
| `max_session_expiry_interval` | `Duration` | `0` | 客户端可请求的最大过期时间 |
| `message_retry_interval` | `Duration` | `30s` | QoS 消息重试间隔 |
| `message_expiry_interval` | `Duration` | `300s` | 消息过期时间；`0` → `u32::MAX` |
| `max_subscriptions` | `usize` | `0`（不限） | 每客户端最大订阅数 |
| `max_topic_aliases` | `u16` | `0` | 最大主题别名数 |
| `cross_certificate` | `bool` | `false` | 验证交叉证书 |
| `cert` | `Option<String>` | `None` | TLS 证书文件路径 |
| `key` | `Option<String>` | `None` | TLS 密钥文件路径 |
| `client_ca_certs` | `Option<String>` | `None` | 客户端 CA 证书文件 |
| `limit_subscription` | `bool` | `false` | 启用订阅限制 |
| `delayed_publish` | `bool` | `false` | 启用延迟发布 |
| `proxy_protocol` | `bool` | `false` | 启用 PROXY protocol |
| `proxy_protocol_timeout` | `Duration` | `5s` | PROXY 头读取超时 |
| `cert_cn_as_username` | `bool` | `false` | 使用 TLS CN 作为用户名 |
| `cert_subject_dn_as_username` | `bool` | `false` | 使用 TLS 主题 DN 作为用户名 |
| `collect_cert_info` | `bool` | `false` | 收集 TLS 证书信息 |
| `idle_timeout` | `Duration` | `90s` | QUIC 空闲超时 |

### `Plugins` — 插件配置加载

```rust
impl Plugins {
    pub fn load_config<T: Deserialize>(&self, name: &str) -> Result<T>;           // 文件必须存在
    pub fn load_config_default<T: Deserialize>(&self, name: &str) -> Result<T>;   // 文件可选
    pub fn load_config_with<T: Deserialize>(&self, name: &str, env_list_keys: &[&str]) -> Result<T>;
    pub fn load_config_default_with<T: Deserialize>(&self, name: &str, env_list_keys: &[&str]) -> Result<T>;
}
```

配置文件路径：`{plugins.dir}/{name}.toml`，环境变量前缀：`rmqtt_plugin_{name}`。

### `Mqtt` — MQTT 协议配置

| 字段 | 类型 | 默认值 |
|-------|------|---------|
| `delayed_publish_max` | `usize` | `100_000` |
| `delayed_publish_immediate` | `bool` | `true` |
| `max_sessions` | `isize` | `0`（不限，负数表示禁止新建会话） |

### 配置加载优先级

1. `/etc/rmqtt/rmqtt.{toml,json,...}`（可选）
2. `/etc/rmqtt.{toml,json,...}`（可选）
3. `./rmqtt.{toml,json,...}`（可选）
4. `-f` / `--config` 指定路径（可选）
5. `RMQTT_*` 环境变量（如 `RMQTT_NODE_ID=1`, `RMQTT_RPC__SERVER_ADDR=0.0.0.0:5363`）

## 许可证

MIT OR Apache-2.0
