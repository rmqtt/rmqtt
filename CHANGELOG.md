# Changelog

All notable changes to RMQTT are documented in this file.

## [0.23.0] - 2026-08-09

### New Features

- **Unified Circuit Breaker**: Integrated a sliding-window circuit breaker (`CircuitBreakerConfig`) across all storage plugins — `rmqtt-retainer`, `rmqtt-message-storage`, and `rmqtt-session-storage`. When the storage backend failure rate exceeds the threshold, the circuit opens and all operations fast-fail, preventing cascading failures. Configurable via `circuit_breaker.*` settings in each plugin's TOML.
- **gRPC Client Circuit Breaker**: Added per-peer-node circuit breaker to `GrpcClient`, covering `send_message`, `quick_send_message`, `notify`, and `quick_notify`. Configurable via `node_grpc_circuit_breaker_enabled`, `node_grpc_circuit_failure_threshold`, `node_grpc_circuit_reset_timeout`, and `node_grpc_circuit_half_open_success_threshold` in cluster plugin configs.
- **Retainer Cluster Synchronization**: Implemented cluster-wide retain message synchronization with two modes — `Full` (broadcast full payload, used by ram/sled) and `TopicOnly` (broadcast topic name only, used by redis). New `RetainStorage` trait methods: `retain_sync_mode()` and `sync_retain_topic()`.
- **gRPC Quick Path**: Added `quick_send_message` / `quick_notify` fast paths that bypass request-queue-full checks for higher-priority operations. All HTTP API cross-node gRPC calls now use the quick path.
- **Retainer In-Memory Topic Trie**: Built a `RetainTree` index in memory on startup for O(1) exact-topic lookups and fast wildcard matching, replacing the previous SCAN+MATCH approach.
- **Retainer Batch Storage**: Messages are collected into a channel and processed in batches via `batch_insert` / `batch_remove`, controlled by `batch_messages_limit`.
- **Rate Counter**: Added `rmqtt-utils::RateCounter` (lock-free `AtomicU64`) for tracking message processing throughput. Enabled by default via the `rate-counter` feature.
- **Message Storage Timeout**: Added configurable `backend_timeout` for storage I/O operations, channel sends, and circuit breaker per-operation timeout in `rmqtt-message-storage`. Unified the previous separate `timeout` and `backend_timeout` fields into a single `backend_timeout` (default: `"15s"`).
- **Cluster Broadcast Exec Queues**: Added `exec` and `forwards_exec` `TaskExecQueue` to `rmqtt-cluster-broadcast` for queue back-pressure management and busyness detection, aligning with `rmqtt-cluster-raft`.
- **Retainer Enabled by Default**: `rmqtt-retainer` is now included in `plugins.default_startups` by default.
- **HTTP API Feature Support Query**: Added `GET /api/v1/features` and `GET /api/v1/features/{id}` to `rmqtt-http-api`. They report the support state of six features (`retain`, `message_storage`, `session_storage`, `delayed`, `shared_subscription`, `auto_subscription`) per node. The cluster-wide response includes a consistency summary (`consistent` / `conflicts` / `nodes`) and emits a `features inconsistent across cluster` warning log when nodes disagree.
- **HTTP API Retained Messages Query**: Added `GET /api/v1/retains` to `rmqtt-http-api` to query retained messages with `topic_filter` / `offset` / `limit` parameters. The full pagination path (`topic_filter=#`) is served from the storage layer with `remaining_ttl`; filtered queries paginate in memory. Payload is base64-encoded.
- **Dashboard Enhancements**: Added a retained messages page (`#/retains`) with pagination and payload preview/detail dialog, a dedicated "Feature Support" tab with a per-node feature matrix and cluster consistency alert, dual-tab (abnormal / non-subscriber) switching on the message-drop trend panel, a client detail page, and an i18n-aware custom datetime picker. The Dashboard SPA can be served from an external directory via `dashboard_static_dir` for hot-swapping without recompiling.
- **TCP Keepalive on Accepted Connections** (issue #465): Accepted MQTT/TCP connections now enable `SO_KEEPALIVE` by default, so the kernel's `net.ipv4.tcp_keepalive_*` settings (Linux) / registry values (Windows) take effect and dead peers behind cellular/CGNAT NAT black holes are probed and reclaimed (previously the option was never set and connections piled up as ESTABLISHED/FIN-WAIT-1). Configurable via `listener.<proto>.<name>.tcp_keepalive` — `false` (disabled) / `true` (default, enabled with OS probe defaults). The socket option is set with a plain `setsockopt(SO_KEEPALIVE)` (`socket2::SockRef`), avoiding the blocking `WSAIoctl(SIO_KEEPALIVE_VALS)` path on Windows that stalled the tokio worker threads under high connection concurrency. Regression tests added in `rmqtt-test` (`functional_v5`).
- **Stats/Metrics History Persistence**: Added a complete history subsystem to `rmqtt-http-api` — Stats and Metrics are snapshotted periodically (default `flush_interval = "5s"`), cached in an in-memory LRU, and asynchronously persisted to a configurable backend (`storage.type`: `redb` / `sled` / `redis` / `redis-cluster`) with TTL-based expiration (`history_retention`, default `"7d"`). Expired entries are discarded during warmup and removed from storage. New endpoints `GET /api/v1/stats/history` and `GET /api/v1/metrics/history` support cross-node cluster-wide queries via gRPC, with results merged by timestamp. A 30s recovery loop retries failed entries and a global `UNPERSISTED_COUNT` tracks pending writes.
- **Dashboard Embedded via rust-embed**: The `rmqtt-dashboard` SPA is now compiled into the `rmqtt-http-api` binary at build time (`rust-embed`), so the dashboard is served at `http://127.0.0.1:6060/` with zero configuration and no filesystem dependency. The optional `dashboard_static_dir` setting still allows a filesystem override for development hot-reloading.
- **Dashboard Retained Message Deletion**: Added the ability to delete individual retained messages from the retained messages page (`#/retains`), backed by the new HTTP API endpoint `DELETE /api/v1/retains?topic={topic}` (rejects wildcard topics, returns 404 when no retained message exists, uses MQTT empty-payload retained publish semantics, and propagates the deletion to all cluster peers via `retain_set_broadcast`). The dashboard shows a confirmation dialog with optimistic UI updates.

### Bug Fixes

- **QoS 2 Exactly-Once on Replayed PUBLISH** (issue #456): A replayed QoS 2 PUBLISH (same Packet Identifier, `DUP=1`, before the PUBREL exchange completes) is now answered with PUBREC and **no longer delivered to the subscriber a second time** (`[MQTT-4.3.3-10]`). Implemented via an `InInflight::exist` check on the inbound inflight set, plus a new `client_publish_duplicate` metrics counter.
- **Inflight Packet-ID Space Isolation on Session Resume**: Fixed a QoS 2 session-resume bug where transferred inflight messages kept their old packet-ids (1..N) while the new session's `OutInflight` allocator restarted at 1, so a concurrently delivered stored message could be assigned the same id and silently overwritten by `push_back` (`HashMap::insert`), permanently destroying its QoS 2 state (no resend, no ack hook, possible loss). The allocator is now advanced past the transferred id range via a new `OutInflight::advance_next_id()` before any concurrent delivery path can allocate, and `send_rerelease` gained a defensive existence check.
- **Session Resume Reforwards All Inflight Messages** (issue #456): On session transfer/resume, every inflight message is now reforwarded regardless of status — `UnComplete` messages were previously skipped, so an owed PUBREL was lost. A re-sent PUBREL now always acknowledges Success instead of the previous `PacketIdNotFound` choice (`[MQTT-4.4.0-1]`).
- **CONNACK Reason Code for Empty ClientId**: A CONNECT with a zero-length ClientId while CleanStart (v5) / CleanSession (v3.1.1) is 0 is now rejected with the spec-mandated reason codes — v5 `0x85` Client Identifier not valid (`[MQTT-3.1.3-8]`) and v3.1.1 `0x02` Identifier Rejected (`[MQTT-3.1.3-6]`) — instead of the previous generic 0x88 / 0x03.
- **Will Retain Rejected When Retain Unavailable** (issue #457): A CONNECT whose Will Message has Will Retain = 1 is now rejected with CONNACK reason `0x9A` (Retain not supported) when the server advertises Retain Available = 0 (`[MQTT-3.2.2-13]`); previously such connections were accepted with reason 0x00.
- **Cluster Sync Handles MessageReply::Success**: `MessageReply::Success` responses received during cluster retain synchronization and message loading were treated as errors and logged at `warn!` level. Both `rmqtt-cluster-broadcast` and `rmqtt-cluster-raft` now handle them explicitly (`debug!` level, empty result, end-of-sync), reducing production log noise.

### Refactoring

- **Circuit Breaker Simplification**: Simplified `CircuitBreaker` in `rmqtt-utils` — consolidated configuration into `CircuitBreakerConfig` with sliding-window semantics (failure rate, slow call detection, per-operation timeout).
- **Message Storage Refactor**: Removed `merge_on_read` and `TaskExecQueue` from `rmqtt-message-storage`; added `with_timeout` wrapper for storage operations; introduced async callback support and back-pressure limits. Unified `timeout` and `backend_timeout` into a single `backend_timeout` field.
- **Session Storage Improvements**: Added detailed timing metrics for rebuild operations; improved init timing and timeout handling.
- **Error Logging Normalization**: Normalized error logging across the workspace to use `Display` instead of `Debug` formatting.
- **Cluster Retain Exec Queue**: Added a dedicated `retainer_exec` `TaskExecQueue` in `rmqtt-cluster-raft` for retain operations, separating from the main `exec` queue.
- **Unified Server Error Handling**: `rmqtt-bin` startup logic was split into `main()` + `run()`; listener bind failures now propagate through the `anyhow` error chain with the listener address included, and are logged once in `main()` before exiting (previously logged redundantly without address context). Also filled in the empty error messages on `Listener::accept()` / `accept_quic()` in `rmqtt-net`.

### Test Improvements

- **MQTT Spec-Conformance Coverage**: Expanded `rmqtt-test` to systematically cover the MQTT 3.1, 3.1.1 and 5.0 specifications with positive, negative and boundary cases — the three functional suites grew from ~100 to **174 cases**, all passing (v3: 47, v3.1.1: 64, v5: 62 + 1 intentional skip). New modules cover protocol errors (SUBSCRIBE QoS 3, reserved flag bits, second CONNECT), keepalive, last will, QoS 2 conformance (`qos2_conformance_v3/v311/v5`), retain edge cases, wildcards, and CONNACK capability advertisement.
- **Per-Case Broker Config Switching**: Test cases can now declare their required broker config via `TestCase::broker_config()` and are split at suite-build time into `{suite}@{config}` sub-suites (e.g. `functional_v5@retain-disabled`, `functional_v5@tcp-keepalive`, `functional_v5@pubrel-collision`); the scheduler restarts the broker to switch configs only at suite boundaries. All test broker configs are self-contained under `rmqtt-test/configs/`, and `rmqttd` is always started with an explicit `-f` config.
- **QoS 2 Regression Suites**: Added single-node `qos2_pubrel_resume_collision` (functional_v5) and a new cluster end-to-end `functional_v5_cluster` suite (`qos2_pubrel_resume_collision_cluster`, two manually started nodes) — the cluster suite reproduced the bug 3/3 rounds before the fix and passes 3/3 after. Chaos broker-restart tests now SKIP in `--no-broker` mode instead of failing.
- **Test Harness Fixes**: Fixed a broker child-process leak on failure exit (the managed broker is now killed before `std::process::exit`), fixed `clippy::type_complexity` in suite splitting, and added `TestResult::note` / `TestContext::guard_retain_required` so retain-dependent tests skip with a note when the `rmqtt-retainer` plugin is not loaded.

### Dependency Upgrades

- `rmqtt-net` 0.3.5 → **0.4.0** (`Builder::tcp_keepalive()`)
- `rmqtt-conf` 0.3.5 → **0.4.0** (new `tcp_keepalive` listener option, `bool`)
- `rmqtt-storage` 0.10.2 → **0.11.1** (history storage backend for `rmqtt-http-api`)
- Docker base images: Alpine 3.22.4 → **3.24.1** (amd64) / arm64v8/alpine 3.22.4 → **3.24.1** (aarch64)

### Configuration Changes

- `rmqtt-retainer.toml`: Circuit breaker config changed from flat fields (`circuit_breaker_enabled`, `circuit_failure_threshold`, `circuit_reset_timeout`, `circuit_half_open_success_threshold`) to nested `circuit_breaker.*` sliding-window format. `retained_message_ttl` and `batch_messages_limit` are now uncommented by default.
- `rmqtt-message-storage.toml`: `storage.ram.encode` default changed from `true` to `false`. Added `circuit_breaker.*` section. Unified `timeout` and `backend_timeout` into a single `backend_timeout` field (default: `"15s"`).
- `rmqtt-session-storage.toml`: Added `circuit_breaker.*` section.
- `rmqtt-cluster-broadcast.toml` / `rmqtt-cluster-raft.toml`: Added gRPC circuit breaker settings (`node_grpc_circuit_breaker_enabled`, etc.). `rmqtt-cluster-raft.toml`: `node_grpc_client_timeout` default changed from `"60s"` to `"10s"`. `raft.snapshot_interval` default changed from `"600s"` to `"300s"`.
- `rmqtt-http-api.toml`: Added optional `[storage]` section for Stats/Metrics history persistence (`storage.type` = `redb` / `sled` / `redis` / `redis-cluster`), plus `flush_interval` (default `"5s"`) and `history_retention` (default `"7d"`). The `dashboard_static_dir` setting is now commented out by default — the dashboard is embedded in the binary via rust-embed; uncomment to serve from a filesystem directory for development.
- `rmqtt-retainer.toml`: Default storage type switched from `ram` to `sled`.
- `rmqtt.toml`: `rmqtt-retainer` is included in `plugins.default_startups` by default.

---

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
