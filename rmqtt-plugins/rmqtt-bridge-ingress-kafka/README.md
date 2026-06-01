# RMQTT Bridge Ingress Kafka

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-ingress-kafka.svg)](https://crates.io/crates/rmqtt-bridge-ingress-kafka)
[![license](https://img.shields.io/crates/l/rmqtt-bridge-ingress-kafka.svg)](https://github.com/rmqtt/rmqtt/blob/master/LICENSE)

## Overview

`rmqtt-bridge-ingress-kafka` is an RMQTT plugin that consumes messages from one or more Apache Kafka topics and publishes them to the local RMQTT broker. It acts as an ingress bridge, bridging Kafka messages into MQTT with flexible topic mapping and consumer group management.

Key features:
- Consume messages from Kafka topics with consumer groups
- Flexible topic transformation with `${kafka.key}` variable substitution
- Configurable consumer offset start position
- Configurable Kafka consumer properties (librdkafka settings)
- Partition-level consumption control
- Message expiration interval

## Usage

### Add Dependency

Add the following to your `Cargo.toml`:

```toml
[dependencies]
rmqtt-bridge-ingress-kafka = { version = "*", features = ["plugin"] }
```

### Register Plugin

Add the plugin to your RMQTT node configuration:

```toml
[[plugins]]
name = "rmqtt-bridge-ingress-kafka"
register = true
```

Optionally, specify a custom configuration file path:

```toml
[[plugins]]
name = "rmqtt-bridge-ingress-kafka"
register = true
config_file = "/path/to/rmqtt-bridge-ingress-kafka.toml"
```

## Configuration

The configuration file uses TOML format. Below is the complete list of configuration options.

### Bridge Instance (`[[bridges]]`)

Each bridge instance defines a connection to a Kafka cluster.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `enable` | Boolean | `false` | Whether to enable this bridge instance |
| `name` | String | - | Unique bridge name identifier |
| `servers` | String | - | Comma-separated list of Kafka broker addresses. Format: `host1:port1,host2:port2` |
| `client_id_prefix` | String | - | Kafka client ID prefix |
| `expiry_interval` | Duration | `5m` | Message expiration time. `0` means no expiration |

### Kafka Properties (`[bridges.properties]`)

Additional librdkafka consumer properties. See [librdkafka Configuration](https://github.com/confluentinc/librdkafka/blob/master/CONFIGURATION.md) for all available options.

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `message.timeout.ms` | String | `5000` | Message timeout in milliseconds |
| `enable.auto.commit` | String | `true` | Whether to enable auto-commit of consumer offsets |
| Any other librdkafka property | String | - | See librdkafka configuration documentation |

### Bridge Entry (`[[bridges.entries]]`)

Each bridge entry defines a topic consumption and forwarding rule.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `remote.topic` | String | - | Kafka topic to consume from |
| `remote.group_id` | String | - | Kafka consumer group ID |
| `remote.start_partition` | Integer | `-1` | Start partition index (inclusive). `-1` means all partitions |
| `remote.stop_partition` | Integer | `-1` | Stop partition index (inclusive). `-1` means all partitions |
| `remote.offset` | String | `end` | Consumer offset start position. Values: `beginning`, `end`, `stored`, or a specific offset number |
| `local.qos` | Integer | - | QoS for publishing to the local broker. Values: 0, 1, 2. If not set, follows Kafka message metadata |
| `local.topic` | String | - | Local MQTT topic to publish to. Supports `${kafka.key}` variable for the Kafka message key |
| `local.retain` | Boolean | `false` | Whether to retain the published message locally |

## Example Configuration

```toml
[[bridges]]
enable = true
name = "bridge_kafka_1"
servers = "127.0.0.1:9092"
client_id_prefix = "kafka_001"
expiry_interval = "5m"

[bridges.properties]
"message.timeout.ms" = "5000"

[[bridges.entries]]
remote.topic = "remote-topic1-ingress"
remote.group_id = "group_id_001"
local.topic = "local/topic1/ingress/${kafka.key}"
```

## Dependencies

- rmqtt (feature: plugin)
- rdkafka
- tokio

## License

This project is licensed under the MIT License. See the [LICENSE](https://github.com/rmqtt/rmqtt/blob/master/LICENSE) file for details.
