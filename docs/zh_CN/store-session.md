[English](../en_US/store-session.md)  | 简体中文

# 存储会话信息

连接信息，订阅关系，离线消息，飞行消息将被存储。

当连接成功后存储“连接信息”，每次订阅成功后存储“订阅关系”，会话连接持续期间会定期刷新最后操作时间，会话断开连接时会存储飞行中的消息，在会话断连但未过期期间
会存储离线消息。

当RMQTT服务节点重启时会载入未过期会话基本信息和订阅关系，转发未过期离线消息和飞行消息。如果会话已经过期，将丢弃所有信息。

插件：

```bash
rmqtt-session-storage
```

存储会话信息插件配置项：

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

当前支持“sled”和“redis”两种存储引擎。“sled”是存储在本地，需要配置存储位置和在内存中的缓存容量，适当大小可以提高读写效率。“redis”存储当前仅支持单节点，
前缀配置方便不同rmqtt节点使用同一套redis存储服务。{node}将被替换为当前节点标识。

默认情况下并没有启动此插件，如果要开启会话存储插件，必须在主配置文件“rmqtt.toml”中的“plugins.default_startups”配置中添加“rmqtt-session-storage”项，如：
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










