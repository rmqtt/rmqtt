# RMQTT Bridge Ingress MQTT

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-ingress-mqtt.svg)](https://crates.io/crates/rmqtt-bridge-ingress-mqtt)
[![license](https://img.shields.io/crates/l/rmqtt-bridge-ingress-mqtt.svg)](https://github.com/rmqtt/rmqtt/blob/master/LICENSE)

## Overview

`rmqtt-bridge-ingress-mqtt` is an RMQTT plugin that connects to a remote MQTT broker and forwards messages to the local RMQTT broker. It acts as an ingress bridge, subscribing to remote topics and republishing the received messages locally with topic transformation support.

Key features:
- Support for MQTT v4 (3.1.1) and v5.0 protocol versions
- TLS/SSL encrypted connections to remote brokers
- Configurable multiple bridge instances
- Topic transformation with `${remote.topic}` variable substitution
- Shared subscription support (`$share/` prefix)
- Message expiry interval
- Automatic reconnection

## Usage

### Add Dependency

Add the following to your `Cargo.toml`:

```toml
[dependencies]
rmqtt-bridge-ingress-mqtt = { version = "*", features = ["plugin"] }
```

### Register Plugin

Add the plugin to your RMQTT node configuration:

```toml
[[plugins]]
name = "rmqtt-bridge-ingress-mqtt"
register = true
```

Optionally, specify a custom configuration file path:

```toml
[[plugins]]
name = "rmqtt-bridge-ingress-mqtt"
register = true
config_file = "/path/to/rmqtt-bridge-ingress-mqtt.toml"
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
| `expiry_interval` | Duration | `5m` | Message expiration time. `0` means no expiration |
| `mqtt_ver` | String | `v4` | MQTT protocol version: `v4` (3.1.1) or `v5` (5.0) |

#### MQTT v4 Options

Options applied when `mqtt_ver = "v4"`.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `v4.clean_session` | Boolean | `true` | Clear session state on connection |

#### MQTT v5 Options

Options applied when `mqtt_ver = "v5"`.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `v5.clean_start` | Boolean | `true` | Clean start on connection |
| `v5.session_expiry_interval` | Duration | `0s` | Session expiry interval. `0` means session ends immediately on disconnect |
| `v5.receive_maximum` | Integer | `16` | Maximum number of QoS 1 and QoS 2 messages the client can handle simultaneously |
| `v5.maximum_packet_size` | Size | `1M` | Maximum packet size negotiated with the broker |
| `v5.topic_alias_maximum` | Integer | `0` | Maximum number of topic aliases |

### Bridge Entry (`[[bridges.entries]]`)

Each bridge entry defines a topic subscription and forwarding rule.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `remote.qos` | Integer | `0` | QoS level for subscribing to the remote topic. Values: 0, 1, 2 |
| `remote.topic` | String | - | Remote topic to subscribe to. Supports shared subscription format `$share/g/remote/topic` |
| `local.qos` | Integer | - | QoS for publishing to the local broker. Values: 0, 1, 2. If not set, follows original message QoS |
| `local.topic` | String | - | Local topic for published messages. Supports `${remote.topic}` variable |
| `local.retain` | Boolean | `false` | Whether to retain the published message locally |

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
expiry_interval = "5m"
mqtt_ver = "v4"

v4.clean_session = true

[[bridges.entries]]
remote.qos = 1
remote.topic = "$share/g/remote/topic1/ingress/#"
local.qos = 1
local.topic = "local/topic1/ingress/${remote.topic}"
local.retain = true
```

## Dependencies

- rmqtt (feature: plugin)
- ntex
- ntex-mqtt
- tokio

## License

This project is licensed under the MIT License. See the [LICENSE](https://github.com/rmqtt/rmqtt/blob/master/LICENSE) file for details.
