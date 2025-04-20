[English](../en_US/bridge-egress-nats.md)  | 简体中文

# NATS桥接-出口模式

*NATS*数据桥接是一种连接其他 *NATS* 服务的方式。在出口模式下，本地的 *RMQTT* 将当前集群中的消息转发给桥接的远程 *NATS* 服务器。

#### 插件：

```bash
rmqtt-bridge-egress-nats
```

#### 插件配置文件：

```bash
plugins/rmqtt-bridge-egress-nats.toml
```

#### 插件配置结构：
```bash
[[bridges]]
name = "bridge_nats_1"
连接配置
[[bridges.entries]]
主题过滤器配置
[[bridges.entries]]
主题过滤器配置

[[bridges]]
name = "bridge_nats_2"
连接配置
[[bridges.entries]]
主题过滤器配置
[[bridges.entries]]
主题过滤器配置
```
通过配置文件结构可以看出，我们能够配置多个桥接，用于连接到不同的远程*NATS*服务器。每个桥接连接，也可以配置多组主题过滤项。

#### 插件配置项：
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

默认情况下并没有启动此插件，如果要开启此插件，必须在主配置文件“rmqtt.toml”中的“plugins.default_startups”配置中添加“rmqtt-bridge-egress-nats”项，如：
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


