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
| `storage.ram.cache_max_count` | integer | `1_000_000` | 最大缓存条目数 |
| `storage.ram.encode` | boolean | `true` | 启用消息编码 |

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

## 依赖

`rmqtt`（features：`plugin`、`msgstore`）、redis（可选）

## 许可证

MIT OR Apache-2.0
