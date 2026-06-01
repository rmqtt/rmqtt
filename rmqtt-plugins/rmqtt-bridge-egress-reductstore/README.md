[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-bridge-egress-reductstore

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-egress-reductstore.svg)](https://crates.io/crates/rmqtt-bridge-egress-reductstore)

ReductStore egress bridge. Forwards MQTT publish messages to a ReductStore time-series database.

## Overview

Connects to a ReductStore HTTP API server and stores MQTT messages as time-series entries organized by bucket and entry key.

## Usage

```toml
rmqtt-bridge-egress-reductstore = "0.22"
```

```rust
rmqtt_bridge_egress_reductstore::register(&scx, true, false).await?;
```

## Configuration

File: `rmqtt-bridge-egress-reductstore.toml`

```toml
[[bridges]]
enable = true
name = "bridge_reductstore_1"
# ReductStore HTTP API server
servers = "http://127.0.0.1:8383"
producer_name_prefix = "producer_1"
# API token for authentication
# api_token = ""
# HTTP request timeout
# timeout = "15s"
disable_ssl = false

[[bridges.entries]]
# Local topic filter to match messages for forwarding
local.topic_filter = "local/topic1/egress/#"

remote.bucket = "bucket1"          # ReductStore bucket name
remote.entry = "test1"             # ReductStore entry key
remote.quota_size = 1_000_000_000  # Bucket quota size
remote.exist_ok = true             # Don't fail if bucket exists
remote.forward_all_from = true     # Forward from metadata (from_type, from_node, etc.)
remote.forward_all_publish = true  # Forward publish metadata (dup, retain, qos, etc.)
```

### Config Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `bridges[].enable` | `bool` | `true` | Enable this bridge |
| `bridges[].name` | `string` | — | Bridge identifier |
| `bridges[].servers` | `string` | — | ReductStore HTTP URL |
| `bridges[].producer_name_prefix` | `string` | — | Producer name prefix |
| `bridges[].api_token` | `string` | — | API authentication token |
| `bridges[].timeout` | `string` | `"15s"` | HTTP request timeout |
| `bridges[].disable_ssl` | `bool` | `false` | Disable SSL verification |
| `bridges[].entries[].local.topic_filter` | `string` | — | Local topic filter for forwarding |
| `bridges[].entries[].remote.bucket` | `string` | — | ReductStore bucket name |
| `bridges[].entries[].remote.entry` | `string` | — | ReductStore entry key |
| `bridges[].entries[].remote.quota_size` | `u64` | — | Bucket quota in bytes |
| `bridges[].entries[].remote.exist_ok` | `bool` | `true` | Allow existing bucket |
| `bridges[].entries[].remote.forward_all_from` | `bool` | `true` | Forward source metadata |
| `bridges[].entries[].remote.forward_all_publish` | `bool` | `true` | Forward publish metadata |
| `bridges[].entries[].remote.skip_levels` | `u32` | `0` | Topic levels to skip in `${local.topic}` |

## Dependencies

`rmqtt` (feature `plugin`), `reduct-rs`, `tokio`

## License

MIT OR Apache-2.0
