[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-delayed

[![crates.io](https://img.shields.io/crates/v/rmqtt-delayed.svg)](https://crates.io/crates/rmqtt-delayed)

Delayed message publishing plugin. Implements MQTT delayed delivery via the `$delayed/<interval>/<topic>` topic prefix, backed by an in-memory priority queue.

## Overview

Intercepts PUBLISH packets whose topic starts with `$delayed/<interval>/` on the session publish path:

- **Topic parsing**: The `$delayed/<interval>/` prefix is stripped and the trigger interval (integer, in seconds) is extracted. The message is then scheduled for the real target topic. If the interval is not an integer, the PUBLISH is rejected with an error.
- **Per-node priority queue**: Pending messages are held in a trigger-time ordered `BinaryHeap` (in-memory, node-local).
- **Background forwarding**: A background task polls the queue every 500 ms and forwards expired messages through the normal routing path. The task exits within one tick after the sender is dropped (plugin stop/unload) and flushes pending messages according to `publish_immediate` (see Enablement Semantics below).
- **Capacity control**: The number of pending delayed messages is limited by `publish_max`. The overflow decision is made by the plugin itself — `publish_immediate` selects forwarding immediately or dropping.

### Enablement Semantics

Loading the plugin enables the delayed delivery feature (`/api/v1/features` reports `delayed: true`); unloading it restores the core placeholder. Pending messages receive a final handling on unload according to `publish_immediate`: they are **forwarded immediately** when `true`, or **dropped** through the `message_dropped` hook (reason `DelayedPublishRefused`) when `false`. Messages are in-memory only and are lost on node restart.

## Usage

### Enable

Uncomment `rmqtt-delayed` in the `plugins.default_startups` list of `rmqtt.toml`:

```toml
plugins.default_startups = [
    #"rmqtt-acl",
    #"rmqtt-auth-http",
    #"rmqtt-cluster-broadcast",
    #"rmqtt-cluster-raft",
    "rmqtt-shared-subscription",
    "rmqtt-delayed",        # <- enable
    "rmqtt-http-api"
]
```

Or add the dependency in `rmqttd/Cargo.toml` / enable via the `rmqtt-plugins` meta-crate:

```toml
# Direct dependency
rmqtt-delayed = { version = "0.24" }

# Or via meta-crate
rmqtt-plugins = { version = "0.24", features = ["delayed"] }
```

### Register

```rust
rmqtt_delayed::register(&scx, true, false).await?;
```

Parameters: `(scx, default_startup, immutable)`. When embedded via `rmqttd`, startup is controlled by the `package.metadata.plugins` entry in `rmqtt-bin/Cargo.toml` plus `plugins.default_startups` / `plugins.disabled_default_startups` in `rmqtt.toml` (`rmqtt-delayed` is **not** a default startup).

## Topic Format

```
$delayed/<interval>/<topic>
```

| Part | Description |
|------|-------------|
| `$delayed` | Fixed prefix that marks a delayed publish |
| `<interval>` | Delay in seconds, integer (e.g. `60`, `3600`) |
| `<topic>` | The real target topic; subscribers match against `<topic>` to receive the message |

### Examples

```text
# Deliver to a/b/c after 60 seconds
$delayed/60/a/b/c

# Deliver to x/y after 1 hour
$delayed/3600/x/y
```

When the message is forwarded, subscribers of `a/b/c` / `x/y` receive it as a regular publish.

## Configuration

File: `rmqtt-delayed.toml` (in the plugin config directory). Loaded via `scx.plugins.read_config_default::<PluginConfig>("rmqtt-delayed")` and hot-reloadable on plugin reload.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `publish_max` | `usize` | `100_000` | Maximum number of pending delayed messages per node. |
| `publish_immediate` | `bool` | `true` | Behavior when the limit is reached: `true` — forward immediately as a regular message; `false` — drop the message (fires the `message_dropped` hook with reason `DelayedPublishRefused`). The same switch also governs how pending messages are flushed when the plugin is stopped/unloaded (see Enablement Semantics). |

### Configuration Source

1. `{plugins.dir}/rmqtt-delayed.toml` (file, optional — falls back to defaults if missing)
2. `rmqtt_plugin_rmqtt_delayed_*` environment variables (maps TOML keys to env vars with underscore prefix)
3. Inline config via `ServerContext::plugins_config_map_add()`

### Example

```toml
# Maximum number of pending delayed messages per node
#publish_max = 100_000

# Behavior when publish_max is exceeded:
# true  - message will be forwarded immediately as a regular message
# false - message will be dropped (message_dropped hook, reason
#         DelayedPublishRefused)
#publish_immediate = true
```

> **Migration note**: These two options previously lived in `rmqtt.toml` as `mqtt.delayed_publish_max` / `mqtt.delayed_publish_immediate`. They were moved into the plugin config (renamed to `publish_max` / `publish_immediate`); leftover keys in `rmqtt.toml` are ignored.

## HTTP API

Requires the `rmqtt-http-api` plugin. Pending delayed messages can be inspected cluster-wide:

| Endpoint | Description |
|----------|-------------|
| `GET /api/v1/delayed_publishs` | Query pending delayed publishes (metadata only) with optional `topic_filter` / `offset` / `limit`. Cluster-wide; results are merged and sorted by trigger time (oldest first, topic as tiebreaker). The `$delayed/<interval>/` prefix is stripped in returned topics. Response: `{ "items": [...], "has_more": bool }`. |
| `GET /api/v1/delayed_publishs/detail` | Fetch one pending delayed publish with its full payload by composite key `node_id` / `topic` / `expired_time` / `client_id`. Returns 404 once the message has been fired. |

Plugin runtime attributes (visible via the plugin attrs API) report the current pending count: `{"pending": <n>}`.

## Limitations

- **In-memory only**: Pending delayed messages are not persisted. They are lost on node restart; on plugin stop/unload they are flushed according to `publish_immediate` (forwarded immediately, or dropped via the `message_dropped` hook).
- **Node-local**: Messages are scheduled on the node that accepted the publish; there is no cross-node synchronization of the queue. In a cluster, the HTTP API aggregates views across nodes, but delivery is still performed by the originating node.
- **Fixed 500 ms tick**: Actual delivery time may be up to ~500 ms later than the scheduled trigger time.

## Dependencies

`rmqtt` (features: `plugin`, `stats`, `delayed`), `tokio`, `async-trait`, `log`, `serde`, `serde_json`, `anyhow`

## License

MIT OR Apache-2.0
