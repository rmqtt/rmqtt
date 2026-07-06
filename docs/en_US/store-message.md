English | [简体中文](../zh_CN/store-message.md)


# Store unexpired messages

Messages that are published will be stored until they expire. As long as a message remains unexpired, any subscriptions made to the corresponding topic after the message is published will be forwarded. Messages will be automatically cleared once they expire.

#### Plugin:

```bash
rmqtt-message-storage
```

#### Plugin configuration file:

```bash
plugins/rmqtt-message-storage.toml
```

#### Plugin configuration options:

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
##Default: 300000
#storage.ram.queue_max = 300_000

##redis
storage.redis.url = "redis://127.0.0.1:6379/"
storage.redis.prefix = "message-{node}"

##redis-cluster
storage.redis-cluster.urls = ["redis://127.0.0.1:6380/", "redis://127.0.0.1:6381/", "redis://127.0.0.1:6382/"]
storage.redis-cluster.prefix = "message-{node}"

##Quantity of expired messages cleared during each cleanup cycle.
cleanup_count = 5000

##Timeout for storage I/O operations. 0 = no timeout.
timeout = "5s"

##─── Circuit breaker ─────────────────────────────────────────────────────────
##Optional circuit-breaker tuning.
##When this section is absent (or commented out), the built-in
##CircuitBreakerConfig::default() is used verbatim.
##Only the fields you explicitly set override the defaults.
##
##Failure rate threshold (0.0 – 1.0). Default: 0.25
##When the sliding-window failure rate exceeds this value, the
##circuit trips to OPEN and all storage operations fast-fail.
#circuit_breaker.failure_rate_threshold = 0.25
##
##Sliding window type: "CountBased" or "TimeBased". Default: "TimeBased"
#circuit_breaker.sliding_window_type = "TimeBased"
##
##Sliding window size (number of calls). Default: 20
#circuit_breaker.sliding_window_size = 20
##
##Sliding window duration for TimeBased mode.
##Only used when sliding_window_type is "TimeBased".
##Calls older than this duration are excluded from the failure rate.
##Default: "45s"
#circuit_breaker.sliding_window_duration = "45s"
##
##Minimum number of calls before the breaker can trip.
##Prevents premature tripping during low-traffic periods.
##Default: 10
#circuit_breaker.minimum_number_of_calls = 10
##
##Duration in OPEN state before transitioning to HALF_OPEN (probe).
##Default: "30s"
#circuit_breaker.wait_duration_in_open = "30s"
##
##Slow call duration threshold. Default: "2s"
#circuit_breaker.slow_call_duration_threshold = "2s"
##
##Slow call rate threshold (0.0 – 1.0). 1.0 = disabled. Default: 1.0
#circuit_breaker.slow_call_rate_threshold = 1.0
##
##Per-operation timeout. Set to "0s" to disable. Default: "8s"
#circuit_breaker.operation_timeout = "8s"
```

Currently, three storage engines are supported: "ram," "redis," and "redis-cluster." "ram" is stored in local memory and 
can be configured with maximum memory usage or maximum message count, and it can specify whether messages should be encoded 
before storage. Prefix configuration allows different rmqtt nodes to use the same Redis storage service. `{node}` will be 
replaced by the identifier of the current node.

`timeout` configures the timeout for storage I/O operations. Set to `0` for no timeout.

The **Circuit Breaker** prevents cascading failures when the storage backend becomes unavailable. The breaker uses a sliding window to track the call failure rate. When the failure rate exceeds `circuit_breaker.failure_rate_threshold` (default: 0.25) and the minimum number of calls `circuit_breaker.minimum_number_of_calls` (default: 10) has been reached, the circuit trips to **OPEN** state and all storage operations fast-fail. After `circuit_breaker.wait_duration_in_open` (default: `"30s"`) the circuit transitions to **HALF_OPEN** for probe testing. `circuit_breaker.slow_call_duration_threshold` (default: `"2s"`) can detect slow calls, and `circuit_breaker.operation_timeout` (default: `"8s"`) sets a per-operation timeout.


By default, this plugin is not enabled. To activate it, you must add the `rmqtt-message-storage` entry to the
`plugins.default_startups` configuration in the main configuration file `rmqtt.toml`, as shown below:
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










