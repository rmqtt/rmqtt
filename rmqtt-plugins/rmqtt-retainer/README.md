[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-retainer

[![crates.io](https://img.shields.io/crates/v/rmqtt-retainer.svg)](https://crates.io/crates/rmqtt-retainer)

Retained message storage plugin. Supports RAM, Sled (embedded), and Redis backends. Replaces the default retain engine with persistent storage.

All backends support both single-node and cluster mode. In cluster mode, local backends (ram, sled) use full retain synchronization, while Redis uses lightweight topic-only synchronization.

## Overview

Intercepts the `BeforeStartup` hook to inject a persistent retain store. A background task periodically (every 10s) cleans expired retained messages.

### Architecture (v0.22.0+)

- **In-Memory Topic Trie Index**: Built on startup by scanning stored retain messages. Provides fast wildcard matching for subscription queries, replacing the previous SCAN+MATCH approach.
- **Batch Storage**: Messages are collected and processed in batches (`batch_insert` / `batch_remove`), controlled by `batch_messages_limit`.
- **Rate Counter**: Tracks processing throughput (enabled by default via the `rate-counter` feature).
- **RetainSyncMode**:
  - `Full`: Full retain payload broadcast (ram, sled).
  - `TopicOnly`: Lightweight topic-name sync only (Redis with shared storage).
- **Circuit Breaker**: Built-in storage failure detection with automatic fast-fail degradation and recovery.
  See configuration below.

## Usage

### Build

Add the dependency in `rmqttd/Cargo.toml` or enable via the `rmqtt-plugins` meta-crate:

```toml
# Direct dependency
rmqtt-retainer = { version = "0.22", features = ["ram"] }

# Or via meta-crate
rmqtt-plugins = { version = "0.22", features = ["retainer-ram"] }
```

Available feature flags for storage backends:

| Feature | Backend | Persistence | Cluster Support |
|---------|---------|-------------|-----------------|
| `ram` | In-memory HashMap | No (lost on restart) | Yes |
| `sled` | Sled embedded DB (disk) | Yes | Yes |
| `redis` | Redis remote store | Yes | Yes |

Additional features:

| Feature | Description |
|---------|-------------|
| `rate-counter` | Enable throughput rate counter (enabled by default) |

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
| `batch_messages_limit` | `usize` | `500` | Maximum number of messages per batch store operation |
| `circuit_breaker.failure_rate_threshold` | `f64` | `0.25` | Failure rate threshold (0.0–1.0) for tripping the circuit to OPEN |
| `circuit_breaker.sliding_window_type` | `string` | `"TimeBased"` | Sliding window type: `CountBased` or `TimeBased` |
| `circuit_breaker.sliding_window_size` | `usize` | `20` | Sliding window size (number of calls) |
| `circuit_breaker.sliding_window_duration` | `string` | `"45s"` | Sliding window duration (TimeBased mode only) |
| `circuit_breaker.minimum_number_of_calls` | `usize` | `10` | Minimum calls before the breaker can trip |
| `circuit_breaker.wait_duration_in_open` | `string` | `"30s"` | Duration in OPEN state before transitioning to HALF_OPEN |
| `circuit_breaker.slow_call_duration_threshold` | `string` | `"2s"` | Slow call duration threshold |
| `circuit_breaker.slow_call_rate_threshold` | `f64` | `1.0` | Slow call rate threshold (1.0 = disabled) |
| `circuit_breaker.operation_timeout` | `string` | `"8s"` | Per-operation timeout (`"0s"` to disable) |

### Configuration Source

The plugin loads configuration via `scx.plugins.load_config_default::<PluginConfig>("rmqtt-retainer")`, supporting:

1. `{plugins.dir}/rmqtt-retainer.toml` (file, optional — falls back to defaults if missing)
2. `rmqtt_plugin_rmqtt_retainer_*` environment variables (maps TOML keys to env vars with underscore prefix)
3. Inline config via `ServerContext::plugins_config_map_add()`

### Example

```toml
# RAM mode (default)
storage.type = "ram"

# Sled mode (persistent)
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
batch_messages_limit = 500

# Circuit breaker (sliding-window model, uses defaults if commented out)
#circuit_breaker.failure_rate_threshold = 0.25
#circuit_breaker.sliding_window_type = "TimeBased"
#circuit_breaker.sliding_window_size = 20
#circuit_breaker.sliding_window_duration = "45s"
#circuit_breaker.minimum_number_of_calls = 10
#circuit_breaker.wait_duration_in_open = "30s"
#circuit_breaker.slow_call_duration_threshold = "2s"
#circuit_breaker.slow_call_rate_threshold = 1.0
#circuit_breaker.operation_timeout = "8s"
```

## Dependencies

`rmqtt` (features: `plugin`, `retain`), `rmqtt-storage`, `serde`, `tokio`, `sled` (optional), `redis` (optional)

## License

MIT OR Apache-2.0
