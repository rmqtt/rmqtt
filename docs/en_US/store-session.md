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

##sled, redis
storage.type = "sled"

##sled
storage.sled.path = "/var/log/rmqtt/.cache/session/{node}"
storage.sled.cache_capacity = "3G"

##redis
storage.redis.url = "redis://127.0.0.1:6379/"
storage.redis.prefix = "session-{node}"
```

Currently, two storage engines are supported: "sled" and "redis." "sled" stores data locally and requires configuration 
of storage location and cache capacity in memory. An appropriate size can improve read/write efficiency. "redis" storage 
currently only supports single node configuration. The prefix configuration facilitates the use of the same set of Redis 
storage services by different RMQTT nodes. {node} will be replaced with the current node identifier.


By default, this plugin is not enabled. To activate the session storage plugin, you must add the "rmqtt-session-storage" 
entry to the "plugins.default_startups" configuration in the main configuration file "rmqtt.toml", like so:
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





