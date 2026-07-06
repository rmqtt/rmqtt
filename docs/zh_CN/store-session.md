[English](../en_US/store-session.md)  | 简体中文

# 存储会话信息

连接信息，订阅关系，离线消息，飞行消息将被存储。

当连接成功后存储“连接信息”，每次订阅成功后存储“订阅关系”，会话连接持续期间会定期刷新最后操作时间，会话断开连接时会存储飞行中的消息，在会话断连但未过期期间
会存储离线消息。

当RMQTT服务节点重启时会载入未过期会话基本信息和订阅关系，转发未过期离线消息和飞行消息。如果会话已经过期，将丢弃所有信息。

#### 插件：

```bash
rmqtt-session-storage
```

#### 插件配置文件：

```bash
plugins/rmqtt-session-storage.toml
```

#### 插件配置项：

```bash
##--------------------------------------------------------------------
## rmqtt-session-storage
##--------------------------------------------------------------------

##sled, redis, redis-cluster
storage.type = "sled"

##sled
storage.sled.path = "/var/log/rmqtt/.cache/session/{node}"
storage.sled.cache_capacity = "3G"

##redis
storage.redis.url = "redis://127.0.0.1:6379/"
storage.redis.prefix = "session-{node}"

##redis-cluster
storage.redis-cluster.urls = ["redis://127.0.0.1:6380/", "redis://127.0.0.1:6381/", "redis://127.0.0.1:6382/"]
storage.redis-cluster.prefix = "session-{node}"

##─── Circuit breaker ─────────────────────────────────────────────────────────
##可选熔断器调优。当此段落缺失时，直接使用上游默认值。
##仅显式设置的字段会覆盖默认值。

##失败率阈值 (0.0 – 1.0)。超过此值时熔断器打开。
##默认值: 0.25
#circuit_breaker.failure_rate_threshold = 0.25

##滑动窗口类型："CountBased" 或 "TimeBased"。
##  CountBased — 窗口在 N 次调用后滑动 (sliding_window_size)。
##  TimeBased  — 窗口在固定持续时间后滑动 (如 45s)。
##默认值: "TimeBased"
#circuit_breaker.sliding_window_type = "TimeBased"

##CountBased 类型的滑动窗口大小（调用次数）。
##对于 TimeBased，此值设置最大跟踪调用数。
##默认值: 20
#circuit_breaker.sliding_window_size = 20

##TimeBased 类型的滑动窗口持续时间 (如 "45s")。
##仅当 sliding_window_type 为 "TimeBased" 时使用。
##默认值: "45s"
#circuit_breaker.sliding_window_duration = "45s"

##熔断器跳闸前的最小调用次数。
##默认值: 10
#circuit_breaker.minimum_number_of_calls = 10

##OPEN 状态持续时间，过后转入 HALF_OPEN（探针）状态。
##默认值: "30s"
#circuit_breaker.wait_duration_in_open = "30s"

##慢调用持续时间阈值。
##默认值: "2s"
#circuit_breaker.slow_call_duration_threshold = "2s"

##慢调用率阈值 (0.0 – 1.0)。1.0 = 禁用。
##默认值: 1.0
#circuit_breaker.slow_call_rate_threshold = 1.0

##单次操作超时。设为 "0s" 禁用。
##默认值: "15s"
#circuit_breaker.operation_timeout = "15s"
```

当前支持"sled"、"redis"和"redis-cluster"三种存储引擎。"sled"是存储在本地，需要配置存储位置和在内存中的缓存容量，适当大小可以提高读写效率。
前缀配置方便不同rmqtt节点使用同一套redis存储服务。{node}将被替换为当前节点标识。

**熔断器（Circuit Breaker）** 防止存储后端不可用时发生级联故障。熔断器使用滑动窗口统计调用失败率，当失败率超过 `circuit_breaker.failure_rate_threshold`（默认值：0.25）且达到 `circuit_breaker.minimum_number_of_calls`（默认值：10）最小调用次数时，熔断器进入 **OPEN** 状态，所有会话存储操作快速失败。经过 `circuit_breaker.wait_duration_in_open`（默认值：`"30s"`）后进入 **HALF_OPEN** 状态进行探针探测。`circuit_breaker.operation_timeout`（默认值：`"15s"`）为每个操作设置超时。

默认情况下并没有启动此插件，如果要开启此插件，必须在主配置文件“rmqtt.toml”中的“plugins.default_startups”配置中添加“rmqtt-session-storage”项，如：
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
    #"rmqtt-message-storage",
    "rmqtt-session-storage",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```










