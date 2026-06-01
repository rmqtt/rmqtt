[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-p2p-messaging

[![crates.io](https://img.shields.io/crates/v/rmqtt-p2p-messaging.svg)](https://crates.io/crates/rmqtt-p2p-messaging)

Peer-to-peer messaging plugin for RMQTT. Enables direct client-to-client message delivery over MQTT topics.

## Overview

Provides a peer-to-peer messaging mechanism where one client can send messages directly to another specific client using a topic pattern that includes the target client ID. Messages are delivered only to the intended recipient client, not broadcast to all subscribers.

### Topic Format

The plugin supports three modes for P2P topic format:

| Mode | Topic Pattern | Description |
|------|--------------|-------------|
| `prefix` | `p2p/{clientid}/{topic}` | The `p2p` identifier is at the start of the topic |
| `suffix` | `{topic}/p2p/{clientid}` | The `p2p` identifier is at the end of the topic |
| `both` | Both patterns | Supports both prefix and suffix formats |

When a client publishes to a P2P topic (e.g., `p2p/target-client-id/sensors/temp`), the message is routed only to the client identified by `target-client-id`.

## Usage

Add the dependency to `Cargo.toml`:

```toml
rmqtt-p2p-messaging = "0.22"
```

Register the plugin in your broker startup code:

```rust
rmqtt_p2p_messaging::register(&scx, true, false).await?;
```

## Configuration

File: `rmqtt-p2p-messaging.toml`

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `mode` | String | `"prefix"` | P2P topic format: `"prefix"`, `"suffix"`, or `"both"` |

### Default Configuration

```toml
mode = "prefix"
```

### Example: Publishing with prefix mode

Client A wants to send a message to Client B:

- **Publish topic**: `p2p/Client-B/sensors/temp`
- **Content**: `{ "temperature": 23.5 }`
- **Result**: Only Client B receives the message on `sensors/temp`

### Example: Publishing with suffix mode

```toml
mode = "suffix"
```

- **Publish topic**: `sensors/temp/p2p/Client-B`
- **Content**: `{ "temperature": 23.5 }`
- **Result**: Only Client B receives the message

### Example: Publishing with both modes

```toml
mode = "both"
```

Both prefix and suffix topic formats are recognized:

- `p2p/Client-B/sensors/temp`
- `sensors/temp/p2p/Client-B`

## Dependencies

- `rmqtt` (feature `plugin`)

## License

MIT OR Apache-2.0
