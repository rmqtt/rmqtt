English | [简体中文](../zh_CN/bridge-egress-nats.md)


# NATS Bridging - Egress Mode

*NATS* data bridging is a method for connecting to other *NATS* services. In egress mode, the
local *RMQTT* forwards messages from the current cluster to the bridged remote *NATS* server.


#### Plugin:

```bash
rmqtt-bridge-egress-nats
```

#### Plugin Configuration File:

```bash
plugins/rmqtt-bridge-egress-nats.toml
```

#### Plugin Configuration Structure:
```bash
[[bridges]]
name = "bridge_nats_1"
connection configuration
[[bridges.entries]]
topic filter configuration
[[bridges.entries]]
topic filter configuration

[[bridges]]
name = "bridge_nats_2"
connection configuration
[[bridges.entries]]
topic filter configuration
[[bridges.entries]]
topic filter configuration
```

The configuration file structure provides the capability to configure multiple bridges, each of which can connect
to a distinct remote *NATS* server. Furthermore, multiple topic filter sets can be specified for each bridge
connection.

#### Plugin Configuration Options:
```bash
##--------------------------------------------------------------------
## rmqtt-bridge-egress-nats
##--------------------------------------------------------------------

# See more keys and their definitions at https://github.com/rmqtt/rmqtt/blob/master/docs/en_US/bridge-egress-nats.md

[[bridges]]
# Whether to enable
enable = true
# Bridge name
name = "bridge_nats_1"
# The address of the NATS broker that the client will connect to using plain TCP.
# In this case, it's connecting to the local broker at port 4222.
servers = "nats://127.0.0.1:4222"
#servers = "tls://127.0.0.1:4433"

# producer name prefix
producer_name_prefix = "producer_1"

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
#Local topic filter: All messages matching this topic filter will be forwarded.
local.topic_filter = "local/topic1/egress/#"

remote.topic = "test1"
# forward all from data, including: from_type, from_node, from_ipaddress, from_clientid, from_username
#remote.forward_all_from = true
# forward all publish data, including: dup, retain, qos, packet_id, topic (required to forward), payload (required to forward)
#remote.forward_all_publish = true

[[bridges.entries]]
#Local topic filter: All messages matching this topic filter will be forwarded.
local.topic_filter = "local/topic2/egress/#"

remote.topic = "test2"
```

By default, this plugin is not enabled. To activate it, you must add the `rmqtt-bridge-egress-nats` entry to the
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
    #"rmqtt-bridge-ingress-kafka",
    #"rmqtt-bridge-egress-kafka",
    "rmqtt-bridge-egress-nats",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```


