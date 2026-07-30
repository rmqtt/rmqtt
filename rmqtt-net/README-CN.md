[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-net

[![crates.io page](https://img.shields.io/crates/v/rmqtt-net.svg)](https://crates.io/crates/rmqtt-net)
[![docs.rs page](https://docs.rs/rmqtt-net/badge.svg)](https://docs.rs/rmqtt-net/latest/rmqtt_net)

MQTT 服务端网络层 — TCP、TLS (rustls)、WebSocket (tokio-tungstenite)、QUIC (quinn)。

## 导出项

```rust
pub use builder::{Builder, Listener, ListenerType};
pub use cert_extractor::TlsCertExtractor;     // trait
pub use error::MqttError;
pub use stream::{v3, v5, MqttStream};         // v3::MqttStream, v5::MqttStream
#[cfg(feature = "quic")]
pub use quic::QuinnBiStream;
#[cfg(feature = "tls")]
pub use rustls;
#[cfg(not(target_os = "windows"))]
#[cfg(feature = "tls")]
pub use rustls::crypto::aws_lc_rs as tls_provider;
#[cfg(target_os = "windows")]
#[cfg(feature = "tls")]
pub use rustls::crypto::ring as tls_provider;

pub type Error = anyhow::Error;
pub type Result<T> = anyhow::Result<T, Error>;
```

## `Builder` — 流畅监听器构建器

所有字段通过 `pub` 可见和 setter 方法设置。以下仅为方法签名汇总，默认值见代码注释。

| 方法 | 参数 | Builder 默认值 |
|--------|--------|----------------|
| `new()` | — | `max_connections=1_000_000, max_handshaking_limit=1_000, max_packet_size=1MB, backlog=512, nodelay=false, reuseaddr=None, reuseport=None, allow_anonymous=true, min_keepalive=0, max_keepalive=65535, allow_zero_keepalive=true, keepalive_backoff=0.75, max_inflight=16, handshake_timeout=30s, send_timeout=10s, max_mqueue_len=1000, max_clientid_len=65535, max_qos_allowed=2, max_topic_levels=0, session_expiry_interval=7200s, max_session_expiry_interval=0, message_retry_interval=20s, message_expiry_interval=300s, max_subscriptions=0, max_topic_aliases=0, ...` |
| `.name(s)` | `impl Into<String>` | `""` |
| `.laddr(a)` | `SocketAddr` | `0.0.0.0:1883` |
| `.backlog(n)` | `i32` | `512` |
| `.nodelay(v)` | `bool` | `false` |
| `.reuseaddr(v)` | `Option<bool>` | `None` |
| `.reuseport(v)` | `Option<bool>` | `None` |
| `.max_connections(n)` | `usize` | `1_000_000` |
| `.max_handshaking_limit(n)` | `usize` | `1_000` |
| `.max_packet_size(n)` | `u32` | `1_048_576` (1MB) |
| `.allow_anonymous(v)` | `bool` | `true` |
| `.min_keepalive(v)` | `u16` | `0` |
| `.max_keepalive(v)` | `u16` | `65535` |
| `.allow_zero_keepalive(v)` | `bool` | `true` |
| `.keepalive_backoff(v)` | `f32` | `0.75` |
| `.max_inflight(v)` | `NonZeroU16` | `16` |
| `.handshake_timeout(d)` | `Duration` | `30s` |
| `.send_timeout(d)` | `Duration` | `10s` |
| `.max_mqueue_len(n)` | `usize` | `1000` |
| `.mqueue_rate_limit(burst, period)` | `(NonZeroU32, Duration)` | `(MAX, 1s)` |
| `.max_clientid_len(n)` | `usize` | `65535` |
| `.max_qos_allowed(q)` | `QoS` | `ExactlyOnce` |
| `.max_topic_levels(n)` | `usize` | `0`（不限） |
| `.session_expiry_interval(d)` | `Duration` | `7200s` |
| `.max_session_expiry_interval(d)` | `Duration` | `0`（不限） |
| `.message_retry_interval(d)` | `Duration` | `20s` |
| `.message_expiry_interval(d)` | `Duration` | `300s` |
| `.max_subscriptions(n)` | `usize` | `0`（不限） |
| `.max_topic_aliases(v)` | `u16` | `0` |
| `.limit_subscription(v)` | `bool` | `false` |
| `.delayed_publish(v)` | `bool` | `false` |
| `.tls_cross_certificate(v)` | `bool` | `false` |
| `.tls_cert(s)` | `Option<impl Into<String>>` | `None` |
| `.tls_key(s)` | `Option<impl Into<String>>` | `None` |
| `.tls_client_ca_certs(s)` | `Option<impl Into<String>>` | `None` |
| `.cert_cn_as_username(v)` | `bool` | `false` |
| `.cert_subject_dn_as_username(v)` | `bool` | `false` |
| `.collect_cert_info(v)` | `bool` | `false` |
| `.proxy_protocol(v)` | `bool` | `false` |
| `.proxy_protocol_timeout(d)` | `Duration` | `5s` |
| `.idle_timeout(d)` | `Duration` | `90s` |
| `.bind(self) -> Result<Listener>` | — | 绑定 TCP socket |
| `.bind_quic(self) -> Result<Listener>` | — | `#[cfg(feature = "quic")]` |

## `Listener` — 连接接受

```rust
listener.accept().await? -> Acceptor<S>
listener.accept_quic().await? -> Acceptor<QuinnBiStream>  // #[cfg(feature = "quic")]
listener.local_addr() -> Result<SocketAddr>
```

协议升级方法（消耗 `self`）：
```rust
listener.tcp()?  -> Listener  // 设为 TCP（含协议降级保护）
listener.tls()?  -> Listener  // #[cfg(feature = "tls")] 构建 TlsAcceptor
listener.ws()?   -> Listener  // #[cfg(feature = "ws")]
listener.wss()?  -> Listener  // #[cfg(feature = "tls")] #[cfg(feature = "ws")]
```

### `Acceptor<S>` — 单连接处理器

```rust
acceptor.tcp() -> Result<Dispatcher<S>>            // 创建 TCP 分发器
acceptor.tls() -> Result<Dispatcher<TlsStream<S>>> // #[cfg(feature = "tls")] TLS 握手
acceptor.remote_addr: SocketAddr                    // 客户端地址
```

### `ListenerType` 枚举

```rust
pub enum ListenerType {
    TCP,
    #[cfg(feature = "tls")]     TLS,
    #[cfg(feature = "ws")]      WS,
    #[cfg(feature = "tls")]     // + ws
                                WSS,
    #[cfg(feature = "quic")]    QUIC,
}
```

## `Dispatcher` → `MqttStream`

```rust
dispatcher.mqtt().await? -> MqttStream<S>
// 返回 MqttStream::V3(v3::MqttStream<S>) 或 MqttStream::V5(v5::MqttStream<S>)
```

## `MqttError`

```rust
pub enum MqttError {
    ServiceUnavailable,
    Closed,
    Intermittent,
    Wont(anyhow::Error),
}
```

## Feature 标志

| Feature | 依赖 | 说明 |
|---------|------|------|
| `tls` | `tokio-rustls`, `rustls` | TLS 传输 |
| `ws` | `tokio-tungstenite` | WebSocket 传输 |
| `quic` | `quinn` | QUIC (UDP) 传输 |

## 许可证

MIT OR Apache-2.0
