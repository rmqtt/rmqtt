# RMQTT Bridge Egress Kafka

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-egress-kafka.svg)](https://crates.io/crates/rmqtt-bridge-egress-kafka)
[![license](https://img.shields.io/crates/l/rmqtt-bridge-egress-kafka.svg)](https://github.com/rmqtt/rmqtt/blob/master/LICENSE)

## Overview

`rmqtt-bridge-egress-kafka` is an RMQTT plugin that forwards local MQTT publish messages to one or more Apache Kafka topics. It acts as an egress bridge, subscribing to local MQTT topics and producing the messages to Kafka with topic transformation support.

Key features:
- Forward MQTT messages to Kafka topics
- Topic transformation with `${local.topic}` variable substitution
- Configurable Kafka producer properties (librdkafka settings)
- Multiple bridge and entry configuration
- Automatic reconnection

## Usage

### Add Dependency

Add the following to your `Cargo.toml`:

```toml
[dependencies]
rmqtt-bridge-egress-kafka = { version = "*", features = ["plugin"] }
```

### Register Plugin

Add the plugin to your RMQTT node configuration:

```toml
[[plugins]]
name = "rmqtt-bridge-egress-kafka"
register = true
```

Optionally, specify a custom configuration file path:

```toml
[[plugins]]
name = "rmqtt-bridge-egress-kafka"
register = true
config_file = "/path/to/rmqtt-bridge-egress-kafka.toml"
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
| `concurrent_client_limit` | Integer | `3` | Maximum limit of clients connected to the remote Kafka broker |

### Kafka Properties (`[bridges.properties]`)

Additional librdkafka producer properties. See [librdkafka Configuration](https://github.com/confluentinc/librdkafka/blob/master/CONFIGURATION.md) for all available options.

| Property | Type | Default | Description |
|----------|------|---------|-------------|
| `message.timeout.ms` | String | `5000` | Message timeout in milliseconds |
| Any other librdkafka property | String | - | See librdkafka configuration documentation |

### Bridge Entry (`[[bridges.entries]]`)

Each bridge entry defines a topic forwarding rule.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `local.topic_filter` | String | - | Local MQTT topic filter. All messages matching this filter will be forwarded |
| `remote.topic` | String | - | Kafka topic to produce to. Supports `${local.topic}` variable substitution |
| `remote.queue_timeout` | Duration | `0m` | How long to retry if the librdkafka producer queue is full. `0` means never block |
| `remote.partition` | Integer | - | Destination partition of the record (optional) |
| `remote.skip_levels` | Integer | `0` | Number of leading topic levels to skip when substituting `${local.topic}` |

## Example Configuration

```toml
[[bridges]]
enable = true
name = "bridge_kafka_1"
servers = "127.0.0.1:9092"
client_id_prefix = "kafka_001"
concurrent_client_limit = 3

[bridges.properties]
"message.timeout.ms" = "5000"

[[bridges.entries]]
local.topic_filter = "local/topic1/egress/#"
remote.topic = "remote-topic1-egress-${local.topic}"
remote.queue_timeout = "0m"
```

## Dependencies

- rmqtt (feature: plugin)
- rdkafka
- tokio

## License

This project is licensed under the MIT License. See the [LICENSE](https://github.com/rmqtt/rmqtt/blob/master/LICENSE) file for details.
