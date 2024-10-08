English | [简体中文](../zh_CN/bridge-egress-reductstore.md)


# Reductstore Bridging - Egress Mode

*Reductstore* data bridging is a method for connecting to other *Reductstore* services. In egress mode, the
local *RMQTT* forwards messages from the current cluster to the bridged remote *Reductstore* server.


#### Plugin:

```bash
rmqtt-bridge-egress-reductstore
```

#### Plugin Configuration File:

```bash
plugins/rmqtt-bridge-egress-reductstore.toml
```

#### Plugin Configuration Structure:
```bash
[[bridges]]
name = "bridge_reductstore_1"
connection configuration
[[bridges.entries]]
topic filter configuration
[[bridges.entries]]
topic filter configuration

[[bridges]]
name = "bridge_reductstore_2"
connection configuration
[[bridges.entries]]
topic filter configuration
[[bridges.entries]]
topic filter configuration
```

The configuration file structure provides the capability to configure multiple bridges, each of which can connect
to a distinct remote *Reductstore* server. Furthermore, multiple topic filter sets can be specified for each bridge
connection.

#### Plugin Configuration Options:
```bash
##--------------------------------------------------------------------
## rmqtt-bridge-egress-reductstore
##--------------------------------------------------------------------

# See more keys and their definitions at https://github.com/rmqtt/rmqtt/blob/master/docs/en_US/bridge-egress-reductstore.md

[[bridges]]
# Whether to enable
enable = true
# Bridge name
name = "bridge_reductstore_1"
# The address of the reductstore broker that the client will connect to using plain TCP.
# In this case, it's connecting to the local broker at port 8383.
servers = "http://127.0.0.1:8383"

# producer name prefix
producer_name_prefix = "producer_1"

# Set the API token to use for authentication.
#api_token = ""
# Set the timeout for HTTP requests.
#timeout = "15s"
# Set the SSL verification to false.
#verify_ssl = false

[[bridges.entries]]
# Local topic filter: All messages matching this topic filter will be forwarded.
local.topic_filter = "local/topic1/egress/#"

# The name of the bucket
remote.bucket = "bucket1"
remote.entry = "test1"
# Set the quota size.
remote.quota_size = 1_000_000_000
# Don't fail if the bucket already exists.
remote.exist_ok = true
# forward all from data, including: from_type, from_node, from_ipaddress, from_clientid, from_username
remote.forward_all_from = true
# forward all publish data, including: dup, retain, qos, packet_id, topic (required to forward), payload (required to forward)
remote.forward_all_publish = true

[[bridges.entries]]
# Local topic filter: All messages matching this topic filter will be forwarded.
local.topic_filter = "local/topic2/egress/#"

remote.bucket = "bucket2"
remote.entry = "test2"
# Set the quota size.
remote.quota_size = 1_000_000_000
# Don't fail if the bucket already exists.
remote.exist_ok = true

```

By default, this plugin is not enabled. To activate it, you must add the `rmqtt-bridge-egress-reductstore` entry to the
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
    "rmqtt-bridge-egress-reductstore",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```


