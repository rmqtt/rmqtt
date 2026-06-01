# RMQTT Bridge Egress NATS

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-egress-nats.svg)](https://crates.io/crates/rmqtt-bridge-egress-nats)
[![license](https://img.shields.io/crates/l/rmqtt-bridge-egress-nats.svg)](https://github.com/rmqtt/rmqtt/blob/master/LICENSE)

## Overview

`rmqtt-bridge-egress-nats` is an RMQTT plugin that forwards local MQTT publish messages to one or more NATS subjects. It acts as an egress bridge, subscribing to local MQTT topics and publishing messages to NATS with topic transformation support.

Key features:
- Forward MQTT messages to NATS subjects
- TLS/SSL encrypted connections
- Multiple authentication methods (JWT, NKey, username/password, token)
- Forward MQTT metadata (client info, publish flags)
- Configurable NATS connection options (no echo, ping interval, connection timeout)
- Topic transformation with `${local.topic}` variable substitution

## Usage

### Add Dependency

Add the following to your `Cargo.toml`:

```toml
[dependencies]
rmqtt-bridge-egress-nats = { version = "*", features = ["plugin"] }
```

### Register Plugin

Add the plugin to your RMQTT node configuration:

```toml
[[plugins]]
name = "rmqtt-bridge-egress-nats"
register = true
```

Optionally, specify a custom configuration file path:

```toml
[[plugins]]
name = "rmqtt-bridge-egress-nats"
register = true
config_file = "/path/to/rmqtt-bridge-egress-nats.toml"
```

## Configuration

The configuration file uses TOML format. Below is the complete list of configuration options.

### Bridge Instance (`[[bridges]]`)

Each bridge instance defines a connection to a NATS server.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enable` | Boolean | `false` | Whether to enable this bridge instance |
| `name` | String | - | Unique bridge name identifier |
| `servers` | String | - | NATS server address. Format: `nats://host:port` (plain TCP) or `tls://host:port` (TLS) |
| `producer_name_prefix` | String | - | Producer name prefix |
| `no_echo` | Boolean | - | Do not receive messages published by this connection |
| `ping_interval` | Duration | `60s` | Ping interval for keepalive |
| `connection_timeout` | Duration | `10s` | Connection timeout |
| `tls_required` | Boolean | - | Require TLS for connection |
| `tls_first` | Boolean | - | Perform TLS handshake before NATS protocol |
| `root_certificates` | String | - | Root certificates file path |
| `client_cert` | String | - | Client certificate file path |
| `client_key` | String | - | Client key file path |
| `sender_capacity` | Integer | `256` | Sender channel capacity |
| `auth.jwt` | String | - | JWT authentication token |
| `auth.jwt_seed` | String | - | JWT seed for signing |
| `auth.nkey` | String | - | NKey for authentication |
| `auth.username` | String | - | Username for authentication |
| `auth.password` | String | - | Password for authentication |
| `auth.token` | String | - | Token for authentication |

### Bridge Entry (`[[bridges.entries]]`)

Each bridge entry defines a topic forwarding rule.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `local.topic_filter` | String | - | Local MQTT topic filter. All messages matching this filter will be forwarded |
| `remote.topic` | String | - | NATS subject to publish to. Supports `${local.topic}` variable substitution |
| `remote.forward_all_from` | Boolean | `false` | Forward all from data: `from_type`, `from_node`, `from_ipaddress`, `from_clientid`, `from_username` |
| `remote.forward_all_publish` | Boolean | `false` | Forward all publish data: `dup`, `retain`, `qos`, `packet_id`, `topic`, `payload` |
| `remote.skip_levels` | Integer | `0` | Number of leading topic levels to skip when substituting `${local.topic}` |

## Example Configuration

```toml
[[bridges]]
enable = true
name = "bridge_nats_1"
servers = "nats://127.0.0.1:4222"
producer_name_prefix = "producer_1"

[[bridges.entries]]
local.topic_filter = "local/topic1/egress/#"
remote.topic = "test1"
```

## Dependencies

- rmqtt (feature: plugin)
- async-nats
- tokio

## License

This project is licensed under the MIT License. See the [LICENSE](https://github.com/rmqtt/rmqtt/blob/master/LICENSE) file for details.
