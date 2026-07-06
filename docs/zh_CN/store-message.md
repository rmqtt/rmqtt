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

##存储 I/O 操作超时。0 = 不超时。
timeout = "5s"

##─── Circuit breaker ─────────────────────────────────────────────────────────
##可选熔断器调优。
##当此段落缺失（或注释掉）时，直接使用内置 CircuitBreakerConfig::default() 默认值。
##仅显式设置的字段会覆盖默认值。
##
##失败率阈值 (0.0 – 1.0)。默认值: 0.25
##当滑动窗口内失败率超过此值时，熔断器跳转到 OPEN 状态，所有存储操作快速失败。
#circuit_breaker.failure_rate_threshold = 0.25
##
##滑动窗口类型："CountBased" 或 "TimeBased"。默认值: "TimeBased"
#circuit_breaker.sliding_window_type = "TimeBased"
##
##滑动窗口大小（调用次数）。默认值: 20
#circuit_breaker.sliding_window_size = 20
##
##TimeBased 模式下的滑动窗口持续时间。
##仅当 sliding_window_type 为 "TimeBased" 时使用。
##超过此时间的调用将从失败率计算中排除。
##默认值: "45s"
#circuit_breaker.sliding_window_duration = "45s"
##
##熔断器跳闸前的最小调用次数。
##防止低流量时段过早跳闸。
##默认值: 10
#circuit_breaker.minimum_number_of_calls = 10
##
##OPEN 状态持续时间，过后转入 HALF_OPEN（探针）状态。
##默认值: "30s"
#circuit_breaker.wait_duration_in_open = "30s"
##
##慢调用持续时间阈值。默认值: "2s"
#circuit_breaker.slow_call_duration_threshold = "2s"
##
##慢调用率阈值 (0.0 – 1.0)。1.0 = 禁用。默认值: 1.0
#circuit_breaker.slow_call_rate_threshold = 1.0
##
##单次操作超时。设为 "0s" 禁用。默认值: "8s"
#circuit_breaker.operation_timeout = "8s"
```

当前支持"ram"、"redis"和"redis-cluster"三种存储引擎。"ram"是存储在本地内存，可以配置最大使用内存容量或最大消息数量，以及可以指示消息是否编码后再存储。
前缀配置方便不同rmqtt节点使用同一套redis存储服务。{node}将被替换为当前节点标识。

`timeout` 配置存储 I/O 操作的超时时间，设为 `0` 表示不超时。

**熔断器（Circuit Breaker）** 防止存储后端不可用时发生级联故障。熔断器使用滑动窗口统计调用失败率，当失败率超过 `circuit_breaker.failure_rate_threshold`（默认值：0.25）且达到 `circuit_breaker.minimum_number_of_calls`（默认值：10）最小调用次数时，熔断器进入 **OPEN** 状态，所有存储操作快速失败。经过 `circuit_breaker.wait_duration_in_open`（默认值：`"30s"`）后进入 **HALF_OPEN** 状态进行探针探测。`circuit_breaker.slow_call_duration_threshold`（默认值：`"2s"`）可检测慢调用，`circuit_breaker.operation_timeout`（默认值：`"8s"`）为每个操作设置超时。

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










