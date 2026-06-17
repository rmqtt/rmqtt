[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-bridge-ingress-nats

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-ingress-nats.svg)](https://crates.io/crates/rmqtt-bridge-ingress-nats)

NATS ingress bridge. Consumes messages from NATS subjects and publishes them to the local RMQTT broker.

## Overview

Connects to a NATS server, subscribes to configured subjects, and forwards received messages as MQTT publishes. Supports TLS connections, authentication (JWT, token, username/password, NKey), and configurable consumer options.

## Usage

```toml
rmqtt-bridge-ingress-nats = "0.21"
```

```rust
rmqtt_bridge_ingress_nats::register(&scx, true, false).await?;
```

## Configuration

File: `rmqtt-bridge-ingress-nats.toml`

```toml
[[bridges]]
enable = true
name = "bridge_nats_1"
servers = "nats://127.0.0.1:4222"
consumer_name_prefix = "consumer_1"
no_echo = true
ping_interval = "60s"
connection_timeout = "10s"

[[bridges.entries]]
remote.topic = "test1"           # NATS subject to subscribe to
local.topic = "local/topic1/ingress/${remote.topic}"  # Local MQTT topic
local.qos = 1                    # Optional: override QoS
local.retain = false             # Optional: retain flag
local.allow_empty_forward = false # Optional: forward empty messages
```

### Config Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `bridges[].enable` | `bool` | `true` | Enable this bridge |
| `bridges[].name` | `string` | — | Bridge identifier |
| `bridges[].servers` | `string` | — | NATS server URL (`nats://` or `tls://`) |
| `bridges[].consumer_name_prefix` | `string` | — | Consumer name prefix |
| `bridges[].no_echo` | `bool` | — | No echo mode |
| `bridges[].ping_interval` | `string` | `"60s"` | NATS ping interval |
| `bridges[].connection_timeout` | `string` | `"10s"` | Connection timeout |
| `bridges[].tls_required` | `bool` | — | Require TLS |
| `bridges[].tls_first` | `bool` | — | TLS first (before NATS handshake) |
| `bridges[].auth.*` | — | — | Auth options: `jwt`, `jwt_seed`, `nkey`, `username`, `password`, `token` |
| `bridges[].entries[].remote.topic` | `string` | — | NATS subject to consume |
| `bridges[].entries[].remote.group` | `string` | — | NATS queue group |
| `bridges[].entries[].local.topic` | `string` | — | Local MQTT topic (`${remote.topic}` placeholder) |
| `bridges[].entries[].local.qos` | `u8` | follow message | Override QoS level |
| `bridges[].entries[].local.retain` | `bool` | `false` | Override retain flag |
| `bridges[].entries[].local.allow_empty_forward` | `bool` | `true` | Forward empty messages |

## Dependencies

`rmqtt` (feature `plugin`), `async-nats`, `tokio`

## License

MIT OR Apache-2.0
