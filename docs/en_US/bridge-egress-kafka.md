English | [简体中文](../zh_CN/bridge-egress-kafka.md)

# Kafka Bridging - Egress Mode

Kafka data bridging facilitates connectivity between our local MQTT environment and external Kafka clusters. 
In egress mode, the local MQTT broker is configured to forward messages to the designated remote Kafka server.

### Concurrent Connections:

MQTT provides the capability for multiple clients to establish concurrent connections to the bridged Kafka server. 
The number of concurrent Kafka client connections can be customized during bridge configuration. By fine-tuning 
this parameter, it is possible to maximize server resource utilization, leading to elevated message throughput and 
enhanced concurrent performance. This feature is particularly valuable for applications that demand high-load and 
high-concurrency processing.

*Kafka* client ID generation rules:
```
${client_id_prefix}:${bridge_name}:egress:${node_id}:${entry_index}:${client_no}
```
| 片段 | 描述                                                                                                         |
| ---- |------------------------------------------------------------------------------------------------------------|
| ${client_id_prefix} | Configured client ID prefix                                                                                |
| ${bridge_name} | Name of the bridge                                                                                         |
| ${node_id}  | Node ID running the Kafka client                                                                           |
| ${entry_index} | Topic entry index                                                                                          |
| ${client_no} | Number from 1 to the configured limit of concurrent *Kafka* client connections |


#### Plugin:

```bash
rmqtt-bridge-egress-kafka
```

#### Plugin Configuration File:

```bash
plugins/rmqtt-bridge-egress-kafka.toml
```

#### Plugin Configuration Structure:
```bash
[[bridges]]
name = "bridge_kafka_1"
connection configuration
[[bridges.entries]]
topic filter configuration
[[bridges.entries]]
topic filter configuration

[[bridges]]
name = "bridge_kafka_2"
connection configuration
[[bridges.entries]]
topic filter configuration
[[bridges.entries]]
topic filter configuration
```

The configuration file structure provides the capability to configure multiple bridges, each of which can connect 
to a distinct remote Kafka server. Furthermore, multiple topic filter sets can be specified for each bridge connection.

#### Plugin Configuration Options:
```bash
[[bridges]]
# Whether to enable the bridge. Values: true/false. Default: true.
enable = true
# Name of the bridge.
name = "bridge_kafka_1"
# Bootstrap servers for the Kafka cluster.
servers = "127.0.0.1:9092,127.0.0.1:9093,127.0.0.1:9094"

# Prefix for the client ID.
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
#The queue_timeout parameter controls how long to retry for if the librdkafka producer queue is full. 0 to never block.
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

By default, this plugin is not activated. To enable the session storage plugin, you must add the "rmqtt-bridge-egress-kafka"
entry to the "plugins.default_startups" configuration in the main configuration file "rmqtt.toml", for example:
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


