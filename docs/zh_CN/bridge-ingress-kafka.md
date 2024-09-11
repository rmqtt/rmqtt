[English](../en_US/bridge-ingress-kafka.md)  | 简体中文

# Apache-Kafka桥接-入口模式

在入口模式下，本地的 *RMQTT* 从桥接的远程 *Apache-Kafka* 服务器消费主题，并在当前集群内分发接收到的消息。

在入口模式下，当您拥有多个节点的 *RMQTT* 集群并配置了一个入口 *Apache-Kafka* 桥接并消费主题时，如果所有节点桥接客户端都消费相同的主题，
并且*remote.group_id*不同的情况下，它们将从远程 *Apache-Kafka* 服务器接收到重复的消息。反之，需要将各节点的*remote.group_id*设置
为相同的组。

*Apache-Kafka* 客户端ID生成规则：
```
${client_id_prefix}:${bridge_name}:ingress:${node_id}:${topic_entry_index}
```
| 片段                   | 描述               |
|----------------------|------------------|
| ${client_id_prefix}  | 配置的客户端 ID 前缀     |
| ${bridge_name}       | 桥接的名称            |
| ${node_id}           | RMQTT节点ID |
| ${topic_entry_index} | 主题项索引            |

#### 插件：

```bash
rmqtt-bridge-ingress-kafka
```

#### 插件配置文件：

```bash
plugins/rmqtt-bridge-ingress-kafka.toml
```

#### 插件配置结构：
```bash
[[bridges]]
name = "bridge_kafka_1"
连接配置
[[bridges.entries]]
消费主题配置
[[bridges.entries]]
消费主题配置

[[bridges]]
name = "bridge_kafka_2"
连接配置
[[bridges.entries]]
消费主题配置
[[bridges.entries]]
消费主题配置
```
通过配置文件结构可以看出，我们能够配置多个桥接，用于连接到不同的远程*Apache-Kafka*服务器。每个桥接连接，也可以配置多组消费主题。


#### 插件配置项：
```bash
[[bridges]]
#是否启用，值：true/false, 默认: true
enable = true
#桥接名称
name = "bridge_kafka_1"

#bootstrap.servers
servers = "127.0.0.1:9092,127.0.0.1:9093,127.0.0.1:9094"
#客户端ID前缀
client_id_prefix = "kafka_001"

#是否支持保持消息, 值：true/false, 默认: false
retain_available = false
#消息过期时间, 0 表示不过期
expiry_interval = "5m"

# See more properties and their definitions at https://github.com/confluentinc/librdkafka/blob/master/CONFIGURATION.md
[bridges.properties]
"message.timeout.ms" = "5000"
#"enable.auto.commit" = "false"

[[bridges.entries]]
#Kafka consume topic
remote.topic = "remote-topic1-ingress"
#Kafka group.id
remote.group_id = "group_id_001"
#remote.start_partition = -1
#remote.stop_partition = -1
# "beginning", "end", "stored" or "<offset>", "<-offset>"
#remote.offset = "beginning"

#转发QoS, 值：0,1,2，或 不设置（与消息头中的qos相同）。
local.qos = 1
#转发主题，支持使用消息key替换${kafka.key}
local.topic = "local/topic1/ingress/${kafka.key}"
#保持消息，值：true/false，或 不设置（与消息头中的retain相同）。
local.retain = false

[[bridges.entries]]
remote.topic = "remote-topic2-ingress"
remote.group_id = "group_id_002"
#remote.start_partition = -1
#remote.stop_partition = -1
#remote.offset = "beginning"

#local.qos = 0
local.topic = "local/topic2/ingress"
#local.retain = false

```

默认情况下并没有启动此插件，如果要开启此插件，必须在主配置文件“rmqtt.toml”中的“plugins.default_startups”配置中添加“rmqtt-bridge-ingress-kafka”项，如：
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
    "rmqtt-bridge-ingress-kafka",
    #"rmqtt-bridge-egress-kafka",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```