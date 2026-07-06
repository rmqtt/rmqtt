[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-session-storage

[![crates.io](https://img.shields.io/crates/v/rmqtt-session-storage.svg)](https://crates.io/crates/rmqtt-session-storage)

会话持久化插件。使用 Sled、Redis 或 Redis 集群后端存储客户端会话状态。

## 使用

```toml
[dependencies]
rmqtt-session-storage = "0.21"
```

```rust
rmqtt_session_storage::register(&scx, true, false).await?;
```

## 配置

文件：`rmqtt-session-storage.toml`

### 存储类型

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `storage.type` | string | `"sled"` | 后端存储类型：`sled`、`redis` 或 `redis-cluster` |

### Sled 后端

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `storage.sled.path` | string | `"/var/log/rmqtt/.cache/session/{node}"` | Sled 数据库文件路径（`{node}` = 节点 ID 占位符） |
| `storage.sled.cache_capacity` | string | `"3G"` | Sled 缓存容量 |

### Redis 后端

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `storage.redis.url` | string | `"redis://127.0.0.1:6379/"` | Redis 服务器 URL |
| `storage.redis.prefix` | string | `"session-{node}"` | 键前缀（`{node}` = 节点 ID 占位符） |

### Redis 集群后端

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `storage.redis-cluster.urls` | array of string | `["redis://127.0.0.1:6380/", ...]` | Redis 集群节点 URL |
| `storage.redis-cluster.prefix` | string | `"session-{node}"` | 键前缀 |

### 熔断器

保护 Broker 在存储后端不可用时不会被阻塞。熔断器使用滑动窗口统计调用失败率，
当失败率超过阈值且达到最小调用次数时，熔断器跳转到 OPEN 状态，所有会话存储操作快速失败。

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `circuit_breaker.failure_rate_threshold` | `f64` | `0.25` | 失败率阈值 (0.0–1.0)，超过后跳闸到 OPEN |
| `circuit_breaker.sliding_window_type` | `string` | `"TimeBased"` | 滑动窗口类型：`CountBased` 或 `TimeBased` |
| `circuit_breaker.sliding_window_size` | `usize` | `20` | 滑动窗口大小（调用次数） |
| `circuit_breaker.sliding_window_duration` | `string` | `"45s"` | 滑动窗口持续时间（仅 TimeBased 模式） |
| `circuit_breaker.minimum_number_of_calls` | `usize` | `10` | 熔断器跳闸前的最小调用次数 |
| `circuit_breaker.wait_duration_in_open` | `string` | `"30s"` | OPEN 状态下等待多久后进入探测（HALF_OPEN） |
| `circuit_breaker.slow_call_duration_threshold` | `string` | `"2s"` | 慢调用持续时间阈值 |
| `circuit_breaker.slow_call_rate_threshold` | `f64` | `1.0` | 慢调用率阈值（1.0 = 禁用） |
| `circuit_breaker.operation_timeout` | `string` | `"15s"` | 单次操作超时（`"0s"` 禁用） |

## 依赖

`rmqtt`（feature `plugin`）、`sled`、`redis`、`tokio`

## 许可证

MIT OR Apache-2.0
