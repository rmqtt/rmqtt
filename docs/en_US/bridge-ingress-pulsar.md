English | [简体中文](../zh_CN/bridge-ingress-pulsar.md)

# Apache-Pulsar Bridging - Ingress Mode

In ingress mode, the local RMQTT consumes topics from the bridged remote *Apache-Pulsar* server and distributes the received 
messages within the current cluster.


#### Plugin:

```bash
rmqtt-bridge-ingress-pulsar
```

#### Plugin Configuration File:

```bash
plugins/rmqtt-bridge-ingress-pulsar.toml
```

#### Plugin Configuration Structure:
```bash
[[bridges]]
name = "bridge_pulsar_1"
connection configuration
[[bridges.entries]]
Consumer topic configuration
[[bridges.entries]]
Consumer topic configuration

[[bridges]]
name = "bridge_pulsar_2"
connection configuration
[[bridges.entries]]
Consumer topic configuration
[[bridges.entries]]
Consumer topic configuration
```
The configuration file structure indicates that we can configure multiple bridges to connect to different remote 
*Apache-Pulsar* servers. Each bridge connection can also be configured with multiple consumer topics.

#### Plugin Configuration Options:
```bash
##--------------------------------------------------------------------
## rmqtt-bridge-ingress-pulsar
##--------------------------------------------------------------------

# See more keys and their definitions at https://github.com/rmqtt/rmqtt/blob/master/docs/en_US/bridge-ingress-pulsar.md

[[bridges]]

## Whether to enable
##
## Value: true | false
## Default: false
enable = true

## Bridge name
##
## Value: String
name = "bridge_pulsar_1"

## The address of the Pulsar broker that the client will connect to using plain TCP.
## In this case, it's connecting to the local broker at port 6650.
##
## The address of the Pulsar broker that the client would connect to using SSL/TLS.
## Uncomment this line to enable a secure connection to the local broker at port 6651.
##
## Value: URL
#servers = "pulsar+ssl://127.0.0.1:6651"
servers = "pulsar://127.0.0.1:6650"

## Consumer name prefix
##
## Value: String
consumer_name_prefix = "consumer_1"

## Add a custom certificate chain from a file to authenticate the server in TLS connections
##
## Value: String
#cert_chain_file = "./rmqtt-bin/rmqtt.fullchain.pem"

## Allow insecure TLS connection if set to true, defaults to false
##
## Value: true | false
## Default: false
#allow_insecure_connection = false

## Whether hostname verification is enabled when insecure TLS connection is allowed, defaults to true
##
## Value: true | false
## Default: true
#tls_hostname_verification_enabled = true

## Auth Configuration
##
## Value: token(JWT, Biscuit)/ or oauth2
#auth.name = "token"
#auth.data = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1c2VyX2lkIiwiZXhwIjoxNjI2NzY5MjAwLCJpYXQiOjE2MjY3NjU2MDB9.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"
#auth.name = "oauth2"
#auth.data = "{\"issuer_url\":\"https://example.com/oauth2/issuer\", \"credentials_url\":\"file:///path/to/credentials/file.json\"}"

## Whether to support retain message, true/false, default value: false
##
## Value: true | false
## Default: false
retain_available = false

## Message expiration time, 0 means no expiration
##
## Value: Duration
## Default: "5m"
expiry_interval = "5m"

[[bridges.entries]]

## Sets the consumer's topic or add one to the list of topics
##
## Value: String
remote.topic = "non-persistent://public/default/test"

## Adds a list of topics to the future consumer
##
## Value: String
#remote.topics = ["non-persistent://public/default/test1", "non-persistent://public/default/test2"]

## Sets up a consumer that will listen on all topics matching the regular expression
##
## Value: String
#remote.topic_regex = "non-persistent://public/default/.*"

## Sets the kind of subscription
##
## Value: "exclusive", "shared", "failover", "key_shared"
#remote.subscription_type = "shared"

## Sets the subscription's name
##
## Value: String
#remote.subscription = ""

## Tenant/Namespace to be used when matching against a regex. For other consumers, specify namespace using the
## `<persistent|non-persistent://<tenant>/<namespace>/<topic>` topic format.
##
## Defaults to `public/default` if not specifid
##
## Value: String
#lookup_namespace = ""

## Interval for refreshing the topics when using a topic regex. Unused otherwise.
##
## Value: Duration
#topic_refresh_interval = "10s"

## Sets the consumer id for this consumer
##
## Value: u64
#consumer_id = 1

## Sets the batch size
## batch messages containing more than the configured batch size will not be sent by Pulsar
##
## Value: u64
## Default: 1000
#batch_size = 1000

## Sets the dead letter policy
##
## Value: Object
#remote.dead_letter_policy = {max_redeliver_count = 10, dead_letter_topic = "test_topic"}

## The time after which a message is dropped without being acknowledged or nacked
## that the message is resent. If `None`, messages will only be resent when a
## consumer disconnects with pending unacknowledged messages.
##
## Value: Duration
#remote.unacked_message_resend_delay = "20s"


### Configuration options for consumers

## Sets the priority level
##
## Value: i32
#remote.options.priority_level = 3

## Signal whether the subscription should be backed by a durable cursor or not
##
## Value: true | false
#remote.options.durable = false

## Read compacted
##
## Value: true | false
#remote.options.read_compacted = false

## Signal whether the subscription will initialize on latest
## or earliest message (default on latest)
##
## Value: earliest | latest
## latest - start at the most recent message
## earliest - start at the oldest message
#remote.options.initial_position = "latest"

## User defined properties added to all messages
##
## Value: Object
#remote.options.metadata = {}

## Payload data format
##
## Value: bytes | json
## Default: bytes
#remote.payload_format = "json"

## Payload Path
##
## Extract the publish payload from the Pulsar message payload. This config item is only supported when
## `remote.payload_format` is set to JSON. For example: `/msg` or `/data/msg`
##
## Value: String
#remote.payload_path = "msg"

## MQTT publish message topic,
##
## Value: String
##
## Placeholder:
##  - ${remote.topic}: pulsar message topic.
##  - ${remote.properties.xxxx}: user-defined properties "topic". ${remote.properties.topic}
##  - ${remote.payload.xxxx}: Extract the topic from the Pulsar message payload. If the topic is an array, the message will be
##                     forwarded to each target topic individually. This placeholder is only supported when
##                     `remote.payload_format` is set to JSON. For example: `${remote.payload.topics}`
##
local.topic = "local/topic1/ingress/${remote.topic}"
#local.topics = ["local/topic1/ingress/${remote.properties.topic1}", "${remote.properties.topic2}", "${remote.payload.topic}", "${remote.payload.topics}"]

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

remote.topic = "non-persistent://public/default/test9"

local.topic = "local/topic2/ingress"
#local.qos = 0
#local.retain = false

```

By default, this plugin is not enabled. To activate it, you must add the `rmqtt-bridge-ingress-pulsar` entry to the
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
    #"rmqtt-bridge-egress-pulsar",
    "rmqtt-bridge-ingress-pulsar",
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```