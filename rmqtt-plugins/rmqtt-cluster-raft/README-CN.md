[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-cluster-raft

[![crates.io](https://img.shields.io/crates/v/rmqtt-cluster-raft.svg)](https://crates.io/crates/rmqtt-cluster-raft)

Raft 共识集群插件。提供强一致性分布式集群能力。

## 概述

Raft 在专用操作系统线程中运行，使用独立的 Tokio 运行时。支持可配置压缩、健康检查自动退出和细粒度 Raft 调优。

## 使用

```toml
[dependencies]
rmqtt-cluster-raft = "0.21"
```

需要 `rmqtt` features：`plugin`、`grpc`、`stats`、`msgstore`。

```rust
rmqtt_cluster_raft::register(&scx, true, false).await?;
```

CLI 示例：

```bash
rmqttd --id 1 --plugins-default-startups "rmqtt-cluster-raft" \
  --node-grpc-addrs "1@10.0.0.1:5363" "2@10.0.0.2:5363" "3@10.0.0.3:5363" \
  --raft-peer-addrs "1@10.0.0.1:6003" "2@10.0.0.2:6003" "3@10.0.0.3:6003"
```

## 配置

文件：`rmqtt-cluster-raft.toml`

### 通用

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `worker_threads` | integer | `6` | 集群 Raft 工作线程数 |
| `message_type` | integer | `198` | gRPC 消息类型 |
| `try_lock_timeout` | string | `"10s"` | 握手锁超时 |
| `task_exec_queue_workers` | integer | `500` | 任务执行队列工作线程数 |
| `task_exec_queue_max` | integer | `100_000` | 任务执行队列最大容量 |
| `compression` | string | `"zstd"` | 快照压缩算法 |
| `leader_id` | integer | `0` | 指定领导者节点 ID。主要用于单机伪集群开发/测试（0 = 自动选择配置中的第一个节点）。可通过 `--raft-leader-id` CLI 参数覆盖 |

压缩算法：`lz4_flex`、`lz4`、`zstd`、`snap`、`flate2`

### gRPC

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `node_grpc_addrs` | array of string | `["1@127.0.0.1:5363", ...]` | 集群节点 gRPC 地址 |
| `node_grpc_batch_size` | integer | `128` | 批量发送的最大消息数 |
| `node_grpc_client_concurrency_limit` | integer | `128` | 客户端并发请求限制 |
| `node_grpc_client_timeout` | string | `"60s"` | 连接和发送超时 |
| `laddr` | string | — | Raft 集群监听地址（可选） |

### Raft 对端

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `raft_peer_addrs` | array of string | `["1@127.0.0.1:6003", ...]` | Raft 对端地址 |

### 健康检查

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `health.exit_on_node_unavailable` | boolean | `false` | 节点不可用时自动退出 |
| `health.exit_code` | integer | `-1` | 退出码 |
| `health.max_continuous_unavailable_count` | integer | `2` | 连续失败次数触发退出 |
| `health.unavailable_check_interval` | string | `"2s"` | 不可用检查间隔 |

### Raft 调优

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `raft.grpc_timeout` | string | `"6s"` | Raft gRPC 超时 |
| `raft.grpc_concurrency_limit` | integer | `200` | Raft gRPC 并发限制 |
| `raft.grpc_breaker_threshold` | integer | `5` | 熔断器阈值 |
| `raft.grpc_breaker_retry_interval` | string | `"2500ms"` | 熔断器重试间隔 |
| `raft.proposal_batch_size` | integer | `60` | 提议批量大小 |
| `raft.proposal_batch_timeout` | string | `"200ms"` | 提议批量超时 |
| `raft.snapshot_interval` | string | `"300s"` | 快照间隔 |
| `raft.heartbeat` | string | `"100ms"` | 心跳间隔 |
| `raft.election_tick` | integer | `10` | 选举滴答数 |
| `raft.heartbeat_tick` | integer | `5` | 心跳滴答数 |
| `raft.max_size_per_msg` | integer | `0` | 每消息最大大小（0 = 无限制） |
| `raft.max_inflight_msgs` | integer | `256` | 最大未处理消息数 |
| `raft.check_quorum` | boolean | `true` | 选举时检查法定人数 |
| `raft.pre_vote` | boolean | `true` | 启用预投票 |
| `raft.min_election_tick` | integer | `0` | 最小选举滴答（0 = 使用默认值） |
| `raft.max_election_tick` | integer | `0` | 最大选举滴答（0 = 使用默认值） |
| `raft.read_only_option` | string | `"Safe"` | 只读选项（Safe、LeaseBased） |
| `raft.skip_bcast_commit` | boolean | `false` | 跳过广播提交 |
| `raft.batch_append` | boolean | `false` | 启用批量追加 |
| `raft.priority` | integer | `0` | 选举优先级 |

## 依赖

`rmqtt`（features：`plugin`、`grpc`、`stats`、`msgstore`）、`rmqtt-raft`、`tokio`、`serde`

## 许可证

MIT OR Apache-2.0
