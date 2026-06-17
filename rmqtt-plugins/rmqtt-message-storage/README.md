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
| `storage.ram.cache_max_count` | integer | `1_000_000` | Maximum cache entry count |
| `storage.ram.encode` | boolean | `true` | Enable message encoding |

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

## Dependencies

`rmqtt` (features: `plugin`, `msgstore`), redis (optional)

## License

MIT OR Apache-2.0
