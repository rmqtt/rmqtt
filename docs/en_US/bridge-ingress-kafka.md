English | [简体中文](../zh_CN/bridge-ingress-kafka.md)

# Apache-Kafka Bridging - Ingress Mode

In ingress mode, the local RMQTT consumes topics from the bridged remote *Apache-Kafka* server and distributes the received 
messages within the current cluster.

If multiple nodes in an ingress RMQTT cluster consume the same *Apache-Kafka* topic with different remote.group_id values, 
they will receive duplicate messages. To prevent this, all nodes should use the same remote.group_id.


*Apache-Kafka* client ID generation rules:
```
${client_id_prefix}:${bridge_name}:ingress:${node_id}:${topic_entry_index}
```
| Segment                   | Description                          |
|----------------------|-----------------------------|
| ${client_id_prefix}  | Configured client ID prefix |
| ${bridge_name}       | Name of the bridge          |
| ${node_id}           | RMQTT Node ID                      |
| ${topic_entry_index} | Topic entry index                       |

#### Plugin:

```bash
rmqtt-bridge-ingress-kafka
```

#### Plugin Configuration File:

```bash
plugins/rmqtt-bridge-ingress-kafka.toml
```

#### Plugin Configuration Structure:
```bash
[[bridges]]
name = "bridge_kafka_1"
connection configuration
[[bridges.entries]]
Consumer topic configuration
[[bridges.entries]]
Consumer topic configuration

[[bridges]]
name = "bridge_kafka_2"
connection configuration
[[bridges.entries]]
Consumer topic configuration
[[bridges.entries]]
Consumer topic configuration
```
The configuration file structure indicates that we can configure multiple bridges to connect to different remote 
*Apache-Kafka* servers. Each bridge connection can also be configured with multiple consumer topics.

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

# Message expiry interval, 0 means no expiry
expiry_interval = "5m"

# See more properties and their definitions at https://github.com/confluentinc/librdkafka/blob/master/CONFIGURATION.md
[bridges.properties]
"message.timeout.ms" = "5000"
#"enable.auto.commit" = "false"

[[bridges.entries]]
# Kafka consume topic
remote.topic = "remote-topic1-ingress"
# Kafka group.id
remote.group_id = "group_id_001"
#remote.start_partition = -1
#remote.stop_partition = -1
# "beginning", "end", "stored" or "<offset>", "<-offset>"
#remote.offset = "beginning"

# Forward QoS. Values: 0, 1, 2, or unset (use QoS from message header).
local.qos = 1
# Forward topic. Supports using message key to replace "${kafka.key}".
local.topic = "local/topic1/ingress/${kafka.key}"
# Retain message. Values: true/false, or unset (use retain flag from message header).
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

By default, this plugin is not enabled. To activate it, you must add the `rmqtt-bridge-ingress-kafka` entry to the
`plugins.default_startups` configuration in the main configuration file `rmqtt.toml`, as shown below:
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