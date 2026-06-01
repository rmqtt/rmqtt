[**English**](README.md) | [简体中文](README-CN.md)

# RMQTT-Server

[![crates.io page](https://img.shields.io/crates/v/rmqtt.svg)](https://crates.io/crates/rmqtt)
[![docs.rs page](https://docs.rs/rmqtt/badge.svg)](https://docs.rs/rmqtt/latest/rmqtt/)

Core MQTT broker library — session management, routing, hooks, plugins, and cluster coordination.

## Module structure

```
rmqtt/src/
├── lib.rs          — re-exports: rmqtt_codec as codec, rmqtt_net as net, rmqtt_utils as utils
│
├── acl.rs          — Access Control List types (ACLConfig, AclCheckFn, AuthInfo)
├── args.rs         — CommandArgs struct (node_id, plugins_default_startups, node_grpc_addrs, raft_peer_addrs, raft_leader_id)
├── context.rs      — ServerContext builder (fluent API: .node(), .task_exec_workers(), .plugins_config_dir(), etc.)
├── executor.rs     — Async task executor (wraps rust-box task-exec-queue)
├── extend.rs       — Extension points with RwLock-protected components (10 slots)
├── fitter.rs       — Topic filter matching engine
├── hook.rs         — Hook system (Hook trait, 10+ hook points: message_publish, client_keepalive, session_created, etc.)
├── inflight.rs     — In-flight message tracking (InInflight, OutInflight, OutInflightMessage)
├── node.rs         — Cluster node (Node::new(), Node::version(), Node::rustc_version(), gRPC server start)
├── queue.rs        — Message queue (Limiter, Policy, rate-limited queue)
├── router.rs       — Topic-based message router (publish, subscribe, unsubscribe, route to offline)
├── server.rs       — MqttServer (builder: .listener(), .build(), .start(); accept loop)
├── session.rs      — Session handling (~2400 lines: connect, disconnect, subscribe, publish, QoS flow)
├── shared.rs       — Shared subscriptions ($share/{group}/{topic})
├── topic.rs        — Topic parsing/validation (TopicFilter, parse_topic_filter, topic_size)
├── trie.rs         — Topic trie for subscription matching
├── types.rs        — Core types (~3000 lines: ConnectInfo, Publish, Packet, Reason, Id, SessionTx, etc.)
├── v3.rs           — MQTT v3.1.1 protocol handler
├── v5.rs           — MQTT v5.0 protocol handler
│
├── delayed.rs      — [feature: delayed] Delayed message publishing
├── grpc.rs         — [feature: grpc] gRPC inter-node communication
├── message.rs      — [feature: msgstore] Message storage subsystem
├── metrics.rs      — [feature: metrics] Metrics collection
├── plugin.rs       — [feature: plugin] Plugin trait and registration
├── retain.rs       — [feature: retain] Retained message storage
├── stats.rs        — [feature: stats] Runtime statistics
├── subscribe.rs    — [feature: auto-subscription|shared-subscription] Subscription services
```

## Feature flags

| Feature | Deps enabled | What it enables |
|---------|-------------|-----------------|
| `metrics` | rmqtt-macros/metrics | Metrics collection |
| `stats` | — | Runtime statistics tracking |
| `plugin` | rmqtt-macros/plugin | Plugin system |
| `grpc` | rust-box/handy-grpc, rust-box/mpsc | gRPC inter-node communication |
| `tls` | rmqtt-net/tls | TLS transport |
| `ws` | rmqtt-net/ws | WebSocket transport |
| `quic` | rmqtt-net/quic | QUIC transport |
| `delayed` | — | Delayed message publishing |
| `retain` | — | Retained message storage |
| `msgstore` | — | Message persistence |
| `shared-subscription` | — | Shared subscriptions ($share/) |
| `auto-subscription` | — | Auto-subscribe on connect |
| `limit-subscription` | — | Subscription limiting |
| `macros` | dep:rmqtt-macros | Enables both metrics + plugin |
| `full` | All of the above | All features |
| `debug` | — | Debug mode |
| `default` | (none) | Minimal build |

## Re-exports

```rust
pub use rmqtt_codec as codec;   // MQTT protocol codec
pub use rmqtt_net as net;       // Network layer (Builder, MqttStream, etc.)
pub use rmqtt_utils as utils;   // Utilities (Bytesize, NodeAddr, etc.)
pub use rmqtt_macros as macros; // [features: metrics|plugin] Derive macros
pub use net::{Error, Result};   // Re-exported error types
```

## Usage

```rust,no_run
use rmqtt::context::ServerContext;
use rmqtt::net::Builder;
use rmqtt::server::MqttServer;
use rmqtt::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let scx = ServerContext::new().build().await;

    MqttServer::new(scx)
        .listener(Builder::new().name("tcp").laddr("0.0.0.0:1883".parse()?).bind()?.tcp()?)
        .listener(Builder::new().name("ws").laddr("0.0.0.0:8080".parse()?).bind()?.ws()?)
        .build()
        .run()
        .await?;
    Ok(())
}
```

## Examples

See `rmqtt/examples/` for: `simple`, `simple_tls`, `simple_ws`, `simple_wss`, `simple_quic`, `multi`, `plugin`, `plugins`, `simple_quic_client`.

## License

MIT OR Apache-2.0
