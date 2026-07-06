[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-retainer

[![crates.io](https://img.shields.io/crates/v/rmqtt-retainer.svg)](https://crates.io/crates/rmqtt-retainer)

保留消息存储插件。支持 RAM、Sled（嵌入式）和 Redis 三种后端。替换 Broker 默认的 retain 引擎，提供重启后持久化能力。

所有后端均支持单机和集群模式。集群模式下，本地存储后端（ram、sled）使用完整保留同步，Redis 使用轻量级仅主题同步。

## 概述

在 `BeforeStartup` Hook 中注入持久化 retain 存储。后台任务每 10 秒定期清理过期的保留消息。

### 架构说明（v0.22.0+）

- **内存主题 Trie 索引**：启动时通过扫描已存储的保留消息构建。为订阅查询提供快速通配符匹配，替代了之前的 SCAN+MATCH 方式。
- **批量存储**：消息收集后通过 `batch_insert` / `batch_remove` 批量处理，由 `batch_messages_limit` 控制。
- **速率计数器**：跟踪处理吞吐量（通过 `rate-counter` 特性默认启用）。
- **RetainSyncMode**：
  - `Full`：完整保留消息广播（ram、sled）。
  - `TopicOnly`：仅主题名称同步（共享存储的 Redis）。
- **熔断器（Circuit Breaker）**：内置滑动窗口熔断器，检测存储后端故障并自动快速失败降级和恢复。参见下方配置说明。

## 使用方法

### 构建

在 `rmqttd/Cargo.toml` 中添加依赖，或通过 `rmqtt-plugins` 元 crate 启用：

```toml
# 直接依赖
rmqtt-retainer = { version = "0.22", features = ["ram"] }

# 或通过元 crate
rmqtt-plugins = { version = "0.22", features = ["retainer-ram"] }
```

可用的存储后端 Feature 标志：

| Feature | 后端 | 持久化 | 集群支持 |
|---------|------|--------|----------|
| `ram` | 内存 HashMap | 否（重启丢失） | 是 |
| `sled` | Sled 嵌入式数据库（磁盘） | 是 | 是 |
| `redis` | Redis 远程存储 | 是 | 是 |

其他特性：

| Feature | 说明 |
|---------|------|
| `rate-counter` | 启用吞吐量速率计数器（默认启用） |

### 注册

```rust
rmqtt_retainer::register(&scx, true, false).await?;
// 或指定名称：
rmqtt_retainer::register_named(&scx, "rmqtt-retainer", true, false).await?;
```

参数说明：`(scx, default_startup, immutable)`。

## 配置

配置文件：`rmqtt-retainer.toml`（位于插件配置目录）。通过 `scx.plugins.load_config_default::<PluginConfig>("rmqtt-retainer")` 加载。

| 选项 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `storage.type` | `string` | `"ram"` | 存储后端类型：`ram`、`sled`、`redis` |
| `storage.sled.path` | `string` | `"/var/log/rmqtt/.cache/retain/{node}"` | Sled 数据库路径（支持 `{node}` 占位符替换为节点 ID） |
| `storage.sled.cache_capacity` | `string` | `"3G"` | Sled 缓存容量（例如 `"3G"`、`"512MB"`） |
| `storage.redis.url` | `string` | `"redis://127.0.0.1:6379/"` | Redis 连接 URL |
| `storage.redis.prefix` | `string` | `"retain"` | Redis 键前缀 |
| `max_retained_messages` | `u64` | `0`（无限制） | 最大保留消息数。超出后现有消息可被替换，但无法为新主题存储保留消息。 |
| `max_payload_size` | `string` | `"1MB"` | 保留消息的最大 Payload 大小。超出后该消息将被视为普通消息处理。 |
| `retained_message_ttl` | `string` | `"0m"`（不过期） | 保留消息的 TTL。未设置时默认使用消息过期时间。 |
| `batch_messages_limit` | `usize` | `500` | 单次批量存储操作的最大消息数 |
| `circuit_breaker.failure_rate_threshold` | `f64` | `0.25` | 失败率阈值 (0.0–1.0)，超过后熔断器跳闸到 OPEN |
| `circuit_breaker.sliding_window_type` | `string` | `"TimeBased"` | 滑动窗口类型：`CountBased` 或 `TimeBased` |
| `circuit_breaker.sliding_window_size` | `usize` | `20` | 滑动窗口大小（调用次数） |
| `circuit_breaker.sliding_window_duration` | `string` | `"45s"` | 滑动窗口持续时间（仅 TimeBased 模式） |
| `circuit_breaker.minimum_number_of_calls` | `usize` | `10` | 熔断器跳闸前的最小调用次数 |
| `circuit_breaker.wait_duration_in_open` | `string` | `"30s"` | OPEN 状态持续时间，之后进入 HALF_OPEN 状态 |
| `circuit_breaker.slow_call_duration_threshold` | `string` | `"2s"` | 慢调用持续时间阈值 |
| `circuit_breaker.slow_call_rate_threshold` | `f64` | `1.0` | 慢调用率阈值（1.0 = 禁用） |
| `circuit_breaker.operation_timeout` | `string` | `"8s"` | 单次操作超时（`"0s"` 禁用） |

### 配置来源

插件通过 `scx.plugins.load_config_default::<PluginConfig>("rmqtt-retainer")` 加载配置，支持以下来源：

1. `{plugins.dir}/rmqtt-retainer.toml`（文件，可选——文件缺失时使用默认值）
2. `rmqtt_plugin_rmqtt_retainer_*` 环境变量（将 TOML 键映射为带下划线前缀的环境变量）
3. 通过 `ServerContext::plugins_config_map_add()` 内联配置

### 示例

```toml
# RAM 模式（默认）
storage.type = "ram"

# Sled 模式（持久化）
storage.type = "sled"
storage.sled.path = "/var/log/rmqtt/.cache/retain/{node}"
storage.sled.cache_capacity = "3G"

# Redis 模式（持久化，支持集群）
storage.type = "redis"
storage.redis.url = "redis://127.0.0.1:6379/"
storage.redis.prefix = "retain"

# 限制
max_retained_messages = 10000
max_payload_size = "1MB"
retained_message_ttl = "24h"
batch_messages_limit = 500

# 熔断器（滑动窗口模型，注释掉时使用默认值）
#circuit_breaker.failure_rate_threshold = 0.25
#circuit_breaker.sliding_window_type = "TimeBased"
#circuit_breaker.sliding_window_size = 20
#circuit_breaker.sliding_window_duration = "45s"
#circuit_breaker.minimum_number_of_calls = 10
#circuit_breaker.wait_duration_in_open = "30s"
#circuit_breaker.slow_call_duration_threshold = "2s"
#circuit_breaker.slow_call_rate_threshold = 1.0
#circuit_breaker.operation_timeout = "8s"
```

## 依赖

`rmqtt`（features: `plugin`、`retain`）、`rmqtt-storage`、`serde`、`tokio`、`sled`（可选）、`redis`（可选）

## 许可证

MIT OR Apache-2.0
