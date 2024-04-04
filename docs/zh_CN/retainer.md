[English](../en_US/retainer.md)  | 简体中文

# 保留消息

客户端发布消息时设置了**retain**标记，消息将被保留。然后当客户端订阅此消息匹配的主题过滤器时，将收到此保留消息。

**RMQTT 0.4.0**及之后版本默认将关闭**保留消息**功能。开始**保留消息**功能需要打开**rmqtt-retainer**插件和**listener.tcp.\<xxxx\>.retain_available**配置项。

#### 插件：

```bash
rmqtt-retainer
```

#### 插件配置文件：

```bash
plugins/rmqtt-retainer.toml
```

#### 插件配置项：

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

当前支持“ram”、“sled”和“redis”三种存储模式。“ram”是存储在内存。“sled”是存储在本地磁盘，需要配置存储位置和在内存中的缓存容量，适当大小可以提高读写效率。
“redis”存储当前仅支持单节点。{node}将被替换为当前节点标识。

另外，“max_retained_messages”：可以配置最大保留消息数量，0表示无限制；“max_payload_size”：限制消息负载大小。

如果RMQTT部署为单机模式，那么“ram”、“sled”和“redis”都是支持的。如果RMQTT部署为集群模式，就只支持“redis”。


默认情况下并没有启动此插件，如果要开启会话存储插件，必须在主配置文件“rmqtt.toml”中的“plugins.default_startups”配置中添加“rmqtt-retainer”项，如：
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










