[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-net

[![crates.io page](https://img.shields.io/crates/v/rmqtt-net.svg)](https://crates.io/crates/rmqtt-net)
[![docs.rs page](https://docs.rs/rmqtt-net/badge.svg)](https://docs.rs/rmqtt-net/latest/rmqtt_net)

MQTT server network layer — TCP, TLS via rustls, WebSocket via tokio-tungstenite, and QUIC via quinn.

## Exported items

```rust
pub use builder::{Builder, Listener, ListenerType};
pub use cert_extractor::TlsCertExtractor;    // trait
pub use error::MqttError;
pub use stream::{v3, v5, MqttStream};        // v3::MqttStream, v5::MqttStream
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

## `Builder` — fluent listener builder

All fields have `pub` visibility and fluent setter methods:

| Method | Params | Builder default |
|--------|--------|----------------|
| `new()` | — | `max_connections=1_000_000, max_handshaking_limit=1_000, max_packet_size=1MB, backlog=512, nodelay=false, reuseaddr=None, reuseport=None, allow_anonymous=true, min_keepalive=0, max_keepalive=65535, allow_zero_keepalive=true, keepalive_backoff=0.75, max_inflight=16, handshake_timeout=30s, send_timeout=10s, max_mqueue_len=1000, max_clientid_len=65535, max_qos_allowed=2, max_topic_levels=0, session_expiry_interval=7200s, max_session_expiry_interval=0, message_retry_interval=20s, message_expiry_interval=300s, max_subscriptions=0, shared_subscription=true, max_topic_aliases=0, ...` |
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
| `.max_topic_levels(n)` | `usize` | `0` (unlimited) |
| `.session_expiry_interval(d)` | `Duration` | `7200s` |
| `.max_session_expiry_interval(d)` | `Duration` | `0` (unlimited) |
| `.message_retry_interval(d)` | `Duration` | `20s` |
| `.message_expiry_interval(d)` | `Duration` | `300s` |
| `.max_subscriptions(n)` | `usize` | `0` (unlimited) |
| `.shared_subscription(v)` | `bool` | `true` |
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
| `.bind(self) -> Result<Listener>` | — | Binds TCP socket, returns Listener |
| `.bind_quic(self) -> Result<Listener>` | — | `#[cfg(feature = "quic")]` QUIC bind |

## `Listener` — connection accept

```rust
listener.accept().await? -> Acceptor<S>
listener.accept_quic().await? -> Acceptor<QuinnBiStream>  // #[cfg(feature = "quic")]
listener.local_addr() -> Result<SocketAddr>
```

Protocol upgrade methods (consumes `self`):
```rust
listener.tcp()?  -> Listener  // set type to TCP (downgrade guard)
listener.tls()?  -> Listener  // #[cfg(feature = "tls")] build TlsAcceptor
listener.ws()?   -> Listener  // #[cfg(feature = "ws")]
listener.wss()?  -> Listener  // #[cfg(feature = "tls")] #[cfg(feature = "ws")]
```

### `Acceptor<S>` — per-connection handler

```rust
acceptor.tcp() -> Result<Dispatcher<S>>           // create TCP dispatcher
acceptor.tls() -> Result<Dispatcher<TlsStream<S>>> // #[cfg(feature = "tls")] TLS handshake + dispatcher
acceptor.remote_addr: SocketAddr                   // client address
```

### `ListenerType` enum

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
// returns MqttStream::V3(v3::MqttStream<S>)  or  MqttStream::V5(v5::MqttStream<S>)
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

## Feature flags

| Feature | Deps | Description |
|---------|------|-------------|
| `tls` | `tokio-rustls`, `rustls` | TLS transport |
| `ws` | `tokio-tungstenite` | WebSocket transport |
| `quic` | `quinn` | QUIC (UDP) transport |

## License

MIT OR Apache-2.0
