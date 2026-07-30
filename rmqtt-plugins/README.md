[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-plugins

[![crates.io page](https://img.shields.io/crates/v/rmqtt-plugins.svg)](https://crates.io/crates/rmqtt-plugins)
[![docs.rs page](https://docs.rs/rmqtt-plugins/badge.svg)](https://docs.rs/rmqtt-plugins/latest/rmqtt_plugins)

Plugin collection meta-crate for the [RMQTT](https://github.com/rmqtt/rmqtt) MQTT broker. Each plugin is conditionally re-exported via feature flags (`#![deny(missing_docs)]`).

## Re-exported modules (24 total)

### Core plugins (7)

| Module | Feature flag |
|--------|-------------|
| `acl` | `acl` |
| `retainer` | `retainer` / `retainer-ram` / `retainer-sled` / `retainer-redis` |
| `http_api` | `http-api` |
| `counter` | `counter` |
| `auth_http` | `auth-http` |
| `auth_jwt` | `auth-jwt` |
| `auto_subscription` | `auto-subscription` |

### Bridge plugins (9)

| Module | Feature flag |
|--------|-------------|
| `bridge_egress_kafka` | `bridge-egress-kafka` |
| `bridge_ingress_kafka` | `bridge-ingress-kafka` |
| `bridge_egress_mqtt` | `bridge-egress-mqtt` |
| `bridge_ingress_mqtt` | `bridge-ingress-mqtt` |
| `bridge_egress_pulsar` | `bridge-egress-pulsar` |
| `bridge_ingress_pulsar` | `bridge-ingress-pulsar` |
| `bridge_egress_nats` | `bridge-egress-nats` |
| `bridge_egress_reductstore` | `bridge-egress-reductstore` |
| `bridge_origin` | `bridge-origin` |

Note: `bridge-ingress-nats` feature exists in `Cargo.toml` but is **not** re-exported from `lib.rs`.

### Storage plugins (2)

| Module | Feature flag |
|--------|-------------|
| `message_storage` | `message-storage` / `message-storage-ram` / `message-storage-redis` / `message-storage-redis-cluster` |
| `session_storage` | `session-storage` / `session-storage-sled` / `session-storage-redis` / `session-storage-redis-cluster` |

### Utility plugins (4)

| Module | Feature flag |
|--------|-------------|
| `sys_topic` | `sys-topic` |
| `topic_rewrite` | `topic-rewrite` |
| `web_hook` | `web-hook` |
| `p2p_messaging` | `p2p-messaging` |

### Cluster plugins (2)

| Module | Feature flag |
|--------|-------------|
| `cluster_raft` | `cluster-raft` |
| `cluster_broadcast` | `cluster-broadcast` |

## Usage

Each re-exported module provides a `register_named()` function:

```rust
use rmqtt_plugins;

rmqtt_plugins::acl::register_named(&scx, "rmqtt-acl", true, false).await?;
rmqtt_plugins::http_api::register_named(&scx, "rmqtt-http-api", true, false).await?;
```

## License

MIT OR Apache-2.0
