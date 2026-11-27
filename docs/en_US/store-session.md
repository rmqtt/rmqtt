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

##All circuit-breaker parameters (failure rate, window, etc.) are inherited
##from the global `[circuit_breaker]` section in `rmqtt.toml`.
##Only the backend operation timeout can be overridden here.
##
##Backend storage operation timeout. If a storage call exceeds this duration
##it is aborted and counted as a failure by the circuit breaker.
##Set to "0s" to disable.
##Default: "15s"
#backend_timeout = "15s"
```

Currently, three storage engines are supported: "sled," "redis," and "redis-cluster." "sled" is stored locally, requiring 
configuration for the storage location and in-memory cache size, with an appropriate size improving read and write efficiency.
Prefix configuration enables different rmqtt nodes to use the same Redis storage service. `{node}` will be replaced with 
the current node identifier.

The Circuit Breaker parameters (failure rate threshold, sliding window type/size, minimum calls, OPEN duration, slow call threshold, etc.) are inherited from the global `[circuit_breaker]` section in `rmqtt.toml`. Only the backend storage operation timeout can be overridden via `backend_timeout` (default: `"15s"`, set to `"0s"` to disable).


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

## Clearing the session store

The following scenarios require wiping **all persisted session information**, so that the node does not restore stale
subscriptions on start:

- Tightening or revoking ACL rules (see [Built-in ACL](acl.md#changing-acl-rules)). Persisted subscriptions are
  re-registered into the routing table when the node starts; without a wipe the revocation never takes effect and
  messages queued while the client is offline are replayed on reconnect.
- Forcing every client back to a session-less state (`session_present = 0`).

**Stop the node before clearing.** A running node keeps writing session state back, so clearing while it runs has no
effect.

`sled` (default backend, one directory per node; `{node}` is `node.id`, `1` by default):

```bash
rm -rf /var/log/rmqtt/.cache/session/1
```

`redis` / `redis-cluster` (keys are prefixed with `storage.redis.prefix` / `storage.redis-cluster.prefix`):

```bash
# Sample first to confirm the prefix and key layout (prefix defaults to session-{node}, i.e. session-1 for node 1)
redis-cli --scan --pattern 'session-1*' | head
# Delete in batches to avoid long blocking; do NOT use KEYS / FLUSHDB
redis-cli --scan --pattern 'session-1*' | xargs -r -n 100 redis-cli UNLINK
```

> Both `storage.sled.path` and the redis prefixes are configurable — use the values from your own configuration. In a
> multi-node deployment, repeat this for every node.

Impact of wiping: offline messages, inflight messages, subscription relationships and the `session_present` flag are
all lost. Clients reconnect into a brand-new session and must subscribe again.





