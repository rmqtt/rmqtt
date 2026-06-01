[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-plugins

[![crates.io page](https://img.shields.io/crates/v/rmqtt-plugins.svg)](https://crates.io/crates/rmqtt-plugins)
[![docs.rs page](https://docs.rs/rmqtt-plugins/badge.svg)](https://docs.rs/rmqtt-plugins/latest/rmqtt_plugins)

[RMQTT](https://github.com/rmqtt/rmqtt) MQTT Broker 的插件集合元 crate。每个插件通过 feature 标志条件编译重新导出（`#![deny(missing_docs)]`）。

## 重新导出的模块（共 24 个）

### 核心插件（7 个）

| 模块 | Feature 标志 |
|--------|-------------|
| `acl` | `acl` |
| `retainer` | `retainer` / `retainer-ram` / `retainer-sled` / `retainer-redis` |
| `http_api` | `http-api` |
| `counter` | `counter` |
| `auth_http` | `auth-http` |
| `auth_jwt` | `auth-jwt` |
| `auto_subscription` | `auto-subscription` |

### 桥接插件（9 个）

| 模块 | Feature 标志 |
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

注意：`Cargo.toml` 中定义了 `bridge-ingress-nats` feature，但 `lib.rs` 中**没有**对应的 re-export。

### 存储插件（2 个）

| 模块 | Feature 标志 |
|--------|-------------|
| `message_storage` | `message-storage` / 各后端子 feature |
| `session_storage` | `session-storage` / 各后端子 feature |

### 功能插件（4 个）

| 模块 | Feature 标志 |
|--------|-------------|
| `sys_topic` | `sys-topic` |
| `topic_rewrite` | `topic-rewrite` |
| `web_hook` | `web-hook` |
| `p2p_messaging` | `p2p-messaging` |

### 集群插件（2 个）

| 模块 | Feature 标志 |
|--------|-------------|
| `cluster_raft` | `cluster-raft` |
| `cluster_broadcast` | `cluster-broadcast` |

## 使用方式

每个重新导出的模块提供 `register_named()` 函数：

```rust
use rmqtt_plugins;

rmqtt_plugins::acl::register_named(&scx, "rmqtt-acl", true, false).await?;
rmqtt_plugins::http_api::register_named(&scx, "rmqtt-http-api", true, false).await?;
```

## 许可证

MIT OR Apache-2.0
