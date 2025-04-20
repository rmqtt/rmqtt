[English](../en_US/bridge-egress-pulsar.md)  | 简体中文

# Apache-Pulsar桥接-出口模式

*Apache-Pulsar*数据桥接是一种连接其他 *Apache-Pulsar* 服务的方式。在出口模式下，本地的 RMQTT 将当前集群中的消息转发给桥接的远程 *Apache-Pulsar* 服务器。

#### 插件：

```bash
rmqtt-bridge-egress-pulsar
```

#### 插件配置文件：

```bash
plugins/rmqtt-bridge-egress-pulsar.toml
```

#### 插件配置结构：
```bash
[[bridges]]
name = "bridge_pulsar_1"
连接配置
[[bridges.entries]]
主题过滤器配置
[[bridges.entries]]
主题过滤器配置

[[bridges]]
name = "bridge_pulsar_2"
连接配置
[[bridges.entries]]
主题过滤器配置
[[bridges.entries]]
主题过滤器配置
```
通过配置文件结构可以看出，我们能够配置多个桥接，用于连接到不同的远程*Apache-Pulsar*服务器。每个桥接连接，也可以配置多组主题过滤项。

#### 插件配置项：
```bash

# 是否启用
enable = true
# 桥接名称
name = "bridge_pulsar_1"

servers = "pulsar://127.0.0.1:6650"
#servers = "pulsar+ssl://127.0.0.1:6651"

# 生产者名称前缀
producer_name_prefix = "producer_1"

# 添加自定义证书链文件，以在 TLS 连接中验证服务器身份
cert_chain_file = "./rmqtt-bin/rmqtt.fullchain.pem"
# 如果设置为 true，则允许不安全的 TLS 连接，默认为 false
allow_insecure_connection = false
# 在允许不安全的 TLS 连接时，是否启用主机名验证，默认为 true
tls_hostname_verification_enabled = true

# 身份认证方式（支持 token 或 oauth2）
#auth.name = "token"
#auth.data = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiJ1c2VyX2lkIiwiZXhwIjoxNjI2NzY5MjAwLCJpYXQiOjE2MjY3NjU2MDB9.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c"
auth.name = "oauth2"
auth.data = "{\"issuer_url\":\"https://example.com/oauth2/issuer\", \"credentials_url\":\"file:///path/to/credentials/file.json\"}"

[[bridges.entries]]
# 本地主题过滤器：所有匹配此主题过滤器的消息将被转发
local.topic_filter = "local/topic1/egress/#"

remote.topic = "non-persistent://public/default/test1"
# 转发所有 from 数据，包括：from_type, from_node, from_ipaddress, from_clientid, from_username
remote.forward_all_from = false
# 转发所有 publish 数据，包括：dup, retain, qos, packet_id, topic（必转）, payload（必转）
remote.forward_all_publish = false

# 决定消息的目标分区, 具有相同 partition_key 的消息将被发送到同一个分区
remote.partition_key = ""

# 影响的是消息在特定分区内的顺序，而不是跨分区的顺序。如果消息具有相同的 ordering_key，它们将会按照发送顺序被消费
#Values: clientid, uuid, random or {type="random", len=10}
# clientid - use mqtt clientid
# uuid - uuid
# random - randomly generated
#remote.ordering_key = "clientid"
#remote.ordering_key = {type="random", len=10}

# 覆盖命名空间的复制设置
remote.replicate_to = []
# 当前的 schema 版本
remote.schema_version = ""

# 添加到所有消息的用户自定义属性
remote.options.metadata = {}
# 消息压缩算法，值为：lz4，zlib，zstd，snappy
remote.options.compression = "lz4"
# 生产者访问模式：shared = 0, exclusive = 1, waitforexclusive = 2, exclusivewithoutfencing = 3
remote.options.access_mode = 0


[[bridges.entries]]
local.topic_filter = "local/topic2/egress/#"

remote.topic = "non-persistent://public/default/test2"

```

默认情况下并没有启动此插件，如果要开启此插件，必须在主配置文件“rmqtt.toml”中的“plugins.default_startups”配置中添加“rmqtt-bridge-egress-pulsar”项，如：
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


