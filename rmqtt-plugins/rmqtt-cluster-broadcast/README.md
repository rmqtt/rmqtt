[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-cluster-broadcast

[![crates.io](https://img.shields.io/crates/v/rmqtt-cluster-broadcast.svg)](https://crates.io/crates/rmqtt-cluster-broadcast)

Broadcast cluster plugin. Provides high-throughput distributed clustering via message broadcasting.

## Usage

```toml
[dependencies]
rmqtt-cluster-broadcast = "0.21"
```

```rust
rmqtt_cluster_broadcast::register(&scx, true, false).await?;
```

## Configuration

File: `rmqtt-cluster-broadcast.toml`

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `message_type` | integer | `98` | gRPC message type |
| `node_grpc_addrs` | array of string | `["1@127.0.0.1:5363", "2@127.0.0.1:5364", "3@127.0.0.1:5365"]` | Cluster node gRPC addresses |
| `node_grpc_batch_size` | integer | `128` | Maximum messages sent in batch |
| `node_grpc_client_concurrency_limit` | integer | `128` | Client concurrent request limit |
| `node_grpc_client_timeout` | string | `"15s"` | Connect and send timeout |
| `task_exec_queue_workers` | integer | `500` | Task execution queue workers |
| `task_exec_queue_max` | integer | `100_000` | Task execution queue max capacity |

## Dependencies

`rmqtt` (features: `plugin`, `grpc`, `stats`)

## License

MIT OR Apache-2.0
