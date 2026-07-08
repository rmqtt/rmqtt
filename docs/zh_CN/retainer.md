[English](../en_US/retainer.md)  | 简体中文

# 保留消息

客户端发布消息时设置了**retain**标记，消息将被保留。然后当客户端订阅此消息匹配的主题过滤器时，将收到此保留消息。

**RMQTT 0.4.0**及之后版本默认将关闭**保留消息**功能。开始**保留消息**功能需要打开**rmqtt-retainer**插件和**listener.tcp.\<xxxx\>.retain_available**配置项。

注意：**RMQTT 0.11.0**及之后版本已经移除：**listener.tcp.\<xxxx\>.retain_available**配置项

#### 插件：

```bash
rmqtt-retainer
```

#### 插件配置文件：

```bash
plugins/rmqtt-retainer.toml
```

#### 插件配置项：

```bash
##--------------------------------------------------------------------
## rmqtt-retainer
##--------------------------------------------------------------------
#
# Single node mode         - ram, sled, redis
# Multi-node cluster mode  - ram, sled, redis
#

##ram, sled, redis
storage.type = "ram"

##sled
storage.sled.path = "/var/log/rmqtt/.cache/retain/{node}"
storage.sled.cache_capacity = "3G"

##redis
storage.redis.url = "redis://127.0.0.1:6379/"
storage.redis.prefix = "retain"

# The maximum number of retained messages, where 0 indicates no limit. After the number of reserved messages exceeds
# the maximum limit, existing reserved messages can be replaced, but reserved messages cannot be stored for new topics.
max_retained_messages = 0

# The maximum Payload value for retaining messages. After the Payload size exceeds the maximum value, the RMQTT
# message server will process the received reserved message as a regular message.
max_payload_size = "1MB"

# TTL for retained messages. Set to 0 for no expiration.
# If not specified, the message expiration time will be used by default.
retained_message_ttl = "0m"

# Maximum number of messages per batch (default: 500).
batch_messages_limit = 500

##All circuit-breaker parameters (failure rate, window, etc.) are inherited
##from the global `[circuit_breaker]` section in `rmqtt.toml`.
##Only the backend operation timeout can be overridden here.
##
##Backend storage operation timeout. If a storage call exceeds this duration
##it is aborted and counted as a failure by the circuit breaker.
##Set to "0s" to disable.
##Default: "8s"
#backend_timeout = "8s"
```

当前支持 "ram"、"sled" 和 "redis" 三种存储模式。"ram" 是存储在内存。"sled" 是存储在本地磁盘，需要配置存储位置和在内存中的缓存容量，适当大小可以提高读写效率。
"redis" 存储当前仅支持单节点。{node} 将被替换为当前节点标识。

另外，"max_retained_messages"：可以配置最大保留消息数量，`0` 表示无限制；"max_payload_size"：限制消息负载大小；"retained_message_ttl"
配置保留消息过期时间，`"0m"` 表示不过期，如果未指定，则默认情况下将使用消息过期时间。

"batch_messages_limit" 限制单次批量存储操作处理的最大消息数（默认值：500）。当大量保留消息同时到达时，会按此大小分组进行批量处理以提高效率。

熔断器（Circuit Breaker）参数继承自主配置文件 `rmqtt.toml` 中的全局 `[circuit_breaker]` 配置段（失败率阈值、滑动窗口类型/大小、最小调用次数、OPEN 持续时间、慢调用阈值等）。插件仅允许覆盖存储后端操作超时 `backend_timeout`（默认值：`"8s"`，设为 `"0s"` 可禁用超时）。

如果 RMQTT 部署为单机模式，"ram"、"sled" 和 "redis" 都是支持的。集群模式下同样支持所有三种存储模式；
对于 "redis" 模式，使用轻量级的仅主题同步机制来减少节点间流量。

### 架构说明（v0.22.0+）

从 RMQTT **0.22.0** 版本开始，保留消息插件采用了以下关键优化：

- **内存主题 Trie 索引**：启动时通过扫描所有已存储的保留消息，在内存中构建 `RetainTree`。该索引在客户端订阅时提供快速的通配符匹配，
  替代了之前的 SCAN+MATCH 方式。随着消息的设置或移除，该 Trie 索引会增量更新。

- **批量存储**：消息收集到通道后，通过 `batch_insert` / `batch_remove` 操作进行批量处理（由 `batch_messages_limit` 控制），
  在高负载下显著提高吞吐量。

- **速率计数器**：当使用 `rate-counter` 特性（默认启用）构建时，插件会跟踪消息处理吞吐量，用于监控和调试。

- **RetainSyncMode**：存储后端上报集群同步所需的模式，决定重同步的方式：
  - `Full`（完整同步）：用于本地存储后端（ram、sled）—— 将完整保留消息广播到所有节点。
  - `TopicOnly`（仅主题同步）：用于共享存储后端（redis）—— 仅广播主题名称，每个节点直接从共享存储读取保留数据并更新其本地内存主题索引。

- **熔断器（Circuit Breaker）**：集成到 Retainer 存储层，检测存储后端故障并快速失败保留操作，防止级联节点故障。参见下方配置说明。

- **Retain 引擎 API**：向 `RetainStorage` trait 添加了 `retain_sync_mode()` 和 `sync_retain_topic()` 方法，使存储后端能够声明其集群同步策略并处理来自对等节点的仅主题同步通知。


该插件现在默认已**启用**。要验证或更改此设置，请检查主配置文件 `rmqtt.toml` 中的 `plugins.default_startups` 配置：
```bash
##--------------------------------------------------------------------
## Plugins
##--------------------------------------------------------------------
#Plug in configuration file directory
plugins.dir = "rmqtt-plugins/"
#Plug in started by default, when the mqtt server is started
plugins.default_startups = [
    "rmqtt-retainer",
    #"rmqtt-auth-http",
    #"rmqtt-cluster-broadcast",
    #"rmqtt-cluster-raft",
    #"rmqtt-sys-topic",
    #"rmqtt-message-storage",
    #"rmqtt-session-storage",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```
