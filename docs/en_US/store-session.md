English | [简体中文](../zh_CN/store-session.md)

# Store session information

Connection information, subscription relationships, offline messages, and inflight messages will be stored.

Upon successful connection, "Connection Information" will be stored. After each successful subscription, "Subscription 
Relationships" will be stored. During the duration of the session connection, the last operation time will be periodically 
refreshed. In the event of a session disconnection, inflight messages will be stored. During the period of disconnection 
but before expiration, offline messages will be stored.

Upon restart of the RMQTT service node, non-expired session basic information and subscription relationships will be 
loaded, and non-expired offline messages and inflight messages will be forwarded. If the session has already expired, 
all information will be discarded.

#### Plugins:

```bash
rmqtt-session-storage
```

#### Plugin configuration file:

```bash
plugins/rmqtt-session-storage.toml
```

#### Plugin configuration options:

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
##Uncomment the following section to customize circuit-breaker behaviour.
##When the whole section is absent, the upstream default is used verbatim.

##Failure rate threshold (0.0 – 1.0). When exceeded, circuit opens.
##Default: 0.25
#circuit_breaker.failure_rate_threshold = 0.25

##Sliding window type: "CountBased" or "TimeBased".
##  CountBased — window slides after N calls (sliding_window_size).
##  TimeBased  — window slides after a fixed duration (e.g. 45s).
##Default: "TimeBased"
#circuit_breaker.sliding_window_type = "TimeBased"

##Sliding window size (number of calls) for CountBased type.
##For TimeBased, this value sets the max tracked calls (fallback for minimum_number_of_calls).
##Default: 20
#circuit_breaker.sliding_window_size = 20

##Sliding window duration for TimeBased type (e.g. "45s").
##Only used when sliding_window_type is "TimeBased".
##Default: "45s"
#circuit_breaker.sliding_window_duration = "45s"

##Minimum calls before the breaker can trip.
##Default: 10
#circuit_breaker.minimum_number_of_calls = 10

##Duration in OPEN state before transitioning to HALF_OPEN (probe).
##Default: "30s"
#circuit_breaker.wait_duration_in_open = "30s"

##Slow call duration threshold.
##Default: "2s"
#circuit_breaker.slow_call_duration_threshold = "2s"

##Slow call rate threshold (0.0 – 1.0). 1.0 = disabled.
##Default: 1.0
#circuit_breaker.slow_call_rate_threshold = 1.0

##Per-operation timeout. Set to "0s" to disable.
##Default: "15s"
#circuit_breaker.operation_timeout = "15s"
```

Currently, three storage engines are supported: "sled," "redis," and "redis-cluster." "sled" is stored locally, requiring 
configuration for the storage location and in-memory cache size, with an appropriate size improving read and write efficiency.
Prefix configuration enables different rmqtt nodes to use the same Redis storage service. `{node}` will be replaced with 
the current node identifier.

The **Circuit Breaker** prevents cascading failures when the storage backend becomes unavailable. The breaker uses a sliding window to track the call failure rate. When the failure rate exceeds `circuit_breaker.failure_rate_threshold` (default: 0.25) and the minimum number of calls `circuit_breaker.minimum_number_of_calls` (default: 10) has been reached, the circuit trips to **OPEN** state and all session storage operations fast-fail. After `circuit_breaker.wait_duration_in_open` (default: `"30s"`) the circuit transitions to **HALF_OPEN** for probe testing. `circuit_breaker.operation_timeout` (default: `"15s"`) sets a per-operation timeout.


By default, this plugin is not enabled. To activate it, you must add the `rmqtt-session-storage` entry to the
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
    #"rmqtt-message-storage",
    "rmqtt-session-storage",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```





