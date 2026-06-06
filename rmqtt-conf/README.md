[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-conf

[![crates.io page](https://img.shields.io/crates/v/rmqtt-conf.svg)](https://crates.io/crates/rmqtt-conf)
[![docs.rs page](https://docs.rs/rmqtt-conf/badge.svg)](https://docs.rs/rmqtt-conf/latest/rmqtt_conf)

Centralized configuration management for the RMQTT MQTT broker.

## Public API

### `Settings` — singleton (wraps `Arc<Inner>`)

```rust
impl Settings {
    pub fn init(opts: Options) -> Result<&'static Self>;  // must call once before instance()
    pub fn instance() -> &'static Self;                    // panics if init() was not called
    pub fn logs() -> Result<()>;                           // logs config at INFO level
}
```

### `Inner` — configuration struct (deserialized from TOML)

```rust
pub struct Inner {
    pub task: Task,             // [task] section
    pub node: Node,             // [node] section
    pub rpc: Rpc,               // [rpc] section
    pub log: Log,               // [log] section
    pub listeners: Listeners,   // [listener] section
    pub plugins: Plugins,       // [plugins] section
    pub mqtt: Mqtt,             // [mqtt] section
    pub opts: Options,          // CLI overrides (skipped in serde)
}
```

### `Options` — CLI arguments (`clap::Parser`)

| `clap` attr | Field | Type | Default |
|---|---|---|---|
| `-f, --config` | `cfg_name` | `Option<String>` | `None` |
| `-V, --version` | `version` | `bool` | `false` |
| `--id` | `node_id` | `Option<NodeId>` (u64) | `None` |
| `--plugins-default-startups` | `plugins_default_startups` | `Option<Vec<String>>` | `None` |
| `--node-grpc-addrs` | `node_grpc_addrs` | `Option<Vec<NodeAddr>>` | `None` |
| `--raft-peer-addrs` | `raft_peer_addrs` | `Option<Vec<NodeAddr>>` | `None` |
| `--raft-leader-id` | `raft_leader_id` | `Option<NodeId>` (u64) | `None` |

### `Task` — executor config

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `exec_workers` | `usize` | `1000` | Concurrent tasks |
| `exec_queue_max` | `usize` | `300_000` | Max queue capacity |

### `Node` — cluster node config

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `id` | `NodeId` (u64) | `0` | Node identifier |
| `cookie` | `String` | `"rmqttsecretcookie"` | Cluster auth cookie |
| `busy` | `Busy` | (see below) | Busy detection settings |

`Busy` fields:
- `check_enable: bool` (default `true`)
- `update_interval: Duration` (default `2s`)
- `loadavg: f32` (default `80.0`)
- `cpuloadavg: f32` (default `90.0`)
- `handshaking: isize` (default `0`)

### `Rpc` — gRPC inter-node config

| Field | Type | Default |
|-------|------|---------|
| `server_addr` | `SocketAddr` | `0.0.0.0:5363` |
| `reuseaddr` | `bool` | `true` |
| `reuseport` | `bool` | `false` |

### `Log` — logging config

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `to` | `To` enum | `Console` | `off` / `console` / `file` / `both` |
| `level` | `Level` enum | `Info` | `trace` / `debug` / `info` / `warn` / `error` |
| `dir` | `String` | `"/var/log/rmqtt"` | Log directory |
| `file` | `String` | `"rmqtt.log"` | Log filename |

`Log` also exposes `fn filename(&self) -> String` which joins `dir + "/" + file`.

### `Listeners` — network listener collection

Parsed from raw TOML `[listener.<protocol>.<name>]` into protocol-specific maps keyed by port:

```rust
impl Listeners {
    pub fn tcp(&self, port: u16) -> Option<Listener>;
    pub fn tls(&self, port: u16) -> Option<Listener>;
    pub fn ws(&self, port: u16) -> Option<Listener>;
    pub fn wss(&self, port: u16) -> Option<Listener>;
    pub fn quic(&self, port: u16) -> Option<Listener>;
    pub fn get(&self, port: u16) -> Option<Listener>;  // search all protocols
}
```

### `Listener` — single listener config (wraps `Arc<ListenerInner>`)

Fields via `Deref<Target = ListenerInner>`:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | `String` | `"external/tcp"` | Listener name |
| `enable` | `bool` | `true` | Enable this listener |
| `addr` | `SocketAddr` | `0.0.0.0:1883` | Bind address |
| `max_connections` | `usize` | `1_024_000` | Max concurrent connections |
| `max_handshaking_limit` | `usize` | `500` | Max concurrent handshakes |
| `max_packet_size` | `Bytesize` | `1M` | Max MQTT packet size |
| `backlog` | `i32` | `1024` | TCP listen backlog |
| `nodelay` | `bool` | `false` | TCP_NODELAY |
| `reuseaddr` | `Option<bool>` | `Some(true)` | SO_REUSEADDR |
| `reuseport` | `Option<bool>` | `None` | SO_REUSEPORT |
| `allow_anonymous` | `bool` | `false` | Allow anonymous login |
| `min_keepalive` | `u16` | `0` | Min keepalive (seconds) |
| `max_keepalive` | `u16` | `65535` | Max keepalive (seconds) |
| `allow_zero_keepalive` | `bool` | `true` | Allow keepalive=0 |
| `keepalive_backoff` | `f32` | `0.75` | Keepalive backoff factor |
| `max_inflight` | `NonZeroU16` | `16` | Max inflight messages |
| `handshake_timeout` | `Duration` | `15s` | MQTT handshake timeout |
| `max_mqueue_len` | `usize` | `1000` | Max message queue length |
| `mqueue_rate_limit` | `(NonZeroU32, Duration)` | `(MAX, 1s)` | Queue rate (burst, period) |
| `max_clientid_len` | `usize` | `65535` | Max ClientId length |
| `max_qos_allowed` | `QoS` | `2` (ExactlyOnce) | Max allowed QoS level |
| `max_topic_levels` | `usize` | `0` (unlimited) | Max topic levels |
| `session_expiry_interval` | `Duration` | `7200s` | Session expiry |
| `max_session_expiry_interval` | `Duration` | `0` | Max session expiry client can request |
| `message_retry_interval` | `Duration` | `30s` | QoS msg retry interval |
| `message_expiry_interval` | `Duration` | `300s` | Message expiry; `0` → `u32::MAX` |
| `max_subscriptions` | `usize` | `0` (unlimited) | Max subscriptions per client |
| `max_topic_aliases` | `u16` | `0` | Max topic aliases |
| `cross_certificate` | `bool` | `false` | Verify cross certificates |
| `cert` | `Option<String>` | `None` | TLS cert file path |
| `key` | `Option<String>` | `None` | TLS key file path |
| `client_ca_certs` | `Option<String>` | `None` | Client CA cert file |
| `limit_subscription` | `bool` | `false` | Enable subscription limit |
| `delayed_publish` | `bool` | `false` | Enable delayed publish |
| `proxy_protocol` | `bool` | `false` | Enable PROXY protocol v1/v2 |
| `proxy_protocol_timeout` | `Duration` | `5s` | PROXY header read timeout |
| `cert_cn_as_username` | `bool` | `false` | Use TLS CN as username |
| `cert_subject_dn_as_username` | `bool` | `false` | Use TLS subject DN as username |
| `collect_cert_info` | `bool` | `false` | Collect TLS cert info |
| `idle_timeout` | `Duration` | `90s` | QUIC idle timeout |

### `Plugins` — plugin config loading

```rust
impl Plugins {
    // File required, returns Error if missing
    pub fn load_config<T: Deserialize>(&self, name: &str) -> Result<T>;

    // File optional; logs warning if missing and uses defaults
    pub fn load_config_default<T: Deserialize>(&self, name: &str) -> Result<T>;

    // With env list keys (space-separated env vars parsed as lists)
    pub fn load_config_with<T: Deserialize>(&self, name: &str, env_list_keys: &[&str]) -> Result<T>;

    // Optional file + env list keys
    pub fn load_config_default_with<T: Deserialize>(&self, name: &str, env_list_keys: &[&str]) -> Result<T>;
}
```

Config files are loaded from `{plugins.dir}/{name}.toml`. Env prefix: `rmqtt_plugin_{name}`.

### `Mqtt` — MQTT protocol config

| Field | Type | Default |
|-------|------|---------|
| `delayed_publish_max` | `usize` | `100_000` |
| `delayed_publish_immediate` | `bool` | `true` |
| `max_sessions` | `isize` | `0` (unlimited) |

### Config loading priority

1. `/etc/rmqtt/rmqtt.{toml,json,...}` (optional)
2. `/etc/rmqtt.{toml,json,...}` (optional)
3. `./rmqtt.{toml,json,...}` (optional)
4. `-f` / `--config` specified path (optional)
5. `RMQTT_*` env vars (e.g. `RMQTT_NODE_ID=1`, `RMQTT_RPC__SERVER_ADDR=0.0.0.0:5363`)

## License

MIT OR Apache-2.0
