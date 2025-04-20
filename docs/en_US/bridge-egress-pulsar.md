English | [简体中文](../zh_CN/bridge-egress-pulsar.md)


# Apache-Pulsar Bridging - Egress Mode

Apache Pulsar data bridging is a method for connecting to other Apache Pulsar services. In egress mode, the 
local RMQTT forwards messages from the current cluster to the bridged remote Apache Pulsar server.


#### Plugin:

```bash
rmqtt-bridge-egress-pulsar
```

#### Plugin Configuration File:

```bash
plugins/rmqtt-bridge-egress-pulsar.toml
```

#### Plugin Configuration Structure:
```bash
[[bridges]]
name = "bridge_pulsar_1"
connection configuration
[[bridges.entries]]
topic filter configuration
[[bridges.entries]]
topic filter configuration

[[bridges]]
name = "bridge_pulsar_2"
connection configuration
[[bridges.entries]]
topic filter configuration
[[bridges.entries]]
topic filter configuration
```

The configuration file structure provides the capability to configure multiple bridges, each of which can connect
to a distinct remote *Apache-Pulsar* server. Furthermore, multiple topic filter sets can be specified for each bridge 
connection.

#### Plugin Configuration Options:
```bash

[[bridges]]
# Whether to enable
enable = true
# Bridge name
name = "bridge_pulsar_1"

servers = "pulsar://127.0.0.1:6650"
#servers = "pulsar+ssl://127.0.0.1:6651"

# producer name prefix
producer_name_prefix = "producer_1"

# add a custom certificate chain from a file to authenticate the server in TLS connections
#cert_chain_file = "./rmqtt-bin/rmqtt.fullchain.pem"
#allow insecure TLS connection if set to true, defaults to false
#allow_insecure_connection = false
#whether hostname verification is enabled when insecure TLS connection is allowed, defaults to true
#tls_hostname_verification_enabled = true

#token(JWT, Biscuit)/ or oauth2
#auth.name = "token"
#auth.data = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1c2VyX2lkIiwiZXhwIjoxNjI2NzY5MjAwLCJpYXQiOjE2MjY3NjU2MDB9.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"
#auth.name = "oauth2"
#auth.data = "{\"issuer_url\":\"https://example.com/oauth2/issuer\", \"credentials_url\":\"file:///path/to/credentials/file.json\"}"

[[bridges.entries]]
#Local topic filter: All messages matching this topic filter will be forwarded.
local.topic_filter = "local/topic1/egress/#"

remote.topic = "non-persistent://public/default/test1"
#forward all from data, including: from_type, from_node, from_ipaddress, from_clientid, from_username
remote.forward_all_from = false
#forward all publish data, including: dup, retain, qos, packet_id, topic (required to forward), payload (required to forward)
remote.forward_all_publish = false

#determines the target partition of the message. Messages with the same partition_key will be sent to the same partition.
remote.partition_key = ""

#this affects the order of messages within a specific partition, not across partitions. If messages have the same ordering_key, they will be consumed in the order they were sent.
#Values: clientid, uuid, random or {type="random", len=10}
# clientid - use mqtt clientid
# uuid - uuid
# random - randomly generated
#remote.ordering_key = "clientid"
#remote.ordering_key = {type="random", len=10}

#Override namespace's replication
remote.replicate_to = []
#current version of the schema
remote.schema_version = ""

#user defined properties added to all messages
remote.options.metadata = {}
#algorithm used to compress the messages, value: lz4,zlib,zstd,snappy
remote.options.compression = "lz4"
#producer access mode: shared = 0, exclusive = 1, waitforexclusive =2, exclusivewithoutfencing =3
remote.options.access_mode = 0

[[bridges.entries]]
local.topic_filter = "local/topic2/egress/#"

remote.topic = "non-persistent://public/default/test2"

```

By default, this plugin is not enabled. To activate it, you must add the `rmqtt-bridge-egress-pulsar` entry to the 
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
    "rmqtt-bridge-egress-pulsar",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```


