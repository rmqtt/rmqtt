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
storage.ram.encode = true

##redis
storage.redis.url = "redis://127.0.0.1:6379/"
storage.redis.prefix = "message-{node}"

##redis-cluster
storage.redis-cluster.urls = ["redis://127.0.0.1:6380/", "redis://127.0.0.1:6381/", "redis://127.0.0.1:6382/"]
storage.redis-cluster.prefix = "message-{node}"

##Quantity of expired messages cleared during each cleanup cycle.
cleanup_count = 5000
```

Currently, three storage engines are supported: "ram," "redis," and "redis-cluster." "ram" is stored in local memory and 
can be configured with maximum memory usage or maximum message count, and it can specify whether messages should be encoded 
before storage. Prefix configuration allows different rmqtt nodes to use the same Redis storage service. `{node}` will be 
replaced by the identifier of the current node.


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










