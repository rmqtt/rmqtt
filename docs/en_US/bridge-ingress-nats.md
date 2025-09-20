English | [简体中文](../zh_CN/bridge-ingress-nats.md)

# NATS Bridge - Ingress Mode

In ingress mode, the local *RMQTT* consumes topics from the bridged remote *NATS* server and distributes the received messages within the current cluster.

#### Plugin:

```bash
rmqtt-bridge-ingress-nats
```

#### Plugin configuration file:

```bash
plugins/rmqtt-bridge-ingress-nats.toml
```

#### Plugin configuration structure:

```bash
[[bridges]]
name = "bridge_nats_1"
connection settings
[[bridges.entries]]
subscription configuration
[[bridges.entries]]
subscription configuration

[[bridges]]
name = "bridge_nats_2"
connection settings
[[bridges.entries]]
subscription configuration
[[bridges.entries]]
subscription configuration
```

From the configuration file structure, we can see that multiple bridges can be configured to connect to different remote *NATS* servers.
Each bridge connection can also configure multiple subscription entries.

---

#### Plugin configuration items:

```bash
##--------------------------------------------------------------------
## rmqtt-bridge-ingress-nats
##--------------------------------------------------------------------

# See more keys and their definitions at https://github.com/rmqtt/rmqtt/blob/master/docs/en_US/bridge-ingress-nats.md

[[bridges]]

## Whether to enable
##
## Value: true | false
## Default: false
enable = true

## Bridge name
##
## Value: String
name = "bridge_nats_1"

## The address of the NATS broker that the client will connect to using plain TCP.
## In this case, it's connecting to the local broker at port 4222.
##
## Value: URL
servers = "nats://127.0.0.1:4222"
#servers = "tls://127.0.0.1:4433"

## Consumer name prefix
##
## Value: String
consumer_name_prefix = "consumer_1"

## See https://github.com/nats-io/nats.rs/blob/main/async-nats/src/options.rs
#no_echo = true
#ping_interval = "60s"
#connection_timeout = "10s"
#tls_required = true
#tls_first = true
#root_certificates = ""
#client_cert = ""
#client_key = ""
#sender_capacity = 256
#auth.jwt = ""
#auth.jwt_seed = ""
#auth.nkey = ""
#auth.username = ""
#auth.password = ""
#auth.token = ""

[[bridges.entries]]

## NATS topic to subscribe
##
## Value: String
remote.topic = "test1"

## NATS queue group (optional). Messages will be distributed among consumers in the same group.
##
## Value: String
#remote.group = "group1"

## MQTT publish message topic
##
## Placeholder:
##  - ${remote.topic}: The topic of the NATS message.
##
## Value: String
local.topic = "local/topic1/ingress/${remote.topic}"

## Choose 0, 1, 2, or not set (follow message QoS, i.e. user-defined properties "qos")
##
## Value: 0 | 1 | 2
#local.qos = 1

## Choose true or false, or not set (follow message Retain, i.e. user-defined properties "retain")
##
## Value: true | false
## Default: false
#local.retain = false

## Allow forwarding of empty messages
##
## Value: true | false
## Default: true
#local.allow_empty_forward = false


[[bridges.entries]]

remote.topic = "test2"
#remote.group = "group2"

local.topic = "local/topic2/ingress"
#local.qos = 0
#local.retain = false

```

---

By default, this plugin is not enabled. To enable it, you must add the `rmqtt-bridge-ingress-nats` entry in the `plugins.default_startups` section of the main configuration file `rmqtt.toml`, for example:

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
    #"rmqtt-bridge-egress-kafka",
    "rmqtt-bridge-ingress-nats",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```
