[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-message-storage

[![crates.io](https://img.shields.io/crates/v/rmqtt-message-storage.svg)](https://crates.io/crates/rmqtt-message-storage)

Message persistence plugin. Stores unexpired messages for offline clients.

## Usage

```toml
[dependencies]
rmqtt-message-storage = { version = "0.21", features = ["ram"] }
# or: features = ["redis", "redis-cluster"]
```

```rust
rmqtt_message_storage::register(&scx, true, false).await?;
```

## Configuration

File: `rmqtt-message-storage.toml`

### Storage Type

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `storage.type` | string | `"ram"` | Backend storage type: `ram`, `redis`, or `redis-cluster` |

### RAM Backend

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `storage.ram.cache_capacity` | string | `"3G"` | In-memory cache capacity |
| `storage.ram.cache_max_count` | integer | `1_000_000` | Maximum cache entry count (unlimited) |
| `storage.ram.encode` | boolean | `false` | Enable message encoding |
| `storage.ram.queue_max` | integer | `300000` | Maximum task queue backlog (back-pressure limit) |

### Redis Backend

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `storage.redis.url` | string | `"redis://127.0.0.1:6379/"` | Redis server URL |
| `storage.redis.prefix` | string | `"message-{node}"` | Key prefix (`{node}` = node ID placeholder) |

### Redis Cluster Backend

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `storage.redis-cluster.urls` | array of string | `["redis://127.0.0.1:6380/", ...]` | Redis cluster node URLs |
| `storage.redis-cluster.prefix` | string | `"message-{node}"` | Key prefix |

### Cleanup

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `cleanup_count` | integer | `5000` | Expired messages cleaned per cycle |
| `timeout` | string | `"5s"` | Timeout for storage I/O operations. `"0s"` = no timeout |

### Circuit Breaker

Protects the broker from blocking when the Redis backend becomes unreachable.
When the circuit is OPEN, store/mark_forwarded/get return immediately without
touching Redis. The circuit automatically probes for recovery.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `circuit_breaker_enabled` | boolean | `true` | Enable circuit breaker |
| `circuit_failure_threshold` | integer | `10` | Consecutive failures before tripping to OPEN |
| `circuit_reset_timeout` | string | `"15s"` | Wait duration in OPEN before probing (HALF_OPEN) |
| `circuit_half_open_success_threshold` | integer | `3` | Consecutive probe successes to close the circuit |

#### State Machine

```
CLOSED ── failure_count ≥ threshold ──► OPEN ── timeout ──► HALF_OPEN
  ▲                                                               │
  └────────────── success × threshold ◄───────────────────────────┘
```

- **CLOSED**: normal operation, failures are counted.
- **OPEN**: all operations skip Redis immediately; workers drain backlog without waiting.
- **HALF_OPEN**: probe requests are allowed; if enough succeed the circuit closes,
  if any fails it re-opens.

## Dependencies

`rmqtt` (features: `plugin`, `msgstore`), redis (optional)

## License

MIT OR Apache-2.0
