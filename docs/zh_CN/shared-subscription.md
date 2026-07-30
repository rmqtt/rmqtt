[English](../en_US/shared-subscription.md) | 简体中文

# 共享订阅

共享订阅使用 `$share/{group}/{topic}` 主题过滤器格式实现负载均衡的消费者组。当多个客户端订阅同一个共享组时，发布的消息只会投递给组中的**某一个**订阅者，从而实现消息消费者的水平扩展。

此功能由 [rmqtt-shared-subscription](https://github.com/rmqtt/rmqtt/tree/master/rmqtt-plugins/rmqtt-shared-subscription) 插件提供。

## 工作原理

共享订阅主题过滤器格式如下：

```
$share/{group}/{topic}
```

- `$share` — 共享订阅前缀
- `{group}` — 任意组名（如 `group1`、`sensors`）
- `{topic}` — 标准 MQTT 主题过滤器（如 `sensor/+/temp`）

订阅了相同 `{group}` 和 `{topic}` 的所有客户端构成一个消费者组。当有消息匹配主题过滤器时，Broker 从组中选择**一个**订阅者进行投递。

## 选择策略

插件支持七种可配置的订阅者选择策略：

| 策略 | 描述 | 适用场景 |
|------|------|----------|
| `random` | 随机选择一个在线订阅者 | 基础负载均衡 |
| `round_robin`（默认） | 使用原子计数器顺序轮转 | 等容量消费者 |
| `round_robin_per_group` | 每个共享组独立轮转 | 大规模集群多组场景 |
| `sticky` | 通过 LRU 缓存将发布者绑定到固定订阅者 | 有状态处理、会话亲和性 |
| `local` | 优先选择与发布者同节点的订阅者 | 减少跨节点流量 |
| `hash_clientid` | 按发布者 ClientId 哈希确定性选择 | 设备级消息有序性 |
| `hash_topic` | 按主题名哈希确定性选择 | 主题分片 |

### sticky 策略

使用 `sticky` 策略时，插件维护一个发布者到订阅者的 LRU 绑定缓存。缓存大小通过 `sticky_cache_size` 配置。缓存满时，最久未使用的绑定会被自动淘汰。

## 配置

#### 插件：

```bash
rmqtt-shared-subscription
```

#### 插件配置文件：

```bash
plugins/rmqtt-shared-subscription.toml
```

#### 插件配置项：

```bash
##--------------------------------------------------------------------
## rmqtt-shared-subscription
##--------------------------------------------------------------------

# 选择策略
# 有效值：random, round_robin, round_robin_per_group, sticky, local, hash_clientid, hash_topic
# 默认值：round_robin
strategy = "round_robin"

# LRU 缓存的最大粘性绑定数（仅适用于 "sticky" 策略）。
# 缓存满时，最久未使用的绑定会被淘汰。
# 默认值：100000
# sticky_cache_size = 100000
```

#### 启用插件

插件加载后，所有监听器自动支持共享订阅。在 `rmqtt.toml` 的 `plugins.default_startups` 列表中添加 `rmqtt-shared-subscription`：

```bash
##--------------------------------------------------------------------
## Plugins
##--------------------------------------------------------------------
plugins.dir = "rmqtt-plugins/"
plugins.default_startups = [
    "rmqtt-shared-subscription",
    # 其他插件...
]
```

不需要在监听器上单独配置。
