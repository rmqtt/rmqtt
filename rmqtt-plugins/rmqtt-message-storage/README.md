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
| `backend_timeout` | string | `"15s"` | Timeout for storage I/O operations, channel sends, and circuit breaker per-operation timeout. `"0s"` = no timeout |

### Circuit Breaker

Protects the broker from blocking when the storage backend becomes unreachable.
The breaker uses a sliding window to track the call failure rate. When the failure
rate exceeds the threshold and the minimum number of calls has been reached, the
circuit trips to OPEN and all storage operations fast-fail. The circuit automatically
probes for recovery.

The per-operation timeout uses the `backend_timeout` setting (see Cleanup section above).
All other circuit-breaker parameters (failure rate threshold, sliding window, etc.) are
inherited from the global `[circuit_breaker]` section in `rmqtt.toml`.

#### State Machine

```
CLOSED ── failure_rate ≥ threshold (min_calls met) ──► OPEN ── wait_duration ──► HALF_OPEN
  ▲                                                                            │
  └────────────────── probe success ◄──────────────────────────────────────────┘
```

- **CLOSED**: normal operation; calls are tracked in the sliding window.
- **OPEN**: all operations fast-fail without touching the storage backend.
- **HALF_OPEN**: a limited number of probe requests are allowed; if they succeed the
  circuit closes, if any fail it re-opens.

## Dependencies

`rmqtt` (features: `plugin`, `msgstore`), redis (optional)

## License

MIT OR Apache-2.0
