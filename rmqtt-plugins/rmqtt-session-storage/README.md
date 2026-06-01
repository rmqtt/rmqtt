[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-session-storage

[![crates.io](https://img.shields.io/crates/v/rmqtt-session-storage.svg)](https://crates.io/crates/rmqtt-session-storage)

Session persistence plugin. Stores client session state using Sled, Redis, or Redis Cluster backends.

## Usage

```toml
[dependencies]
rmqtt-session-storage = "0.22"
```

```rust
rmqtt_session_storage::register(&scx, true, false).await?;
```

## Configuration

File: `rmqtt-session-storage.toml`

### Storage Type

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `storage.type` | string | `"sled"` | Backend storage type: `sled`, `redis`, or `redis-cluster` |

### Sled Backend

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `storage.sled.path` | string | `"/var/log/rmqtt/.cache/session/{node}"` | Sled database file path (`{node}` = node ID placeholder) |
| `storage.sled.cache_capacity` | string | `"3G"` | Sled cache capacity |

### Redis Backend

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `storage.redis.url` | string | `"redis://127.0.0.1:6379/"` | Redis server URL |
| `storage.redis.prefix` | string | `"session-{node}"` | Key prefix (`{node}` = node ID placeholder) |

### Redis Cluster Backend

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `storage.redis-cluster.urls` | array of string | `["redis://127.0.0.1:6380/", ...]` | Redis cluster node URLs |
| `storage.redis-cluster.prefix` | string | `"session-{node}"` | Key prefix |

## Dependencies

`rmqtt` (feature `plugin`), `sled`, `redis`, `tokio`

## License

MIT OR Apache-2.0
