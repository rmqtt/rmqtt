[English](../en_US/bridge-egress-kafka.md)  | 简体中文

# Kafka桥接-出口模式

*Kafka*数据桥接是一种连接其他 *Kafka* 服务的方式。在出口模式下，本地的 RMQTT 将当前集群中的消息转发给桥接的远程 *Kafka* 服务器。

### 并发连接：

*RMQTT* 允许多个客户端同时连接到桥接的 *Kafka* 服务器。在创建桥接时您可以设置一个 *Kafka* 客户端并发连接数。合适的 *Kafka* 客户端并发连
接数，可以充分利用服务器资源，以实现更大的消息吞吐和更好的并发性能。这对于处理高负载、高并发的场景非常重要。

*Kafka* 客户端ID生成规则：
```
${client_id_prefix}:${bridge_name}:egress:${node_id}:${entry_index}:${client_no}
```
| 片段 | 描述                              |
| ---- |---------------------------------|
| ${client_id_prefix} | 配置的客户端 ID 前缀                    |
| ${bridge_name} | 桥接的名称                           |
| ${node_id}  | 运行 Kafka 客户端的节点ID               |
| ${entry_index} | 主题配置项索引                         |
| ${client_no} | 从 1 到配置的 *Kafka* 客户端并发连接限制大小的数字 |

#### 插件：

```bash
rmqtt-bridge-egress-kafka
```

#### 插件配置文件：

```bash
plugins/rmqtt-bridge-egress-kafka.toml
```

#### 插件配置结构：
```bash
[[bridges]]
name = "bridge_name_1"
连接配置
[[bridges.entries]]
主题过滤器配置
[[bridges.entries]]
主题过滤器配置

[[bridges]]
name = "bridge_name_2"
连接配置
[[bridges.entries]]
主题过滤器配置
[[bridges.entries]]
主题过滤器配置
```
通过配置文件结构可以看出，我们能够配置多个桥接，用于连接到不同的远程*Kafka*服务器。每个桥接连接，也可以配置多组主题过滤项。

#### 插件配置项：
```bash
[[bridges]]
#是否启用，值：true/false, 默认: true
enable = true
#桥接名称
name = "bridge_kafka_1"

# bootstrap.servers
servers = "127.0.0.1:9092,127.0.0.1:9093,127.0.0.1:9094"
# 客户端ID前缀
client_id_prefix = "kafka_001"

# Maximum limit of clients connected to the remote kafka broker
concurrent_client_limit = 3

# See more properties and their definitions at https://github.com/confluentinc/librdkafka/blob/master/CONFIGURATION.md
[bridges.properties]
"message.timeout.ms" = "5000"

[[bridges.entries]]
#Local topic filter: All messages matching this topic filter will be forwarded.
local.topic_filter = "local/topic1/egress/#"

remote.topic = "remote-topic1-egress-${local.topic}"
#参数控制当 librdkafka 生产者队列已满时，重试发送消息的最长时间。设置为 0 表示不阻塞，直接返回错误。
remote.queue_timeout = "0m"
#Sets the destination partition of the record.
#remote.partition = 0

[[bridges.entries]]
#Local topic filter: All messages matching this topic filter will be forwarded.
local.topic_filter = "local/topic2/egress/#"

remote.topic = "remote-topic2-egress"
#remote.queue_timeout = "0m"
#remote.partition = 0
```

默认情况下并没有启动此插件，如果要开启会话存储插件，必须在主配置文件“rmqtt.toml”中的“plugins.default_startups”配置中添加“rmqtt-bridge-egress-kafka”项，如：
```bash
##--------------------------------------------------------------------
## Plugins
##--------------------------------------------------------------------
#Plug in configuration file directory
plugins.dir = "rmqtt-plugins/"
#Plug in started by default, when the mqtt server is started
plugins.default_startups = [
    #"rmqtt-plugin-template",
    #"rmqtt-retainer",
    #"rmqtt-auth-http",
    #"rmqtt-cluster-broadcast",
    #"rmqtt-cluster-raft",
    #"rmqtt-sys-topic",
    #"rmqtt-message-storage",
    #"rmqtt-session-storage",
    #"rmqtt-bridge-ingress-mqtt",
    #"rmqtt-bridge-egress-mqtt",
    #"rmqtt-bridge-ingress-kafka",
    "rmqtt-bridge-egress-kafka",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```


