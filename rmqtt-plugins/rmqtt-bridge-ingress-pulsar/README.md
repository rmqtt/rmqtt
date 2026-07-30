# RMQTT Bridge Ingress Pulsar

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-ingress-pulsar.svg)](https://crates.io/crates/rmqtt-bridge-ingress-pulsar)
[![license](https://img.shields.io/crates/l/rmqtt-bridge-ingress-pulsar.svg)](https://github.com/rmqtt/rmqtt/blob/master/LICENSE)

## Overview

`rmqtt-bridge-ingress-pulsar` is an RMQTT plugin that consumes messages from Apache Pulsar topics and publishes them to the local RMQTT broker. It acts as an ingress bridge, bridging Pulsar messages into MQTT with comprehensive subscription and message transformation capabilities.

Key features:
- Consume from Pulsar topics with various subscription types (exclusive, shared, failover, key_shared)
- Support for single topic, multiple topics, and regex topic patterns
- TLS/SSL encrypted connections with certificate chain verification
- Token (JWT/Biscuit) and OAuth2 authentication support
- Flexible topic transformation with placeholders (`${remote.topic}`, `${remote.properties.*}`, `${remote.payload.*}`)
- Consumer configuration (batch size, dead letter policy, priority level)
- Message format support (bytes, JSON) with payload path extraction
- Consumer group (subscription) management
- Dead letter policy support
- Configurable message expiry interval

## Usage

### Add Dependency

Add the following to your `Cargo.toml`:

```toml
[dependencies]
rmqtt-bridge-ingress-pulsar = { version = "*", features = ["plugin"] }
```

### Register Plugin

Add the plugin to your RMQTT node configuration:

```toml
[[plugins]]
name = "rmqtt-bridge-ingress-pulsar"
register = true
```

Optionally, specify a custom configuration file path:

```toml
[[plugins]]
name = "rmqtt-bridge-ingress-pulsar"
register = true
config_file = "/path/to/rmqtt-bridge-ingress-pulsar.toml"
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
| `consumer_name_prefix` | String | - | Consumer name prefix |
| `cert_chain_file` | String | - | Custom certificate chain file path for TLS authentication |
| `allow_insecure_connection` | Boolean | `false` | Allow insecure TLS connection |
| `tls_hostname_verification_enabled` | Boolean | `true` | Whether hostname verification is enabled for TLS |
| `auth.name` | String | - | Authentication type: `token` (JWT/Biscuit) or `oauth2` |
| `auth.data` | String | - | Authentication data. For token: the JWT/Biscuit token string. For oauth2: JSON with `issuer_url` and `credentials_url` |
| `expiry_interval` | Duration | `5m` | Message expiration time. `0` means no expiration |

### Bridge Entry (`[[bridges.entries]]`)

Each bridge entry defines a topic consumption and forwarding rule.

#### Consumer Configuration

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `remote.topic` | String | - | Pulsar topic to consume from |
| `remote.topics` | Array of String | - | List of topics for the consumer |
| `remote.topic_regex` | String | - | Regex pattern to match topics. Consumer listens on all matching topics |
| `remote.subscription_type` | String | `shared` | Subscription type: `exclusive`, `shared`, `failover`, `key_shared` |
| `remote.subscription` | String | - | Subscription name |
| `lookup_namespace` | String | `public/default` | Tenant/Namespace for regex matching |
| `topic_refresh_interval` | Duration | `10s` | Interval for refreshing topics when using regex pattern |
| `consumer_id` | Integer | - | Consumer ID |
| `batch_size` | Integer | `1000` | Maximum batch size for messages from Pulsar |
| `remote.dead_letter_policy` | Table | - | Dead letter policy: `{max_redeliver_count, dead_letter_topic}` |
| `remote.unacked_message_resend_delay` | Duration | - | Delay before unacknowledged messages are resent |

#### Consumer Options (`remote.options`)

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `remote.options.priority_level` | Integer | - | Priority level for the consumer |
| `remote.options.durable` | Boolean | - | Whether the subscription is backed by a durable cursor |
| `remote.options.read_compacted` | Boolean | - | Read compacted topics |
| `remote.options.initial_position` | String | `latest` | Initial position: `earliest` (oldest) or `latest` (most recent) |
| `remote.options.metadata` | Table | `{}` | User-defined properties added to all messages |

#### Payload and Topic Mapping

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `remote.payload_format` | String | `bytes` | Payload data format: `bytes` or `json` |
| `remote.payload_path` | String | - | Extract publish payload from Pulsar message JSON payload. E.g., `/msg` or `/data/msg` |
| `local.topic` | String | - | MQTT publish topic. Placeholders: `${remote.topic}`, `${remote.properties.xxxx}`, `${remote.payload.xxxx}` |
| `local.topics` | Array of String | - | Multiple MQTT publish topics with placeholders |
| `local.qos` | Integer | - | QoS for publishing. Values: 0, 1, 2. If not set, follows Pulsar message properties |
| `local.retain` | Boolean | `false` | Whether to retain the published message locally |
| `local.allow_empty_forward` | Boolean | `true` | Allow forwarding of empty messages |

## Example Configuration

```toml
[[bridges]]
enable = true
name = "bridge_pulsar_1"
servers = "pulsar://127.0.0.1:6650"
consumer_name_prefix = "consumer_1"
expiry_interval = "5m"

[[bridges.entries]]
remote.topic = "non-persistent://public/default/test"
local.topic = "local/topic1/ingress/${remote.topic}"
```

## Dependencies

- rmqtt (feature: plugin)
- pulsar
- tokio

## License

This project is licensed under the MIT License. See the [LICENSE](https://github.com/rmqtt/rmqtt/blob/master/LICENSE) file for details.
