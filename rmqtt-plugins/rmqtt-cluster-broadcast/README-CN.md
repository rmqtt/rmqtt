[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-cluster-broadcast

[![crates.io](https://img.shields.io/crates/v/rmqtt-cluster-broadcast.svg)](https://crates.io/crates/rmqtt-cluster-broadcast)

广播集群插件。通过消息广播提供高吞吐量的分布式集群能力。

## 使用

```toml
[dependencies]
rmqtt-cluster-broadcast = "0.21"
```

```rust
rmqtt_cluster_broadcast::register(&scx, true, false).await?;
```

## 配置

文件：`rmqtt-cluster-broadcast.toml`

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `message_type` | integer | `98` | gRPC 消息类型 |
| `node_grpc_addrs` | array of string | `["1@127.0.0.1:5363", "2@127.0.0.1:5364", "3@127.0.0.1:5365"]` | 集群节点 gRPC 地址 |
| `node_grpc_batch_size` | integer | `128` | 批量发送的最大消息数 |
| `node_grpc_client_concurrency_limit` | integer | `128` | 客户端并发请求限制 |
| `node_grpc_client_timeout` | string | `"15s"` | 连接和发送超时 |
| `node_grpc_circuit_breaker_enabled` | boolean | `true` | 启用 gRPC 熔断器 |
| `node_grpc_circuit_failure_threshold` | integer | `10` | 连续失败次数超过此值后跳闸到 OPEN |
| `node_grpc_circuit_reset_timeout` | string | `"15s"` | OPEN 状态持续时间，之后进入探测（HALF_OPEN） |
| `node_grpc_circuit_half_open_success_threshold` | integer | `3` | HALF_OPEN 状态下连续成功次数达到此值后关闭电路 |
| `task_exec_queue_workers` | integer | `500` | 任务执行队列工作线程数 |
| `task_exec_queue_max` | integer | `100_000` | 任务执行队列最大容量 |

## 依赖

`rmqtt`（features：`plugin`、`grpc`、`stats`）

## 许可证

MIT OR Apache-2.0
