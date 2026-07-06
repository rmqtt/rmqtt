English | [简体中文](../zh_CN/retainer.md)


# Retain Message


When the client sets the **retain** flag while publishing a message, the message will be retained.
Then, when the client subscribes to a topic filter that matches this message, the retained message will be received.

Starting from **RMQTT 0.4.0**, the **Retain Message** feature will be disabled by default.
Enabling the **Retain Message** feature requires activating the **rmqtt-retainer** plugin and configuring the **listener.tcp.\<xxxx\>.retain_available** option.

**Note:** Starting from **RMQTT 0.11.0**, the configuration item **listener.tcp.\<xxxx\>.retain_available** has been **removed**.

#### Plugins:

```bash
rmqtt-retainer
```

#### Plugin configuration file:

```bash
plugins/rmqtt-retainer.toml
```

#### Plugin configuration options:

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

##─── Circuit breaker ──────────────────────────────────────────────
##Optional circuit-breaker tuning.
##When this section is absent (or commented out), the built-in
##CircuitBreakerConfig::default() is used verbatim.
##Only the fields you explicitly set override the defaults.
##
##Failure rate threshold (0.0 – 1.0). Default: 0.25
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

Currently, three storage modes are supported: "ram", "sled", and "redis".
"ram" storage mode stores data in memory. "sled" storage mode stores data on the local disk and requires configuration of 
the storage location and cache capacity in memory. A suitable size can improve read/write efficiency. "redis" storage 
mode currently supports only single node. {node} will be replaced with the current node identifier.


Additionally, "max_retained_messages" can be configured to set the maximum number of retained messages, where `0` indicates 
no limit; "max_payload_size" limits the size of message payloads; "retained_message_ttl" configures the expiration 
time for retained messages. A value of `"0m"` means no expiration. If not specified, the message expiration time will be used by default.

"batch_messages_limit" limits the maximum number of messages processed in a single batch store operation (default: 500).
When a large number of retained messages arrive at once, they are grouped into batches of this size for efficient processing.

The **Circuit Breaker** prevents cascading failures when the storage backend becomes unavailable. The breaker uses a sliding window to track the call failure rate. When the failure rate exceeds `circuit_breaker.failure_rate_threshold` (default: 0.25, i.e. 25%) and the minimum number of calls `circuit_breaker.minimum_number_of_calls` (default: 10) has been reached, the circuit trips to **OPEN** state and all retain operations fast-fail without touching the storage. After `circuit_breaker.wait_duration_in_open` (default: `"30s"`), the circuit transitions to **HALF_OPEN** and allows a probe request. If the probe succeeds, the circuit closes; otherwise it re-opens. Additionally, `circuit_breaker.slow_call_duration_threshold` (default: `"2s"`) combined with `circuit_breaker.slow_call_rate_threshold` can detect slow calls, and `circuit_breaker.operation_timeout` (default: `"8s"`) sets a per-operation timeout. The sliding window type can be `CountBased` (by call count) or `TimeBased` (by time, default), configured via `circuit_breaker.sliding_window_type`.

If RMQTT is deployed in single-node mode, then "ram", "sled", and "redis" are all supported storage modes. 
In cluster mode, all three storage modes are also supported; for "redis" mode, a lightweight topic-only 
synchronization mechanism is used to reduce inter-node traffic.

### Architecture (v0.22.0+)

Starting from RMQTT **0.22.0**, the retainer plugin uses the following key optimizations:

- **In-Memory Topic Trie Index**: A `RetainTree` is built in memory on startup by scanning all stored retain messages. 
  This index enables fast wildcard matching when clients subscribe, replacing the previous SCAN+MATCH approach.
  The trie is updated incrementally as messages are set or removed.

- **Batch Storage**: Messages are collected into a channel and processed in batches using `batch_insert` / `batch_remove` 
  operations (controlled by `batch_messages_limit`), significantly improving throughput under high load.

- **Rate Counter**: When built with the `rate-counter` feature (enabled by default), the plugin tracks message 
  processing throughput for monitoring and debugging.

- **RetainSyncMode**: The storage backend reports whether it requires full retain message synchronization across 
  cluster nodes (`Full`) or only lightweight topic-name synchronization (`TopicOnly`).
  - `Full`: Used by local-only backends (ram, sled) — the full retain payload is broadcast to all nodes.
  - `TopicOnly`: Used by shared backends (redis) — only the topic name is broadcast; each node reads the 
    retain data directly from the shared storage and updates its in-memory topic index.

- **Circuit Breaker**: Integrated into the Retainer storage layer to detect storage backend failures and fast-fail
  retain operations without touching the backend, preventing cascading node failures. See configuration below.

- **Retain Engine API**: Added `retain_sync_mode()` and `sync_retain_topic()` to the `RetainStorage` trait,
  enabling storage backends to declare their cluster synchronization strategy and handle topic-only sync
  notifications from peers.


The plugin is now **enabled by default** in the main configuration. To verify or change this setting, check the
`plugins.default_startups` configuration in `rmqtt.toml`:
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






