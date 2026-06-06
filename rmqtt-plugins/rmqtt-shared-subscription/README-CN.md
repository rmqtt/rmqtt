[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-shared-subscription

[![crates.io](https://img.shields.io/crates/v/rmqtt-shared-subscription.svg)](https://crates.io/crates/rmqtt-shared-subscription)

RMQTT 共享订阅插件。为 `$share/{group}/{topic}` 订阅提供多种订阅者选择策略。

## 概述

当多个客户端订阅同一个共享订阅组主题时（例如 `$share/group1/sensor/#`），发布的消息只会投递到组中的**某一个**订阅者。这使得 MQTT 能够支持负载均衡的消费者组模式，广泛应用于物联网和 MQTT 流式数据场景。

本插件注册了一个 [`SharedSubscription`](https://docs.rs/rmqtt/latest/rmqtt/subscribe/trait.SharedSubscription.html) trait 实现，将订阅者选择逻辑委托给七种可配置的策略之一。

## 策略

| 策略 | 枚举值 | 描述 | 适用场景 |
|------|--------|------|----------|
| `random` | `Random` | 每次随机选择一个在线订阅者 | 基础负载均衡 |
| `round_robin`（默认） | `RoundRobin` | 全局原子计数器轮转 | 等容量消费者 |
| `round_robin_per_group` | `RoundRobinPerGroup` | 每个共享组独立计数轮转 | 大规模集群多组场景 |
| `sticky` | `Sticky` | 通过 LRU 缓存将发布者绑定到固定订阅者 | 有状态处理、会话亲和性 |
| `local` | `Local` | 优先选择与发布者同节点的订阅者 | 减少跨节点流量 |
| `hash_clientid` | `HashClientId` | 按发布者 ClientId 哈希选择 | 设备级消息有序性 |
| `hash_topic` | `HashTopic` | 按主题名哈希选择 | 主题分片 |

### 策略详情

**random** — 随机选取一个在线订阅者。若无在线订阅者，则回退到同节点订阅者。

**round_robin** — 使用原子计数器在所有订阅者间顺序轮转。优先选择在线订阅者，跳过离线节点。

**round_robin_per_group** — 与 round_robin 类似，但每个共享组维护独立的计数器，组间互不影响。

**sticky** — 首次为 `(共享组, 发布者)` 选定订阅者后，将该绑定关系缓存并复用。若被绑定的订阅者离线，则清除绑定并新建。缓存使用 LRU 淘汰策略——参见下方 `sticky_cache_size` 配置。

**local** — 优先选择与发布者同 Broker 节点的订阅者。若无本地在线订阅者，则回退到跨节点在线订阅者，最后回退到本地（即使离线）订阅者。

**hash_clientid** — 使用 `std::hash::DefaultHasher` 对发布者的 ClientId 进行哈希，确定性地选择订阅者。若该订阅者离线，则回退到随机选择。

**hash_topic** — 与 `hash_clientid` 类似，但对主题名进行哈希，确保同一主题的消息总是投递给同一订阅者。

## 使用

### 启用插件

在 `rmqtt.toml` 的 `plugins.default_startups` 列表中添加：

```toml
plugins.default_startups = [
    "rmqtt-shared-subscription",
    # 其他插件...
]
```

插件通过 `plugins.default_startups` 在启动时注册。无需在每个监听器上单独配置——所有监听器自动支持共享订阅。

### 代码注册（可选）

```rust
rmqtt_shared_subscription::register(register_fn);
```

## 配置

文件：`rmqtt-shared-subscription.toml`

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `strategy` | 字符串 | `"round_robin"` | 选择策略——见上方策略表格 |
| `sticky_cache_size` | 整数 | `100000` | LRU 缓存的最大粘性绑定数（仅 `sticky` 策略生效） |

### 示例

```toml
# 选择策略
# 有效值：random, round_robin, round_robin_per_group, sticky, local, hash_clientid, hash_topic
strategy = "round_robin"

# LRU 缓存的最大粘性绑定数（仅适用于 "sticky" 策略）。
# 缓存满时，最久未使用的绑定会被淘汰。
sticky_cache_size = 100000
```

### 运行时指标

插件通过 `attrs()` 方法（插件系统的 HTTP API 使用）暴露内部状态：

| 字段 | 描述 | 策略条件 |
|------|------|----------|
| `strategy` | 当前策略名称 | 始终存在 |
| `rr_counter` | 全局轮转计数器值 | `round_robin`, `round_robin_per_group` |
| `rr_group_count` | 跟踪的组计数器数量 | `round_robin`, `round_robin_per_group` |
| `rr_group_details` | 各组计数器明细列表（最多 1000 条） | `round_robin`, `round_robin_per_group` |
| `sticky_binding_count` | 活跃的粘性绑定数 | `sticky` |
| `sticky_binding_details` | 粘性绑定明细列表（最多 1000 条） | `sticky` |

## MQTT 协议

共享订阅使用 `$share/{group}/{topic}` 主题过滤器格式：

- `$share/group1/sensor/#` — 订阅 `sensor/` 下所有主题，组名为 "group1"
- 匹配该主题过滤器的消息只会投递给 "group1" 中的**一个**订阅者

## 依赖

- `rmqtt`（features `plugin`, `shared-subscription`）
- `lru` 0.18 — 粘性绑定用 LRU 缓存

## 许可证

MIT OR Apache-2.0
