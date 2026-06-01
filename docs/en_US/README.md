[**English**](README.md) | [简体中文](../zh_CN/README.md)

# RMQTT Documentation

Welcome to the RMQTT documentation. This index provides a structured overview of all available documentation resources.

## Quick Links

| Resource | Description |
|----------|-------------|
| [GitHub Repository](https://github.com/rmqtt/rmqtt) | Source code, issues, discussions |
| [crates.io](https://crates.io/crates/rmqtt) | Published crate versions |
| [docs.rs](https://docs.rs/rmqtt/latest/rmqtt/) | API reference (library mode) |

---

## Architecture

| Document | Description |
|----------|-------------|
| [Architecture Overview](architecture/overview.md) | System architecture, crate layers, core modules, session lifecycle |
| [Plugin System](../architecture/overview.md#plugin-system) | Plugin trait, lifecycle, registration pattern |
| [Hook System](../architecture/overview.md#hook-system) | All 18 hook types, handler registration, priority |
| [Message Flow](../architecture/overview.md#message-flow) | End-to-end publish/subscribe flow with diagrams |

---

## Getting Started

| Document | Description |
|----------|-------------|
| [Installation Guide](install.md) | Install via Docker, binary package, or source build |
| [MQTT Protocol Support](mqtt-protocol.md) | Supported MQTT versions, features, and configuration |

---

## Configuration

| Document | Description |
|----------|-------------|
| [Configuration Reference](https://github.com/rmqtt/rmqtt/blob/master/rmqtt.toml) | Full configuration file example |
| [Permission List](perm-list.md) | Available permissions and their meanings |

---

## Features

### Authentication & Access Control

| Document | Description |
|----------|-------------|
| [ACL (Access Control List)](acl.md) | File-based ACL rule engine |
| [HTTP Authentication](auth-http.md) | External HTTP API authentication |
| [JWT Authentication](auth-jwt.md) | JSON Web Token validation |

### Message Storage & Delivery

| Document | Description |
|----------|-------------|
| [Retained Messages](retainer.md) | Persistent retained message storage |
| [Offline Messages](offline-message.md) | Message storage for disconnected clients |
| [Session Storage](store-session.md) | Session state persistence |
| [Message Storage](store-message.md) | Unexpired message persistence |

### Clustering

| Document | Description |
|----------|-------------|
| [Raft Cluster](cluster-raft.md) | Strongly consistent clustering via Raft consensus |
| [Benchmark Testing](benchmark-testing.md) | Performance benchmarks (1M clients, 150K msg/s) |

### Bridges

| Document | Direction |
|----------|-----------|
| [MQTT Bridge - Ingress](bridge-ingress-mqtt.md) | Remote MQTT → Local |
| [MQTT Bridge - Egress](bridge-egress-mqtt.md) | Local → Remote MQTT |
| [Kafka Bridge - Ingress](bridge-ingress-kafka.md) | Kafka → Local |
| [Kafka Bridge - Egress](bridge-egress-kafka.md) | Local → Kafka |
| [Pulsar Bridge - Ingress](bridge-ingress-pulsar.md) | Pulsar → Local |
| [Pulsar Bridge - Egress](bridge-egress-pulsar.md) | Local → Pulsar |
| [NATS Bridge - Ingress](bridge-ingress-nats.md) | NATS → Local |
| [NATS Bridge - Egress](bridge-egress-nats.md) | Local → NATS |
| [ReductStore Bridge - Egress](bridge-egress-reductstore.md) | Local → ReductStore |
| [Bridge Origin](bridge-origin.md) | Bridge client identification |

### Management & Monitoring

| Document | Description |
|----------|-------------|
| [HTTP API](http-api.md) | RESTful management API reference |
| [WebHook](web-hook.md) | HTTP event notifications |
| [System Topics](sys-topic.md) | `$SYS/` monitoring metrics |

### Topic Features

| Document | Description |
|----------|-------------|
| [Topic Rewrite](topic-rewrite.md) | Topic filter and name rewriting |
| [Auto Subscription](auto-subscription.md) | Automatic subscription on connect |
| [P2P Messaging](p2p-messaging.md) | Direct client-to-client messaging |

---

## Crate Documentation

Each crate has its own bilingual README:

| Crate | Description | README |
|-------|-------------|--------|
| `rmqtt` | Core broker library | [README](../rmqtt/README.md) |
| `rmqttd` | Binary entry point | [README](../rmqtt-bin/README.md) |
| `rmqtt-codec` | MQTT protocol codec | [README](../rmqtt-codec/README.md) |
| `rmqtt-net` | Network layer (TCP/TLS/WS/QUIC) | [README](../rmqtt-net/README.md) |
| `rmqtt-conf` | Configuration management | [README](../rmqtt-conf/README.md) |
| `rmqtt-utils` | Shared utilities | [README](../rmqtt-utils/README.md) |
| `rmqtt-macros` | Procedural macros | [README](../rmqtt-macros/README.md) |
| `rmqtt-test` | Test harness | [README](../rmqtt-test/README.md) |
| `rmqtt-plugins` | Plugin collection meta-crate | [README](../rmqtt-plugins/README.md) |

### Plugin Crate READMEs

| Category | Plugin | Description |
|----------|--------|-------------|
| **Auth** | [rmqtt-acl](../rmqtt-plugins/rmqtt-acl/README.md) | File-based ACL |
| | [rmqtt-auth-http](../rmqtt-plugins/rmqtt-auth-http/README.md) | HTTP authentication |
| | [rmqtt-auth-jwt](../rmqtt-plugins/rmqtt-auth-jwt/README.md) | JWT authentication |
| **Storage** | [rmqtt-retainer](../rmqtt-plugins/rmqtt-retainer/README.md) | Retained message store |
| | [rmqtt-message-storage](../rmqtt-plugins/rmqtt-message-storage/README.md) | Message persistence |
| | [rmqtt-session-storage](../rmqtt-plugins/rmqtt-session-storage/README.md) | Session persistence |
| **Cluster** | [rmqtt-cluster-raft](../rmqtt-plugins/rmqtt-cluster-raft/README.md) | Raft consensus |
| | [rmqtt-cluster-broadcast](../rmqtt-plugins/rmqtt-cluster-broadcast/README.md) | Broadcast cluster |
| **Bridge** | [rmqtt-bridge-*-mqtt](../rmqtt-plugins/rmqtt-bridge-egress-mqtt/README.md) | MQTT bridge |
| | [rmqtt-bridge-*-kafka](../rmqtt-plugins/rmqtt-bridge-egress-kafka/README.md) | Kafka bridge |
| | [rmqtt-bridge-*-pulsar](../rmqtt-plugins/rmqtt-bridge-egress-pulsar/README.md) | Pulsar bridge |
| | [rmqtt-bridge-*-nats](../rmqtt-plugins/rmqtt-bridge-egress-nats/README.md) | NATS bridge |
| | [rmqtt-bridge-egress-reductstore](../rmqtt-plugins/rmqtt-bridge-egress-reductstore/README.md) | ReductStore bridge |
| | [rmqtt-bridge-origin](../rmqtt-plugins/rmqtt-bridge-origin/README.md) | Bridge origin identification |
| **API** | [rmqtt-http-api](../rmqtt-plugins/rmqtt-http-api/README.md) | HTTP REST API |
| | [rmqtt-web-hook](../rmqtt-plugins/rmqtt-web-hook/README.md) | Webhook notifications |
| | [rmqtt-sys-topic](../rmqtt-plugins/rmqtt-sys-topic/README.md) | System topics |
| **Utility** | [rmqtt-counter](../rmqtt-plugins/rmqtt-counter/README.md) | Metrics counters |
| | [rmqtt-auto-subscription](../rmqtt-plugins/rmqtt-auto-subscription/README.md) | Auto-subscription |
| | [rmqtt-topic-rewrite](../rmqtt-plugins/rmqtt-topic-rewrite/README.md) | Topic rewrite |
| | [rmqtt-p2p-messaging](../rmqtt-plugins/rmqtt-p2p-messaging/README.md) | P2P messaging |

---

## Development

| Resource | Description |
|----------|-------------|
| [Contributing Guide](../CONTRIBUTING.md) | Contribution guidelines |
| [Changelog](../CHANGELOG.md) | Release history |
| [Developer Getting Started](development/getting-started.md) | Dev environment setup, build, workflow |
| [Testing Guide](development/testing.md) | Test layers, running tests, writing tests |
| [Test Report](testing-report.md) | Interoperability results and benchmark data |
| [Plugin Development Guide](development/plugin-development.md) | Creating plugins, hook system, lifecycle |
| [FQA](https://github.com/rmqtt/rmqtt/issues) | Issues and discussions |

---

## Reference

| Resource | Description |
|----------|-------------|
| [HTTP API Reference](reference/http-api.md) | Complete REST API endpoint reference (36 endpoints) |

---

## License

RMQTT is licensed under [MIT](https://opensource.org/licenses/MIT) or [Apache 2.0](https://www.apache.org/licenses/LICENSE-2.0) at your option.
