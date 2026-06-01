# Changelog

All notable changes to RMQTT are documented in this file.

## [0.22.0] - 2026-05

### Major Changes

- **Serialization migration**: Migrated from `bincode` to `postcard` across the entire workspace for improved performance and reduced binary size. **Note**: Raft log state must be cleared when upgrading from 0.21.x due to format change.
- **CLI framework migration**: Migrated from `structopt` to `clap v4` for modern argument parsing with better error messages and auto-completion support.
- **Logging ecosystem migration**: Replaced `slog` with the `tracing` ecosystem (`tracing-subscriber`, `tracing-appender`) for structured, async-aware logging with file rotation and env-filter support.
- **Feature flag cleanup**: Removed unused `bridge-ingress-nats` re-export from `rmqtt-plugins` lib.rs (feature still exists in Cargo.toml).

### New Features

- **Bridge Origin plugin**: Added `rmqtt-bridge-origin` plugin to identify bridge client connections by client_id markers. Stores origin in `session.extra_attrs` for anti-loop and routing decisions.
- **TLS Certificate Subject DN as Username**: Added `cert_subject_dn_as_username` listener option alongside existing `cert_cn_as_username`. Useful when multiple CAs are trusted on the same listener.
- **Certificate Info Collection**: Added `collect_cert_info` listener option to conditionally extract TLS certificate metadata.
- **Client Certificate Authentication**: Added `tls_client_ca_certs` and `tls_cross_certificate` options for mutual TLS authentication.
- **Offline Message Webhook**: Added `offline_message` event support to the webhook plugin.
- **Client-level ACL Management**: Added per-client ACL rule management in `rmqtt-acl` plugin.
- **Advanced MQTT v5 Tests**: Comprehensive v5 feature tests including topic aliases, subscription identifiers, request/response, flow control.

### Dependency Upgrades

| Dependency | Old | New | Scope |
|-----------|-----|-----|-------|
| `tokio` | 1.40 | 1.52 | Workspace |
| `reqwest` | 0.12 | 0.13 | Workspace |
| `prometheus` | 0.13 | 0.14 | rmqtt-core |
| `rdkafka` | ~0.36 | 0.38 | Bridge Kafka |
| `rdkafka-sys` | — | pinned | Bridge Kafka |
| `salvo` | 0.76 | 0.90 | HTTP API |
| `async-nats` | 0.38 | 0.49 | Bridge NATS |
| `clap` | 3.x (structopt) | 4.x | CLI |
| `postcard` | — | added | Workspace (replaces bincode) |
| `tracing` | — | added | Workspace (replaces slog) |

### Other Changes

- Bumped `rmqtt-conf` to 0.3.5
- Bumped `rmqtt-macros` to 0.1.2
- Bumped `rmqtt-net` to 0.3.5 (removed linger setting)
- Upgraded Alpine base images to latest stable for Docker builds
- Optimized Docker build context (from 11.4GB to 119MB)
- Added GitHub CI workflow for Linux builds
- HTTP API: added startup synchronization and improved reload handling
- Improved HTTP API hot-reload: old server shuts down after new one starts

### Documentation

- Added comprehensive module-level doc comments across all crates
- Added/improved doc comments for all `.rs` files across workspace
- Updated CLI usage examples for rmqtt-test
- Created bilingual README files for all sub-crates and plugins
- Added bridge-origin documentation (`.toml` config and usage docs)

### Test Improvements

- Added comprehensive MQTT v5 feature tests and enhanced v5 client API
- Added advanced functional tests for MQTT v311 and v5 features
- Added missing functional, stress, and chaos test modules
- Added rmqtt-test to workspace members
- Simplified `max_packet_size` enforcement test
- Formatted CLI test arrays for better readability

---

## [0.21.0] - 2026-04

### New Features

- **Test Harness (rmqtt-test)**: New crate providing industrial-grade test harness with functional, stress, and chaos test suites. Five suite types covering MQTT 3.1, 3.1.1, 5.0, load testing, and fault injection.
- **Topic Rewrite Plugin**: Added `rmqtt-topic-rewrite` for flexible topic filter and topic name remapping.
- **P2P Messaging Plugin**: Added `rmqtt-p2p-messaging` for direct client-to-client message delivery.
- **HTTP API Metrics**: Added Prometheus metrics endpoint integration. View at `/api/v1/metrics`.
- **Shared Subscription Improvements**: Enhanced `$share/{group}/{topic}` subscription handling.

### Dependency Upgrades

- Upgraded `tokio` to 1.44
- Upgraded multiple workspace dependencies to latest compatible versions
- Upgraded Docker base images

### Fixes

- Fixed clippy warnings across all crates
- Fixed Docker build errors related to outdated dependencies
- Fixed subscription matching logic edge cases

---

## [0.20.0] - 2026-03

### New Features

- **NATS Bridging**: Added both ingress and egress NATS bridge plugins (`rmqtt-bridge-ingress-nats`, `rmqtt-bridge-egress-nats`).
- **ReductStore Bridge**: Added egress bridge for ReductStore time-series database.
- **Webhook Offline Messages**: Added `offline_message` event to webhook plugin.
- **Cluster HTTP API**: Enhanced HTTP API with cluster-wide operations via gRPC forwarding.

### Dependency Upgrades

- Upgraded `rdkafka` to 0.38 with pinned `rdkafka-sys`
- Improved Kafka delivery status logging

### Fixes

- Docker build improvements (reduced context size, fixed compile errors)
- Fixed warning about redundant message collection iterator usage

---

## [0.19.1] - 2026-02

### New Features

- **TLS Certificate Info Collection**: Added configurable `collect_cert_info` option for TLS listeners.
- **Propagate Certificate Info to Auth Events**: Certificate metadata now available during authentication hook.

### Fixes

- Suppressed clippy `large_err` warnings in raft store
- Fixed feature flag configuration for TLS
- Enabled `tls` feature for `rmqtt-net` dependency in `rmqtt-conf`

---

## [0.19.0] - 2026-01

### New Features

- **Client Certificate Authentication**: Added `tls_client_ca_certs` and `tls_cross_certificate` options for mutual TLS authentication.
- **Separate Client CA Bundle**: TLS now supports separate CA certificates for client authentication vs server verification.
- **Client-level ACL Management**: Added per-client ACL rule management in `rmqtt-acl` plugin.
- **Pulsar Bridge**: Added Pulsar ingress/egress bridge plugins.

### Changes

- Improved TLS configuration flexibility with separate CA trust anchors
- Enhanced ACL rule management API

---

## [0.18.0] - 2025-12

### New Features

- **Kafka Bridging**: Added Kafka ingress/egress bridge plugins (`rmqtt-bridge-ingress-kafka`, `rmqtt-bridge-egress-kafka`).
- **Webhook Plugin**: Added `rmqtt-web-hook` for HTTP-based event notifications.
- **Sys Topic Plugin**: Added `rmqtt-sys-topic` for `$SYS/` system metrics publishing.
- **Auto Subscription Plugin**: Added `rmqtt-auto-subscription` for auto-subscribing clients on connect.
- **Plugin System Maturity**: Stabilized plugin registration API with `register!` macro and `PackageInfo` trait.

### Changes

- Refactored MQTT codec (inspired by ntex-mqtt)
- Improved hook system with priority-based handler registration

---

## [0.17.0] - 2025-10

### New Features

- **Raft Clustering**: Production-ready `rmqtt-cluster-raft` plugin with configurable compression, health checks, and auto-exit.
- **Broadcast Clustering**: `rmqtt-cluster-broadcast` plugin for high-throughput eventual consistency.
- **Configuration Hot-Reload**: HTTP API plugin supports restartless config reload via graceful server swap.
- **Session/Message Storage**: Added `rmqtt-session-storage` (Sled/Redis) and `rmqtt-message-storage` (RAM/Redis) plugins.

---

## [0.16.0] - 2025-08

### New Features

- **MQTT v5.0 Protocol Support**: Complete implementation including:
  - Session Expiry, Message Expiry
  - Topic Aliases, Subscription Identifiers
  - User Properties, Request/Response
  - Flow Control, Server Keep Alive
  - Assigned Client ID, Maximum Packet Size
- **Retained Message Storage**: `rmqtt-retainer` plugin with RAM, Sled, and Redis backends.
- **HTTP API Plugin**: Initial REST API for broker management.
- **ACL Plugin**: File-based ACL rule engine.

---

## [0.15.0] - 2025-06

### Major Changes

- **Plugin System**: Introduced modular plugin architecture with `#[derive(Plugin)]` and hook-based extension.
- **Codec Rewrite**: MQTT encoding/decoding rewritten with inspiration from ntex-mqtt. Zero-copy, version-negotiating codec.
- **Feature Flag Restructure**: Modular feature flags replacing monolithic builds.
- **Rustls TLS Backend**: Migrated from native-tls to rustls for cross-platform TLS support.

---

## [0.13.0] and earlier

Earlier versions relied on maintained forks of `ntex` and `ntex-mqtt` as dependencies.
