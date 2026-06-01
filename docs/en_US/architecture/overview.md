[**English**](overview.md) | [简体中文](../../zh_CN/architecture/overview.md)

# RMQTT Architecture Overview

This document describes the internal architecture of the RMQTT MQTT broker, its components, module organization, and key design decisions.

---

## System Architecture

```mermaid
graph TB
    subgraph "Client Layer"
        MQTT[TCP Clients]
        TLS[TLS Clients]
        WS[WebSocket Clients]
        QUIC[QUIC Clients]
    end

    subgraph "Network Layer (rmqtt-net)"
        Builder[Builder API<br/>Listener Configuration]
        Listener[TCP/TLS/WS/WSS/QUIC<br/>Listeners]
        Acceptor[Acceptor<br/>Per-Connection Handler]
        Dispatcher[Protocol Dispatcher<br/>v3/v5 Version Negotiation]
    end

    subgraph "Protocol Layer (rmqtt-codec)"
        Codec[MqttCodec<br/>Encoder/Decoder]
        V3[v3::Codec<br/>MQTT 3.1.1]
        V5[v5::Codec<br/>MQTT 5.0]
        Version[VersionCodec<br/>Handshake Detection]
    end

    subgraph "Core Broker (rmqtt)"
        Server[MqttServer<br/>Server Lifecycle]
        Context[ServerContext<br/>Shared State]
        Session[Session Manager<br/>Connect/Subscribe/Publish]
        Router[Topic Router<br/>Trie-based Matching]
        Hook[Hook System<br/>Extension Points]
        Inflight[QoS Engine<br/>Inflight Tracking]
        Queue[Message Queue<br/>Rate Limiter]
        Executor[Task Executor<br/>Async Task Queue]
    end

    subgraph "Configuration (rmqtt-conf)"
        Settings[Settings<br/>Singleton Config]
        Options[Options<br/>CLI Parser]
        ListenerCfg[Listener Config<br/>Per-protocol Settings]
    end

    subgraph "Storage Plugins"
        Retain[Retainer<br/>RAM/Sled/Redis]
        MsgStore[Message Store<br/>RAM/Redis/Redis Cluster]
        SessionStore[Session Store<br/>Sled/Redis/Redis Cluster]
    end

    subgraph "Cluster Plugins"
        Raft[Raft Consensus<br/>Strong Consistency]
        Broadcast[Broadcast<br/>High Throughput]
        gRPC[gRPC Inter-node<br/>Communication]
    end

    MQTT & TLS & WS & QUIC --> Listener
    Listener --> Builder
    Listener -->|accept| Acceptor
    Acceptor -->|dispatch| Dispatcher
    Dispatcher -->|mqtt| Server

    Dispatcher -.-> Codec
    Codec -.-> V3 & V5 & Version

    Server --> Context
    Context --> Settings
    Settings --> Options & ListenerCfg

    Server --> Session
    Session --> Hook
    Session --> Inflight
    Session --> Router
    Router --> Queue
    Queue --> Inflight

    Session -.-> Retain & MsgStore & SessionStore
    Router -.-> Raft & Broadcast
    Raft & Broadcast -.-> gRPC
```

---

## Crate Organization

The workspace is organized into these layers:

### Layer 1: Foundation Crates

These crates have no dependency on other workspace crates:

| Crate | Path | Responsibility |
|-------|------|----------------|
| `rmqtt-utils` | `rmqtt-utils/` | Shared types (`Bytesize`, `NodeAddr`, `Counter`), serde helpers, timestamp/duration parsing |
| `rmqtt-macros` | `rmqtt-macros/` | Procedural macros: `#[derive(Metrics)]` for atomic counters, `#[derive(Plugin)]` for `PackageInfo` trait |
| `rmqtt-codec` | `rmqtt-codec/` | MQTT protocol encoder/decoder — v3.1, v3.1.1, v5.0 with version negotiation |

### Layer 2: Infrastructure Crates

Build on foundation crates:

| Crate | Path | Dependencies | Responsibility |
|-------|------|--------------|----------------|
| `rmqtt-net` | `rmqtt-net/` | `rmqtt-codec`, `rmqtt-utils` | Network layer: TCP/TLS/WS/QUIC listeners, connection accept, protocol dispatch |
| `rmqtt-conf` | `rmqtt-conf/` | `rmqtt-codec`, `rmqtt-net`, `rmqtt-utils`, `config` crate | Configuration management: TOML parsing, CLI args, listener config |

### Layer 3: Core Broker

| Crate | Path | Dependencies | Responsibility |
|-------|------|--------------|----------------|
| `rmqtt` | `rmqtt/` | All above + `rmqtt-net`, `rmqtt-codec`, `rmqtt-utils`, `rmqtt-macros` (optional), `rust-box`, `dashmap`, `tokio` | Core MQTT broker: session management, routing, hooks, plugins, clustering |

### Layer 4: Binaries

| Crate | Path | Responsibility |
|-------|------|----------------|
| `rmqttd` | `rmqtt-bin/` | Production binary: CLI parsing → config → plugin registration → server start |
| `mqtt_harness` | `rmqtt-test/` | Test harness: functional, stress, and chaos testing |

### Layer 5: Plugins

| Crate | Path | Responsibility |
|-------|------|----------------|
| `rmqtt-plugins` | `rmqtt-plugins/` | Meta-crate re-exporting all plugins behind feature flags |
| `rmqtt-*` | `rmqtt-plugins/rmqtt-*/` | 25 individual plugin crates |

---

## Core Module Architecture (rmqtt/src/)

```
rmqtt/src/
├── lib.rs           # Crate root, re-exports, module declarations
│
├── server.rs        # MqttServer — builder + accept loop + lifecycle
├── context.rs       # ServerContext — shared state builder
├── session.rs       # Session — per-client state machine (~2400 lines)
│
├── v3.rs            # MQTT v3.1.1 protocol handler
├── v5.rs            # MQTT v5.0 protocol handler
│
├── router.rs        # Topic-based message router
├── trie.rs          # Trie structure for subscription matching
├── topic.rs         # Topic filter parsing and validation
├── fitter.rs        # Topic filter matching engine
│
├── inflight.rs      # In-flight message tracking (QoS 1/2)
├── queue.rs         # Message queue with rate limiting
│
├── hook.rs          # Hook system — 10+ extension points
├── extend.rs        # Extension point storage (10 RwLock slots)
├── executor.rs      # Async task executor wrapper
│
├── types.rs         # Core data types (~3000 lines)
├── node.rs          # Cluster node coordination, gRPC server
│
├── acl.rs           # ACL types and trait definitions
│
├── args.rs          # Command-line argument struct
├── shared.rs        # Shared subscriptions ($share/)
│
├── delayed.rs       # [feature: delayed] Delayed publish
├── grpc.rs          # [feature: grpc] gRPC communication
├── message.rs       # [feature: msgstore] Message storage
├── metrics.rs       # [feature: metrics] Metrics collection
├── plugin.rs        # [feature: plugin] Plugin trait + registration
├── retain.rs        # [feature: retain] Retained messages
├── stats.rs         # [feature: stats] Runtime statistics
└── subscribe.rs     # [feature: *-subscription] Subscription helpers
```

---

## Session Lifecycle

```mermaid
stateDiagram-v2
    [*] --> Connecting: TCP/TLS/WS accepted
    
    state Connecting {
        [*] --> VersionProbe: read CONNECT packet
        VersionProbe --> v3: MQTT v3 detected
        VersionProbe --> v5: MQTT v5 detected
    }
    
    Connecting --> Authenticating: version negotiated
    
    state Authenticating {
        [*] --> CheckACL: ClientAuthenticate hook
        CheckACL --> Allowed: rule matches
        CheckACL --> Denied: no rule or deny
    }
    
    Authenticating --> Connected: CONNACK sent
    Authenticating --> [*]
    
    state Connected {
        [*] --> Subscribing: SUBSCRIBE received
        Subscribing --> Active: SUBACK sent
        
        Active --> Publishing: PUBLISH received
        Publishing --> Active: PUBACK or PUBREC
        
        Active --> Receiving: Message from router
        Receiving --> Active: Sent to client
        
        Active --> Unsubscribing: UNSUBSCRIBE received
        Unsubscribing --> Active: UNSUBACK sent
        
        Active --> Idle: No activity
        Idle --> Active: PINGREQ PINGRESP
    }
    
    Connected --> Disconnecting: DISCONNECT received
    Connected --> Disconnecting: Keepalive timeout
    Connected --> Disconnecting: Client disconnect
    
    Disconnecting --> Cleanup: store session if expired
    Disconnecting --> Cleanup: store offline messages
    Cleanup --> Terminated: cleanup complete
    Terminated --> [*]
```

---

## Hook System

The hook system is the primary extension mechanism. It provides 10+ interception points along the message processing pipeline.

### Hook Trait

```rust
#[async_trait]
pub trait Handler: Send + Sync {
    async fn hook(&self, param: &Type, acc: Option<()>) -> ReturnType;
}
```

### Hook Types

| Hook Type | Trigger | Handler Returns |
|-----------|---------|-----------------|
| `BeforeStartup` | Broker initialization | Continue |
| `ClientConnect` | CONNECT received | `(bool, Option<ConnAckReason>)` |
| `ClientAuthenticate` | Before CONNACK | `(bool, Option<ConnAckReason>)` |
| `ClientConnack` | CONNACK sent | Continue |
| `ClientConnected` | Session established | Continue |
| `ClientDisconnected` | Session ended | Continue |
| `ClientSubscribe` | SUBSCRIBE received | Continue |
| `ClientSubscribeCheckAcl` | Subscribe ACL check | `(bool, Option<SubscribeAclResult>)` |
| `ClientUnsubscribe` | UNSUBSCRIBE received | Continue |
| `MessagePublish` | PUBLISH received | `(bool, Option<MessagePublishResult>)` |
| `MessagePublishCheckAcl` | Publish ACL check | `(bool, Option<PublishAclResult>)` |
| `MessageDelivered` | Message sent to client | Continue |
| `MessageAcked` | Client acknowledged | Continue |
| `MessageDropped` | Message dropped | Continue |
| `SessionCreated` | Session created | Continue |
| `SessionTerminated` | Session destroyed | Continue |
| `SessionSubscribed` | Subscription added | Continue |
| `SessionUnsubscribed` | Subscription removed | Continue |
| `OfflineMessage` | Offline message stored | Continue |
| `GrpcMessageReceived` | Cross-node gRPC message | `(bool, Option<Vec<u8>>)` |

### Hook Registration Priority

Handlers can register with a priority. Lower values execute first. The `counter` plugin registers at `Priority::MAX` to ensure it runs last.

---

## Plugin System

### Plugin Trait

```rust
#[async_trait]
pub trait Plugin: PackageInfo + Send + Sync {
    async fn init(&mut self) -> Result<()>;         // Register hooks
    async fn get_config(&self) -> Result<Value>;     // Current config
    async fn load_config(&mut self) -> Result<()>;   // Runtime reload
    async fn start(&mut self) -> Result<()>;          // Activate hooks
    async fn stop(&mut self) -> Result<bool>;         // Deactivate
    async fn attrs(&self) -> Value;                   // Runtime attributes
    async fn send(&self, msg: Value) -> Result<Value>;// Inter-plugin message
}
```

### Plugin Lifecycle

```mermaid
sequenceDiagram
    participant App as rmqttd
    participant Plugin as Plugin Instance
    participant Hook as Hook System
    
    App->>Plugin: new(scx, name)
    Plugin->>Plugin: Load config from file
    Plugin-->>App: Instance
    
    App->>Plugin: init()
    Plugin->>Hook: register(Type::X, handler)
    Plugin-->>App: Ok(())
    
    App->>Plugin: start()
    Plugin->>Hook: self.register.start()
    Plugin-->>App: Ok(())
    
    Note over App,Plugin: Runtime: hooks fire automatically
    
    App->>Plugin: load_config()
    Plugin->>Plugin: Reload TOML config
    Plugin-->>App: Ok(())
    
    App->>Plugin: stop()
    Plugin->>Hook: self.register.stop()
    Plugin-->>App: bool (true=stoppable, false=core)
```

### Registration Pattern

Each plugin crate follows the same registration pattern via the `register!` macro:

```rust
// Generated by register!(MyPlugin::new)
pub async fn register_named(
    scx: &ServerContext,
    name: &'static str,
    default_startup: bool,
    immutable: bool,
) -> Result<()>;

pub async fn register(
    scx: &ServerContext,
    default_startup: bool,
    immutable: bool,
) -> Result<()>;
```

---

## Message Flow

```mermaid
sequenceDiagram
    participant Pub as Publishing Client
    participant Broker as RMQTT
    participant Sub as Subscribed Client
    participant Store as Storage Plugin
    participant Cluster as Cluster Plugin

    Pub->>Broker: CONNECT
    Broker->>Broker: Version detect (v3/v5)
    Broker->>Broker: ClientAuthenticate hook
    Broker-->>Pub: CONNACK

    Pub->>Broker: SUBSCRIBE (topic: "sensor/#")
    Broker->>Broker: SubscribeCheckAcl hook
    Broker->>Broker: Add to subscription trie
    Broker-->>Pub: SUBACK

    Sub->>Broker: SUBSCRIBE (topic: "sensor/#")
    Broker-->>Sub: SUBACK

    Pub->>Broker: PUBLISH (topic: "sensor/temp", payload: 23.5)
    Broker->>Broker: PublishCheckAcl hook
    Broker->>Broker: MessagePublish hook
    Broker->>Broker: Match subscriptions in trie
    
    par Concurrent Delivery
        Broker->>Sub: Deliver message
        Broker->>Sub: MessageDelivered hook
    and Cluster Forwarding
        Broker->>Cluster: Forward to cluster nodes
        Cluster->>Cluster: Raft consensus or broadcast
        Cluster-->>Broker: Acknowledged
    end
    
    alt Client Offline
        Broker->>Store: Store offline message
        Store-->>Broker: Stored
    end

    Sub->>Broker: PUBACK (QoS 1) or PUBREC (QoS 2)
    Broker->>Broker: MessageAcked hook
    
    Pub->>Broker: DISCONNECT
    Broker->>Broker: ClientDisconnected hook
    Broker->>Broker: SessionTerminated hook
```

---

## Configuration Loading Order

```mermaid
flowchart LR
    A["/etc/rmqtt/rmqtt.{toml,json}"] --> C[Config Builder]
    B["/etc/rmqtt.{toml,json}"] --> C
    D["./rmqtt.{toml,json}"] --> C
    E["-f / --config path"] --> C
    F["RMQTT_* Env Vars"] --> C
    C --> G[Settings Singleton]
```

Plugin configs are loaded separately from `{plugins.dir}/{name}.toml` with `rmqtt_plugin_{name}_*` environment variables.

---

## Feature Flags

The core broker (`rmqtt`) uses Cargo feature flags to conditionally compile optional functionality:

| Feature | What it enables | Key Dependencies |
|---------|----------------|------------------|
| `default` | Minimal build (no extras) | — |
| `metrics` | Metrics collection | `rmqtt-macros/metrics` |
| `stats` | Runtime statistics | — |
| `plugin` | Plugin system | `rmqtt-macros/plugin` |
| `grpc` | Inter-node gRPC | `rust-box/handy-grpc`, `msgstore` |
| `tls` | TLS transport | `rmqtt-net/tls` |
| `ws` | WebSocket transport | `rmqtt-net/ws` |
| `quic` | QUIC transport | `rmqtt-net/quic` |
| `delayed` | Delayed publish | — |
| `retain` | Retained messages | — |
| `msgstore` | Message storage | — |
| `shared-subscription` | `$share/` groups | — |
| `auto-subscription` | Auto-subscribe on connect | — |
| `limit-subscription` | Subscription limits | — |
| `full` | All above | — |

---

## Key Design Decisions

### 1. Zero Unsafe Code

The entire codebase enforces `#![deny(unsafe_code)]`. All concurrency is handled through safe abstractions (`tokio::sync`, `DashMap`, `Arc`).

### 2. Lock Strategy

- **Hot paths**: `DashMap` (lock-free concurrent hash maps) for subscription trie and session lookups
- **Async contexts**: `tokio::sync::RwLock` for config and shared state (never `std::sync::RwLock` in async code)
- **Fine-grained**: `std::sync::Mutex` only for small, synchronous critical sections

### 3. No Panic in Production

- `unwrap()` / `expect()` only in test code
- All `Result` and `Option` are handled via `?` or pattern matching
- No `panic!` / `todo!` / `unreachable!` in production paths

### 4. Plugin Isolation

Each plugin is a separate crate with optional compilation. The meta-crate (`rmqtt-plugins`) re-exports all plugins behind feature flags, ensuring zero overhead for unused functionality.

### 5. Codec Architecture

The MQTT codec uses a state machine pattern:
1. `VersionCodec` detects protocol version from the CONNECT packet
2. Switches to `v3::Codec` or `v5::Codec` for the remainder of the session
3. Both implement `tokio_util::codec::Encoder/Decoder` for async streaming

---

## License

MIT OR Apache-2.0
