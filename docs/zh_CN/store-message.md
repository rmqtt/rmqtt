[English](../en_US/store-message.md)  | 简体中文

# 存储未过期消息

发布的消息在过期之前将被存储。只要消息未过期，在消息发布之后发起的对此消息主题的订阅都会被转发。消息过期后会被自动清除。

#### 插件：

```bash
rmqtt-message-storage
```

#### 插件配置文件：

```bash
plugins/rmqtt-message-storage.toml
```

#### 插件配置项：

```bash
##--------------------------------------------------------------------
## rmqtt-message-storage
##--------------------------------------------------------------------

##ram, redis, redis-cluster
storage.type = "ram"

##ram
storage.ram.cache_capacity = "3G"
storage.ram.cache_max_count = 1_000_000
storage.ram.encode = false

##Maximum pending messages in the in-memory channel (back-pressure limit).
##默认值: 300000
#storage.ram.queue_max = 300_000

##redis
storage.redis.url = "redis://127.0.0.1:6379/"
storage.redis.prefix = "message-{node}"

##redis-cluster
storage.redis-cluster.urls = ["redis://127.0.0.1:6380/", "redis://127.0.0.1:6381/", "redis://127.0.0.1:6382/"]
storage.redis-cluster.prefix = "message-{node}"

##Quantity of expired messages cleared during each cleanup cycle.
cleanup_count = 5000

##Timeout for storage I/O operations, channel sends, and circuit breaker
##per-operation timeout. 0 = no timeout. Examples: "5s", "500ms".
##Default: "15s"
#backend_timeout = "15s"

##─── Circuit breaker ────────────────────────────────────────────────────────
##All circuit-breaker parameters (failure rate, window, etc.) are inherited
##from the global `[circuit_breaker]` section in `rmqtt.toml`.
##The per-operation timeout uses `backend_timeout` above.
```

当前支持"ram"、"redis"和"redis-cluster"三种存储引擎。"ram"是存储在本地内存，可以配置最大使用内存容量或最大消息数量，以及可以指示消息是否编码后再存储。
前缀配置方便不同rmqtt节点使用同一套redis存储服务。{node}将被替换为当前节点标识。

`backend_timeout`（默认值：`"15s"`）配置存储 I/O 操作、channel 发送及熔断器单次操作的超时时间，设为 `"0s"` 表示不超时。

熔断器（Circuit Breaker）参数继承自主配置文件 `rmqtt.toml` 中的全局 `[circuit_breaker]` 配置段（失败率阈值、滑动窗口类型/大小、最小调用次数、OPEN 持续时间、慢调用阈值等）。单次操作超时使用上述 `backend_timeout` 设置。

默认情况下并没有启动此插件，如果要开启此插件，必须在主配置文件“rmqtt.toml”中的“plugins.default_startups”配置中添加“rmqtt-message-storage”项，如：
```bash
##--------------------------------------------------------------------
## Plugins
##--------------------------------------------------------------------
#Plug in configuration file directory
plugins.dir = "rmqtt-plugins/"
#Plug in started by default, when the mqtt server is started
plugins.default_startups = [
    #"rmqtt-retainer",
    #"rmqtt-auth-http",
    #"rmqtt-cluster-broadcast",
    #"rmqtt-cluster-raft",
    #"rmqtt-sys-topic",
    "rmqtt-message-storage",
    #"rmqtt-session-storage",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```










