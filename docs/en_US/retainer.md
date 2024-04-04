English | [简体中文](../zh_CN/retainer.md)


# Retain Message


When the client sets the **retain** flag while publishing a message, the message will be retained.
Then, when the client subscribes to a topic filter that matches this message, the retained message will be received.

Starting from **RMQTT 0.4.0**, the **Retain Message** feature will be disabled by default.
Enabling the **Retain Message** feature requires activating the **rmqtt-retainer** plugin and configuring the **listener.tcp.\<xxxx\>.retain_available** option.

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
# Multi-node cluster mode  - redis
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
```

Currently, three storage modes are supported: "ram", "sled", and "redis".
"ram" storage mode stores data in memory. "sled" storage mode stores data on the local disk and requires configuration of 
the storage location and cache capacity in memory. A suitable size can improve read/write efficiency. "redis" storage mode 
currently supports only single node. {node} will be replaced with the current node identifier.


Additionally, "max_retained_messages" can be configured to set the maximum number of retained messages, where 0 indicates 
no limit; "max_payload_size" limits the size of message payloads.


If RMQTT is deployed in single-node mode, then "ram", "sled", and "redis" are all supported storage modes. However, 
if RMQTT is deployed in cluster mode, only "redis" is supported.

By default, this plugin is not activated. To enable the session storage plugin, you must add the "rmqtt-retainer" item to 
the "plugins.default_startups" configuration in the main configuration file "rmqtt.toml", like this:
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










