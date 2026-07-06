[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-message-storage

[![crates.io](https://img.shields.io/crates/v/rmqtt-message-storage.svg)](https://crates.io/crates/rmqtt-message-storage)

消息持久化插件。存储未过期的离线客户端消息。

## 使用

```toml
[dependencies]
rmqtt-message-storage = { version = "0.21", features = ["ram"] }
# 或：features = ["redis", "redis-cluster"]
```

```rust
rmqtt_message_storage::register(&scx, true, false).await?;
```

## 配置

文件：`rmqtt-message-storage.toml`

### 存储类型

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `storage.type` | string | `"ram"` | 后端存储类型：`ram`、`redis` 或 `redis-cluster` |

### RAM 后端

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `storage.ram.cache_capacity` | string | `"3G"` | 内存缓存容量 |
| `storage.ram.cache_max_count` | integer | `1_000_000` | 最大缓存条目数（无限制） |
| `storage.ram.encode` | boolean | `false` | 启用消息编码 |
| `storage.ram.queue_max` | integer | `300000` | 任务队列最大积压量（背压保护） |

### Redis 后端

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `storage.redis.url` | string | `"redis://127.0.0.1:6379/"` | Redis 服务器 URL |
| `storage.redis.prefix` | string | `"message-{node}"` | 键前缀（`{node}` = 节点 ID 占位符） |

### Redis 集群后端

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `storage.redis-cluster.urls` | array of string | `["redis://127.0.0.1:6380/", ...]` | Redis 集群节点 URL |
| `storage.redis-cluster.prefix` | string | `"message-{node}"` | 键前缀 |

### 清理

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `cleanup_count` | integer | `5000` | 每轮清理的过期消息数量 |
| `timeout` | string | `"5s"` | 存储 I/O 操作超时。`"0s"` = 不超时 |

### 熔断器

保护 Broker 在存储后端不可用时不会被阻塞。熔断器使用滑动窗口统计调用失败率，
当失败率超过阈值且达到最小调用次数时，熔断器跳转到 OPEN 状态，所有存储操作快速
失败。熔断器会自动探测恢复。

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
| `circuit_breaker.operation_timeout` | `string` | `"8s"` | 单次操作超时（`"0s"` 禁用） |

#### 状态机

```
CLOSED ── 失败率 ≥ 阈值 (满足最小调用数) ──► OPEN ── 等待时间 ──► HALF_OPEN
  ▲                                                                     │
  └─────────────────── 探测成功 ◄───────────────────────────────────────┘
```

- **CLOSED**：正常运行；调用在滑动窗口中被跟踪。
- **OPEN**：所有操作快速失败，不访问存储后端。
- **HALF_OPEN**：允许有限数量的探测请求；成功则关闭熔断器，失败则重新打开。

## 依赖

`rmqtt`（features：`plugin`、`msgstore`）、redis（可选）

## 许可证

MIT OR Apache-2.0
