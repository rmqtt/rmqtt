# RMQTT Bridge Egress Pulsar

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-egress-pulsar.svg)](https://crates.io/crates/rmqtt-bridge-egress-pulsar)
[![license](https://img.shields.io/crates/l/rmqtt-bridge-egress-pulsar.svg)](https://github.com/rmqtt/rmqtt/blob/master/LICENSE)

## Overview

`rmqtt-bridge-egress-pulsar` is an RMQTT plugin that forwards local MQTT publish messages to one or more Apache Pulsar topics. It acts as an egress bridge, subscribing to local MQTT topics and producing messages to Pulsar with comprehensive message transformation capabilities.

Key features:
- Forward MQTT messages to Pulsar persistent and non-persistent topics
- TLS/SSL encrypted connections with certificate chain verification
- Token (JWT/Biscuit) and OAuth2 authentication support
- Message partitioning via partition key
- Message ordering via ordering key
- Configurable compression (lz4, zlib, zstd, snappy)
- Producer access mode configuration
- Message metadata and schema version support
- Forward MQTT metadata (client info, publish flags)

## Usage

### Add Dependency

Add the following to your `Cargo.toml`:

```toml
[dependencies]
rmqtt-bridge-egress-pulsar = { version = "*", features = ["plugin"] }
```

### Register Plugin

Add the plugin to your RMQTT node configuration:

```toml
[[plugins]]
name = "rmqtt-bridge-egress-pulsar"
register = true
```

Optionally, specify a custom configuration file path:

```toml
[[plugins]]
name = "rmqtt-bridge-egress-pulsar"
register = true
config_file = "/path/to/rmqtt-bridge-egress-pulsar.toml"
```

## Configuration

The configuration file uses TOML format. Below is the complete list of configuration options.

### Bridge Instance (`[[bridges]]`)

Each bridge instance defines a connection to a Pulsar cluster.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enable` | Boolean | `false` | Whether to enable this bridge instance |
| `name` | String | - | Unique bridge name identifier |
| `servers` | String | - | Pulsar broker address. Format: `pulsar://host:port` (plain TCP) or `pulsar+ssl://host:port` (TLS) |
| `producer_name_prefix` | String | - | Producer name prefix |
| `cert_chain_file` | String | - | Custom certificate chain file path for TLS authentication |
| `allow_insecure_connection` | Boolean | `false` | Allow insecure TLS connection |
| `tls_hostname_verification_enabled` | Boolean | `true` | Whether hostname verification is enabled for TLS |
| `auth.name` | String | - | Authentication type: `token` (JWT/Biscuit) or `oauth2` |
| `auth.data` | String | - | Authentication data. For token: the JWT/Biscuit token string. For oauth2: JSON with `issuer_url` and `credentials_url` |

### Bridge Entry (`[[bridges.entries]]`)

Each bridge entry defines a topic forwarding rule.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `local.topic_filter` | String | - | Local MQTT topic filter. All messages matching this filter will be forwarded |
| `remote.topic` | String | - | Pulsar topic to produce to. Supports `${local.topic}` variable substitution |
| `remote.forward_all_from` | Boolean | `false` | Forward all from data: `from_type`, `from_node`, `from_ipaddress`, `from_clientid`, `from_username` |
| `remote.forward_all_publish` | Boolean | `false` | Forward all publish data: `dup`, `retain`, `qos`, `packet_id`, `topic`, `payload` |
| `remote.partition_key` | String | - | Determines the target partition. Messages with the same partition key go to the same partition |
| `remote.ordering_key` | String or Table | - | Controls message ordering. Values: `clientid`, `uuid`, `random`, or `{type="random", len=10}` |
| `remote.replicate_to` | Array | - | Override namespace replication destinations |
| `remote.schema_version` | String | - | Schema version for the message |
| `remote.options.metadata` | Table | `{}` | User-defined properties added to all messages |
| `remote.options.compression` | String | - | Message compression algorithm: `lz4`, `zlib`, `zstd`, `snappy` |
| `remote.options.access_mode` | Integer | `0` | Producer access mode: `0` = shared, `1` = exclusive, `2` = wait_for_exclusive, `3` = exclusive_without_fencing |
| `remote.skip_levels` | Integer | `0` | Number of leading topic levels to skip when substituting `${local.topic}` |

## Example Configuration

```toml
[[bridges]]
enable = true
name = "bridge_pulsar_1"
servers = "pulsar://127.0.0.1:6650"
producer_name_prefix = "producer_1"

[[bridges.entries]]
local.topic_filter = "local/topic1/egress/#"
remote.topic = "non-persistent://public/default/test1"
remote.forward_all_from = false
remote.forward_all_publish = false
```

## Dependencies

- rmqtt (feature: plugin)
- pulsar
- tokio

## License

This project is licensed under the MIT License. See the [LICENSE](https://github.com/rmqtt/rmqtt/blob/master/LICENSE) file for details.
