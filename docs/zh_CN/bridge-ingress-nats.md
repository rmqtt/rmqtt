[English](../en_US/bridge-ingress-nats.md)  | 简体中文

# NATS桥接-入口模式

在入口模式下，本地的 *RMQTT* 从桥接的远程 *NATS* 服务器订阅主题，并在当前集群内分发接收到的消息。

#### 插件：

```bash
rmqtt-bridge-ingress-nats
```

#### 插件配置文件：

```bash
plugins/rmqtt-bridge-ingress-nats.toml
```

#### 插件配置结构：

```bash
[[bridges]]
name = "bridge_nats_1"
连接配置
[[bridges.entries]]
消费主题配置
[[bridges.entries]]
消费主题配置

[[bridges]]
name = "bridge_nats_2"
连接配置
[[bridges.entries]]
消费主题配置
[[bridges.entries]]
消费主题配置
```

通过配置文件结构可以看出，我们能够配置多个桥接，用于连接到不同的远程 *NATS* 服务器。每个桥接连接，也可以配置多组消费主题。

---

#### 插件配置项：

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
##  - ${remote.topic}: NATS 消息的主题。
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

默认情况下并没有启动此插件，如果要开启此插件，必须在主配置文件“rmqtt.toml”中的 `plugins.default_startups` 配置中添加 `rmqtt-bridge-ingress-nats` 项，如：

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

