[English](../en_US/bridge-ingress-pulsar.md)  | 简体中文

# Apache-Pulsar桥接-入口模式

在入口模式下，本地的 *RMQTT* 从桥接的远程 *Apache-Pulsar* 服务器消费主题，并在当前集群内分发接收到的消息。


#### 插件：

```bash
rmqtt-bridge-ingress-pulsar
```

#### 插件配置文件：

```bash
plugins/rmqtt-bridge-ingress-pulsar.toml
```

#### 插件配置结构：
```bash
[[bridges]]
name = "bridge_pulsar_1"
连接配置
[[bridges.entries]]
消费主题配置
[[bridges.entries]]
消费主题配置

[[bridges]]
name = "bridge_pulsar_2"
连接配置
[[bridges.entries]]
消费主题配置
[[bridges.entries]]
消费主题配置
```
通过配置文件结构可以看出，我们能够配置多个桥接，用于连接到不同的远程*Apache-Pulsar*服务器。每个桥接连接，也可以配置多组消费主题。


#### 插件配置项：
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
## 从 Pulsar 消息的负载中提取发布数据。此配置项仅在 `remote.payload_format` 设置为 JSON 时支持。例如：`/msg` 或 `/data/msg`
##
## Value: String
#remote.payload_path = "msg"

## MQTT publish message topic,
##
## Value: String
##
## Placeholder:
##  - ${remote.topic}: Pulsar 消息的主题。
##  - ${remote.properties.xxxx}: 用户自定义属性。例如：${remote.properties.topic}
##  - ${remote.payload.xxxx}: 从 Pulsar 消息负载中提取主题。如果主题是一个数组，消息将分别转发到每个目标主题。此占位符仅在 
##                           `remote.payload_format` 设置为 JSON 时支持。例如：`${remote.payload.topics}`
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

默认情况下并没有启动此插件，如果要开启此插件，必须在主配置文件“rmqtt.toml”中的“plugins.default_startups”配置中添加“rmqtt-bridge-ingress-pulsar”项，如：
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