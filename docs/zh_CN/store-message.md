[English](../en_US/store-message.md)  | 简体中文

# 存储未过期消息

发布的消息在过期之前将被存储。只要消息未过期，在消息发布之后发起的对此消息主题的订阅都会被转发。消息过期后会被自动清除。

插件：

```bash
rmqtt-message-storage
```

消息存储插件配置项：

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

当前支持“ram”和“redis”两种存储引擎。“ram”是存储在本地内存，可以配置最大使用内存容量或最大消息数量，以及可以指示消息是否编码后再存储。
“redis”存储当前仅支持单节点，前缀配置方便不同rmqtt节点使用同一套redis存储服务。{node}将被替换为当前节点标识。

默认情况下并没有启动此插件，如果要开启消息存储插件，必须在主配置文件“rmqtt.toml”中的“plugins.default_startups”配置中添加“rmqtt-message-storage”项，如：
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










