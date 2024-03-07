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

##ram, redis
storage.type = "ram"

##ram
storage.ram.cache_capacity = "3G"
storage.ram.cache_max_count = 1_000_000
storage.ram.encode = true

##redis
storage.redis.url = "redis://127.0.0.1:6379/"
storage.redis.prefix = "message-{node}"

##Quantity of expired messages cleared during each cleanup cycle.
cleanup_count = 5000
```

Currently, there are two supported storage engines: "ram" and "redis." "ram" stores data in local memory and allows 
configuration of maximum memory usage or maximum number of messages. It also supports indicating whether messages 
should be encoded before storage. "redis" storage currently only supports single-node configurations. Prefix configuration 
facilitates multiple RMQTT nodes using the same Redis storage service. {node} will be replaced with the identifier for 
the current node.


By default, this plugin is not enabled. To activate the message storage plugin, you must add the "rmqtt-message-storage" 
item to the "plugins.default_startups" configuration in the main configuration file "rmqtt.toml", like this:
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










