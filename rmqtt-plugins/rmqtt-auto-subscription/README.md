[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-auto-subscription

[![crates.io](https://img.shields.io/crates/v/rmqtt-auto-subscription.svg)](https://crates.io/crates/rmqtt-auto-subscription)

Auto-subscription plugin for RMQTT. Automatically subscribes clients to predefined topic filters when they connect.

## Overview

When a client connects, the plugin subscribes them to configured topic filters without requiring the client to send SUBSCRIBE packets. This is useful for:

- Forcing clients to receive specific system or telemetry topics
- Ensuring all clients have a baseline set of subscriptions
- Reducing client-side logic by pre-configuring subscriptions on the broker

Each subscription entry supports configurable QoS, No Local, Retain as Published, and Retain Handling options (MQTTv5 properties).

### Variable Interpolation

Topic filter expressions can use `${clientid}` to represent the client ID and `${username}` to represent the client username, allowing dynamic per-client subscription topics.

## Usage

Add the dependency to `Cargo.toml`:

```toml
rmqtt-auto-subscription = "0.22"
```

Register the plugin in your broker startup code:

```rust
rmqtt_auto_subscription::register(&scx, true, false).await?;
```

## Configuration

File: `rmqtt-auto-subscription.toml`

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `subscribes` | Array of tables | *(see below)* | List of topic subscriptions to apply on client connect |

### Subscription Entry Fields

Each entry in the `subscribes` array supports:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `topic_filter` | String | *(required)* | Topic filter to subscribe to. Supports `${clientid}` and `${username}` placeholders |
| `qos` | Integer | `1` | QoS level: `0`, `1`, or `2` |
| `no_local` | Boolean | `false` | MQTTv5: Do not send messages published by the client itself |
| `retain_as_published` | Boolean | `false` | MQTTv5: Keep the RETAIN flag when forwarding |
| `retain_handling` | Integer | `0` | MQTTv5: Retain handling policy (`0`, `1`, or `2`) |

### Default Subscriptions

```toml
subscribes = [
    { topic_filter = "x/+/#", qos = 1, no_local = false, retain_as_published = false, retain_handling = 0 },
    { topic_filter = "foo/${clientid}/#", qos = 1, no_local = false, retain_as_published = false, retain_handling = 0 },
    { topic_filter = "iot/${username}/#", qos = 1 }
]
```

### Placeholder Variables

| Placeholder | Description |
|-------------|-------------|
| `${clientid}` | Replaced with the connecting client's client ID |
| `${username}` | Replaced with the connecting client's username |

### Example: Device-Specific Subscriptions

```toml
subscribes = [
    # All clients subscribe to a common telemetry topic
    { topic_filter = "telemetry/#", qos = 1 },

    # Each client subscribes to its own device-specific control topic
    { topic_filter = "control/${clientid}/#", qos = 2 },

    # Subscribe to user-specific configuration topics
    { topic_filter = "config/${username}/#", qos = 1 }
]
```

## Dependencies

- `rmqtt` (feature `plugin`)

## License

MIT OR Apache-2.0
