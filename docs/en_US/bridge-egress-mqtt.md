English | [简体中文](../zh_CN/bridge-egress-mqtt.md)

# MQTT Bridging - Egress Mode

MQTT data bridging is a way to connect multiple RMQTT clusters or other MQTT services. In egress mode, the local 
RMQTT forwards messages from the current cluster to the bridged remote MQTT server.

### Concurrent Connections:

RMQTT allows multiple clients to connect simultaneously to the bridged MQTT server. When creating a bridge, you can 
set the number of concurrent MQTT client connections. An appropriate number of concurrent MQTT client connections can 
fully utilize server resources to achieve higher message throughput and better concurrent performance. This is crucial 
for handling high-load, high-concurrency scenarios.

As the MQTT protocol requires that each client connecting to an MQTT server must have a unique client ID, each client 
in the concurrent connections is assigned a unique client ID. To make client IDs predictable, RMQTT automatically 
generates client IDs based on the following pattern:

```
${client_id_prefix}:${bridge_name}:egress:${node_id}:${entry_index}:${client_no}
```

| Segment | Description                         |
| ---- |----------------------------|
| ${client_id_prefix} | Configured client ID prefix               |
| ${bridge_name} | Name of the bridge                      |
| ${node_id}  | Node ID running the MQTT client          |
| ${subscribe_entry_index} | Subscription entry index                      |
| ${client_no} | Number from 1 to the configured limit of concurrent MQTT client connections |


#### Plugin：

```bash
rmqtt-bridge-egress-mqtt
```

#### Plugin Configuration File:

```bash
plugins/rmqtt-bridge-egress-mqtt.toml
```

#### Plugin Configuration Structure:
```bash
[[bridges]]
name = "bridge_name_1"
connection configuration
[[bridges.entries]]
topic filter configuration
[[bridges.entries]]
topic filter configuration

[[bridges]]
name = "bridge_name_2"
connection configuration
[[bridges.entries]]
topic filter configuration
[[bridges.entries]]
topic filter configuration
```
From the configuration file structure, it is evident that we can configure multiple bridges to connect to different 
remote MQTT servers. Each bridge connection can also be configured with multiple sets of topic filters.

#### Plugin Configuration Options:
```bash
[[bridges]]
# Whether to enable
enable = true
# Bridge name
name = "bridge_name_1"
# Client ID prefix
client_id_prefix = "prefix"
# Address and port of the remote MQTT broker
server = "127.0.0.1:2883"
# Username to connect to the remote MQTT broker
username = "rmqtt_u"
# Password to connect to the remote MQTT broker
password = "public"

# Maximum limit of clients connected to the remote MQTT broker
concurrent_client_limit = 5
# Connection timeout
connect_timeout = "20s"
# Keepalive interval
keepalive = "60s"
# Automatic reconnection interval
reconnect_interval = "5s"
# Specifies the maximum number of messages that the channel can hold simultaneously.
message_channel_capacity = 100_000
# MQTT protocol version to use: v4, v5 corresponding to MQTT 3.1.1, 5.0
mqtt_ver = "v5"

# The following configurations are specific to the protocol version
# Clear session state
v4.clean_session = true
# Last will message configuration, optional
# Message encoding method, supports plain and base64, default: plain
v4.last_will = {qos = 0, retain = false, topic = "a/b/c", message = "message content", encoding = "plain"}
# Or
# Clear session state on connection
v5.clean_start = true
# Session expiry interval, 0 means the session will end immediately upon network disconnection
v5.session_expiry_interval = "0s"
# Limit the maximum number of QoS 1 and QoS 2 messages the client can handle simultaneously
v5.receive_maximum = 16
# Negotiate the maximum packet size with the client and server
v5.maximum_packet_size = "1M"
# Negotiate the maximum number of topic aliases with the client and server
v5.topic_alias_maximum = 0
# Last will message configuration, optional
v5.last_will = {qos = 0, retain = false, topic = "a/b/c", message = "message content", encoding = "plain"}

[[bridges.entries]]
#Local topic filter: All messages matching this topic filter will be forwarded.
local.topic_filter = "local/topic/egress/#"

# Choose 0, 1, 2, or not set (follow message QoS)
remote.qos = 1
# true/false, default: false
remote.retain = false
# Topic for messages forwarded to the remote MQTT server, where ${local.topic} represents the original topic of the forwarded message.
remote.topic = "remote/topic/egress/${local.topic}"

[[bridges.entries]]
local.topic_filter = "local/topic/egress/a"

remote.qos = 1
remote.retain = false
remote.topic = "remote/topic/egress/a/${local.topic}"

[[bridges.entries]]
local.topic_filter = "local/topic/egress/a/#"

remote.qos = 1
remote.retain = false
remote.topic = "remote/topic/egress/a/a/${local.topic}"
```

By default, this plugin is not enabled. To activate it, you must add the `rmqtt-bridge-egress-mqtt` entry to the
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
    "rmqtt-bridge-egress-mqtt",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```

