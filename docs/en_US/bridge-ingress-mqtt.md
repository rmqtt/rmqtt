English | [简体中文](../zh_CN/bridge-ingress-mqtt.md)

# MQTT Bridging - Ingress Mode

MQTT data bridging is a way to connect multiple RMQTT clusters or other MQTT services. In ingress mode, 
the local RMQTT subscribes to topics from the bridged remote MQTT server and distributes the received 
messages within the current cluster.


### Concurrent Connections:

RMQTT allows multiple clients to simultaneously connect to the bridged MQTT server. When creating a bridge, 
you can set the number of concurrent MQTT client connections. An appropriate number of concurrent MQTT client 
connections can fully utilize server resources to achieve higher message throughput and better concurrent 
performance. This is crucial for handling high-load, high-concurrency scenarios.

In ingress mode, if you have an RMQTT cluster with multiple nodes and configure an ingress MQTT bridge to 
subscribe to non-shared topics from a remote MQTT server, all node bridge clients will receive duplicate 
messages from the remote MQTT server if they subscribe to the same topic or if the concurrent connection 
count is greater than one, putting stress on the server. In this case, it is strongly recommended to use 
shared subscriptions as a safety measure. For example, you can configure the remote MQTT server's topic 
as $share/group1/topic1 or, when using topic filters, configure it as $share/group2/topic2/#. In non-shared 
subscription scenarios, the concurrent MQTT client connections will be reduced to one client, meaning only 
one client will be active.

Due to the MQTT protocol requirement that clients connecting to an MQTT server must have unique client IDs, 
each client in concurrent connections is assigned a unique client ID. RMQTT automatically generates client 
IDs based on the following pattern for predictability:
```
${client_id_prefix}:${bridge_name}:ingress:${node_id}:${subscribe_entry_index}:${client_no}
```

| Segment | Description                         |
| ---- |----------------------------|
| ${client_id_prefix} | Configured client ID prefix               |
| ${bridge_name} | Name of the bridge                      |
| ${node_id}  | Node ID running the MQTT client          |
| ${subscribe_entry_index} | Subscription entry index                      |
| ${client_no} | Number from 1 to the configured limit of concurrent MQTT client connections |



#### Plugin:

```bash
rmqtt-bridge-ingress-mqtt
```

#### Plugin Configuration File:

```bash
plugins/rmqtt-bridge-ingress-mqtt.toml
```

#### Plugin Configuration Structure:
```bash
[[bridges]]
name = "bridge_name_1"
connection configuration
[[bridges.entries]]
subscription configuration
[[bridges.entries]]
subscription configuration

[[bridges]]
name = "bridge_name_2"
connection configuration
[[bridges.entries]]
subscription configuration
[[bridges.entries]]
subscription configuration
```
The configuration file structure shows that we can configure multiple bridges to connect to different remote MQTT servers. 
Each bridge connection can also be configured with multiple sets of subscriptions.

#### Plugin Configuration Options:
```bash
[[bridges]]
# Enable or disable, values: true/false, default: true
enable = true
# Bridge name
name = "bridge_name_1"
# Client ID prefix
client_id_prefix = "prefix"
# Remote MQTT broker address and port
server = "127.0.0.1:1883"
# Username to connect to the remote MQTT broker
username = "rmqtt_u"
# Password to connect to the remote MQTT broker
password = "public"

# Maximum limit for concurrent clients connecting to the remote MQTT broker (with the same subscription), shared subscription is recommended
concurrent_client_limit = 5
# Connection timeout
connect_timeout = "20s"
# Keepalive interval
keepalive = "60s"
# Auto-reconnect interval
reconnect_interval = "5s"
# MQTT protocol version, values: v4, v5, corresponding to MQTT 3.1.1, 5.0
mqtt_ver = "v4"

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
# Subscription QoS, values: 0,1,2, default: 0
remote.qos = 0
# Subscription topic
remote.topic = "$share/g/remote/topic1/ingress/#"

# Forward QoS, values: 0,1,2, or unset (same as message QoS)
local.qos = 1
# Forward topic, supports message topic substitution ${remote.topic}
local.topic = "local/topic1/ingress/${remote.topic}"
# Retain message, values: true/false, default: false
local.retain = true

## Other configuration items
# Support retain messages, values: true/false, default: false
retain_available = false
# Support message storage, values: true/false, default: false
storage_available = false
# Message expiry interval, 0 means no expiry
expiry_interval = "5m"

[[bridges.entries]]
# Subscription QoS, values: 0,1,2, default: 0
remote.qos = 1
# Subscription topic
remote.topic = "$share/g/remote/topic2/ingress"

# Forward QoS, values: 0,1,2, or unset (same as message QoS)
#local.qos = 0
# Forward topic
local.topic = "local/topic2/ingress"
# Retain message, values: true/false, default: false
local.retain = false

## Other configuration items
# Support retain messages, values: true/false, default: false
retain_available = false
# Support message storage, values: true/false, default: false
storage_available = false
# Message expiry interval, 0 means no expiry
expiry_interval = "5m"

```

By default, this plugin is not activated. To enable the session storage plugin, you must add the "rmqtt-bridge-ingress-mqtt"
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
    "rmqtt-bridge-ingress-mqtt",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```