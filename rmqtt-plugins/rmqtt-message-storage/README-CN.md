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

保护 Broker 在 Redis 后端不可用时不会被阻塞。电路 OPEN 时，
store/mark_forwarded/get 会立即返回，不碰 Redis。熔断器会自动探测恢复。

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `circuit_breaker_enabled` | boolean | `true` | 启用熔断器 |
| `circuit_failure_threshold` | integer | `10` | 连续失败次数超过此值后跳闸到 OPEN |
| `circuit_reset_timeout` | string | `"15s"` | OPEN 状态下等待多久后进入探测（HALF_OPEN） |
| `circuit_half_open_success_threshold` | integer | `3` | HALF_OPEN 状态下连续成功次数达到此值后关闭电路 |

#### 状态机

```
CLOSED ── 失败次数 ≥ 阈值 ──► OPEN ── 超时 ──► HALF_OPEN
  ▲                                                    │
  └─────────── 成功次数 ≥ 阈值 ◄────────────────────────┘
```

- **CLOSED**：正常运行，统计失败次数。
- **OPEN**：所有操作跳过 Redis；worker 直接丢弃积压消息不等待。
- **HALF_OPEN**：允许探测请求；连续成功达到阈值后关闭，任意失败立即回到 OPEN。

## 依赖

`rmqtt`（features：`plugin`、`msgstore`）、redis（可选）

## 许可证

MIT OR Apache-2.0
