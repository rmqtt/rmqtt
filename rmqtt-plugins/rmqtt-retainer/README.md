[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-retainer

[![crates.io](https://img.shields.io/crates/v/rmqtt-retainer.svg)](https://crates.io/crates/rmqtt-retainer)

Retained message storage plugin. Supports RAM, Sled (embedded), and Redis backends. Replaces the default retain engine with persistent storage.

## Overview

Intercepts the `BeforeStartup` hook to inject a persistent retain store. A background task periodically (every 10s) cleans expired retained messages. Only the Redis backend supports cluster mode.

## Usage

### Build

Add the dependency in `rmqttd/Cargo.toml` or enable via the `rmqtt-plugins` meta-crate:

```toml
# Direct dependency
rmqtt-retainer = { version = "0.21", features = ["ram"] }

# Or via meta-crate
rmqtt-plugins = { version = "0.21", features = ["retainer-ram"] }
```

Available feature flags for storage backends:

| Feature | Backend | Persistence | Cluster Support |
|---------|---------|-------------|-----------------|
| `ram` | In-memory HashMap | No (lost on restart) | No |
| `sled` | Sled embedded DB (disk) | Yes | No |
| `redis` | Redis remote store | Yes | Yes |

### Register

```rust
rmqtt_retainer::register(&scx, true, false).await?;
// or with explicit name:
rmqtt_retainer::register_named(&scx, "rmqtt-retainer", true, false).await?;
```

Parameters: `(scx, default_startup, immutable)`.

## Configuration

File: `rmqtt-retainer.toml` (in the plugin config directory). Loaded via `scx.plugins.load_config_default::<PluginConfig>("rmqtt-retainer")`.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `storage.type` | `string` | `"ram"` | Backend type: `ram`, `sled`, `redis` |
| `storage.sled.path` | `string` | `"/var/log/rmqtt/.cache/retain/{node}"` | Sled database path (supports `{node}` placeholder for node ID) |
| `storage.sled.cache_capacity` | `string` | `"3G"` | Sled cache capacity (e.g. `"3G"`, `"512MB"`) |
| `storage.redis.url` | `string` | `"redis://127.0.0.1:6379/"` | Redis connection URL |
| `storage.redis.prefix` | `string` | `"retain"` | Redis key prefix |
| `max_retained_messages` | `u64` | `0` (unlimited) | Maximum retained messages. After exceeding, existing messages can be replaced but new topics cannot be stored. |
| `max_payload_size` | `string` | `"1MB"` | Maximum payload size for retained messages. Exceeding this causes the message to be treated as a regular message. |
| `retained_message_ttl` | `string` | `"0m"` (no expiry) | TTL for retained messages. If not set, defaults to the message expiry interval. |

### Configuration Source

The plugin loads configuration via `scx.plugins.load_config_default::<PluginConfig>("rmqtt-retainer")`, supporting:

1. `{plugins.dir}/rmqtt-retainer.toml` (file, optional — falls back to defaults if missing)
2. `rmqtt_plugin_rmqtt_retainer_*` environment variables (maps TOML keys to env vars with underscore prefix)
3. Inline config via `ServerContext::plugins_config_map_add()`

### Example

```toml
# RAM mode (default)
storage.type = "ram"

# Sled mode (persistent, single-node)
storage.type = "sled"
storage.sled.path = "/var/log/rmqtt/.cache/retain/{node}"
storage.sled.cache_capacity = "3G"

# Redis mode (persistent, cluster-compatible)
storage.type = "redis"
storage.redis.url = "redis://127.0.0.1:6379/"
storage.redis.prefix = "retain"

# Limits
max_retained_messages = 10000
max_payload_size = "1MB"
retained_message_ttl = "24h"
```

## Dependencies

`rmqtt` (features: `plugin`, `retain`), `rmqtt-storage`, `serde`, `tokio`, `sled` (optional), `redis` (optional)

## License

MIT OR Apache-2.0
