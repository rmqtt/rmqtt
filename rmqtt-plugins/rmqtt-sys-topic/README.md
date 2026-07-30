[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-sys-topic

[![crates.io](https://img.shields.io/crates/v/rmqtt-sys-topic.svg)](https://crates.io/crates/rmqtt-sys-topic)

System topics plugin for RMQTT. Publishes broker metrics as `$SYS/` MQTT topics for monitoring.

## Overview

Periodically publishes broker statistics to the `$SYS/` topic tree, which clients can subscribe to for real-time monitoring. This provides visibility into broker health and operational metrics without external monitoring tools.

### Available Metrics

The plugin publishes various broker metrics under the `$SYS/` hierarchy, including:

- **Client statistics**: connected client counts, disconnected sessions
- **Message statistics**: published, received, and delivered message counts
- **Broker uptime**: time since broker started
- **Version information**: RMQTT version and build info

## Usage

Add the dependency to `Cargo.toml`:

```toml
rmqtt-sys-topic = "0.21"
```

Register the plugin in your broker startup code:

```rust
rmqtt_sys_topic::register(&scx, true, false).await?;
```

## Configuration

File: `rmqtt-sys-topic.toml`

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `publish_qos` | Integer | `1` | QoS level for `$SYS` messages: `0`, `1`, or `2` |
| `publish_interval` | String | `"1m"` | Interval between `$SYS` metric publications (e.g., `"30s"`, `"1m"`, `"5m"`) |
| `message_expiry_interval` | String | `"5m"` | Message expiry interval for `$SYS` messages (`0` = no expiry) |

### Default Configuration

```toml
publish_qos = 1
publish_interval = "1m"
message_expiry_interval = "5m"
```

## Dependencies

- `rmqtt` (feature `plugin`)

## License

MIT OR Apache-2.0
