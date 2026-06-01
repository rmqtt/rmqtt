# RMQTT Bridge Egress MQTT

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-egress-mqtt.svg)](https://crates.io/crates/rmqtt-bridge-egress-mqtt)
[![license](https://img.shields.io/crates/l/rmqtt-bridge-egress-mqtt.svg)](https://github.com/rmqtt/rmqtt/blob/master/LICENSE)

## Overview

`rmqtt-bridge-egress-mqtt` is an RMQTT plugin that forwards local MQTT publish messages to one or more remote MQTT brokers. It acts as an egress bridge, subscribing to local topics and republishing matching messages to the configured remote brokers with topic transformation support.

Key features:
- Support for MQTT v4 (3.1.1) and v5.0 protocol versions
- TLS/SSL encrypted connections to remote brokers
- Configurable multiple bridge instances
- Topic transformation with `${local.topic}` variable substitution
- Message QoS and retain flag override
- Last will message support
- Automatic reconnection

## Usage

### Add Dependency

Add the following to your `Cargo.toml`:

```toml
[dependencies]
rmqtt-bridge-egress-mqtt = { version = "*", features = ["plugin"] }
```

### Register Plugin

Add the plugin to your RMQTT node configuration:

```toml
[[plugins]]
name = "rmqtt-bridge-egress-mqtt"
register = true
```

Optionally, specify a custom configuration file path:

```toml
[[plugins]]
name = "rmqtt-bridge-egress-mqtt"
register = true
config_file = "/path/to/rmqtt-bridge-egress-mqtt.toml"
```

## Configuration

The configuration file uses TOML format. Below is the complete list of configuration options.

### Bridge Instance (`[[bridges]]`)

Each bridge instance defines a connection to a remote MQTT broker.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enable` | Boolean | `false` | Whether to enable this bridge instance |
| `name` | String | - | Unique bridge name identifier |
| `client_id_prefix` | String | - | Client ID prefix for the MQTT connection |
| `server` | String | - | Remote MQTT broker address. Formats: `tcp://host:port` or `tls://host:port` |
| `root_cert` | String | - | CA signed server certificate PEM file path (for TLS) |
| `client_cert` | String | - | Client certificate PEM file path (for TLS) |
| `client_key` | String | - | Client private key PEM file path (for TLS) |
| `username` | String | - | Username for remote broker authentication. Supports `${ENV:VAR}` environment variable substitution |
| `password` | String | - | Password for remote broker authentication. Supports `${ENV:VAR}` environment variable substitution |
| `concurrent_client_limit` | Integer | `5` | Maximum number of concurrent clients connected to the remote broker |
| `connect_timeout` | Duration | `20s` | Connection timeout |
| `keepalive` | Duration | `60s` | MQTT keepalive interval |
| `reconnect_interval` | Duration | `5s` | Automatic reconnection interval |
| `message_channel_capacity` | Integer | `100_000` | Maximum number of messages the channel can hold simultaneously |
| `mqtt_ver` | String | `v5` | MQTT protocol version: `v4` (3.1.1) or `v5` (5.0) |

#### MQTT v4 Options

Options applied when `mqtt_ver = "v4"`.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `v4.clean_session` | Boolean | `true` | Clear session state on connection |
| `v4.last_will` | Table | - | Last will message (optional). See Last Will table below |

#### MQTT v5 Options

Options applied when `mqtt_ver = "v5"`.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `v5.clean_start` | Boolean | `true` | Clean start on connection |
| `v5.session_expiry_interval` | Duration | `0s` | Session expiry interval. `0` means session ends immediately on disconnect |
| `v5.receive_maximum` | Integer | `16` | Maximum number of QoS 1 and QoS 2 messages the client can handle simultaneously |
| `v5.maximum_packet_size` | Size | `1M` | Maximum packet size negotiated with the broker |
| `v5.topic_alias_maximum` | Integer | `0` | Maximum number of topic aliases |
| `v5.last_will` | Table | - | Last will message (optional). See Last Will table below |

#### Last Will Message (`v4.last_will` / `v5.last_will`)

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `qos` | Integer | `0` | QoS level (0, 1, or 2) |
| `retain` | Boolean | `false` | Whether to retain the will message |
| `topic` | String | - | Will message topic |
| `message` | String | - | Will message content |
| `encoding` | String | `plain` | Message encoding method: `plain` or `base64` |

### Bridge Entry (`[[bridges.entries]]`)

Each bridge entry defines a topic forwarding rule.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `local.topic_filter` | String | - | Local topic filter. All messages matching this topic filter will be forwarded |
| `remote.qos` | Integer | - | QoS for forwarded messages. Values: 0, 1, 2. If not set, follows original message QoS |
| `remote.retain` | Boolean | `false` | Whether to retain the forwarded message |
| `remote.topic` | String | - | Remote topic for forwarded messages. Supports `${local.topic}` variable for the original local topic |
| `remote.skip_levels` | Integer | `0` | Number of leading topic levels to skip when substituting `${local.topic}`. For example, if local topic is `a/b/c/d` and `skip_levels = 2`, `${local.topic}` resolves to `c/d` |

## Example Configuration

```toml
[[bridges]]
enable = true
name = "bridge_name_1"
client_id_prefix = "prefix"
server = "tcp://127.0.0.1:2883"
username = "rmqtt_u"
password = "public"
concurrent_client_limit = 5
connect_timeout = "20s"
keepalive = "60s"
reconnect_interval = "5s"
message_channel_capacity = 100_000
mqtt_ver = "v5"

v5.clean_start = true
v5.session_expiry_interval = "0s"
v5.receive_maximum = 16
v5.maximum_packet_size = "1M"
v5.topic_alias_maximum = 0

[[bridges.entries]]
local.topic_filter = "local/topic/egress/#"
remote.qos = 1
remote.retain = false
remote.topic = "remote/topic/egress/${local.topic}"
```

## Dependencies

- rmqtt (feature: plugin)
- ntex
- ntex-mqtt
- ntex-tls
- tokio
- rustls

## License

This project is licensed under the MIT License. See the [LICENSE](https://github.com/rmqtt/rmqtt/blob/master/LICENSE) file for details.
