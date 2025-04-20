[English](../en_US/bridge-ingress-mqtt.md)  | 简体中文

# MQTT桥接-入口模式

MQTT 数据桥接是一种连接多个 RMQTT 集群或其他 MQTT 服务的方式。在入口模式下，本地的 RMQTT 从桥接的远程 MQTT 服务器订阅主题，
并在当前集群内分发接收到的消息。


### 并发连接：

RMQTT 允许多个客户端同时连接到桥接的 MQTT 服务器。在创建桥接时您可以设置一个 MQTT 客户端并发连接数。合适的 MQTT 客户端并发连
接数，可以充分利用服务器资源，以实现更大的消息吞吐和更好的并发性能。这对于处理高负载、高并发的场景非常重要。

在入口模式下，当您拥有多个节点的 RMQTT 集群并配置了一个入口 MQTT 桥接以从远程 MQTT 服务器订阅非共享主题时，如果所有节点桥接客
户端都订阅相同的主题或并发连接数大于1，它们将从远程 MQTT 服务器接收到重复的消息，这将给服务器带来压力。在这种情况下，强烈建议使
用共享订阅作为一种安全措施。例如，您可以将远程 MQTT 服务器的主题配置为 $share/group1/topic1 或者在使用主题过滤器时配置为 
$share/group2/topic2/#。在非共享订阅情况下，MQTT 客户端并发连接将缩减为一个客户端，这意味着只有一个客户端会启动。

由于 MQTT 协议要求连接到一个 MQTT 服务器的客户端必须具有唯一的客户端 ID，因此并发连接中的每个客户端都被分配了一个唯一的客户端 ID。
为了使客户端 ID 可预测，RMQTT 根据以下模式自动生成客户端 ID：

```
${client_id_prefix}:${bridge_name}:ingress:${node_id}:${subscribe_entry_index}:${client_no}
```


| 片段 | 描述                         |
| ---- |----------------------------|
| ${client_id_prefix} | 配置的客户端 ID 前缀               |
| ${bridge_name} | 桥接的名称                      |
| ${node_id}  | 运行 MQTT 客户端的节点ID           |
| ${subscribe_entry_index} | 订阅项索引                      |
| ${client_no} | 从 1 到配置的 MQTT 客户端并发连接限制大小的数字 |



#### 插件：

```bash
rmqtt-bridge-ingress-mqtt
```

#### 插件配置文件：

```bash
plugins/rmqtt-bridge-ingress-mqtt.toml
```

#### 插件配置结构：
```bash
[[bridges]]
name = "bridge_name_1"
连接配置
[[bridges.entries]]
订阅配置
[[bridges.entries]]
订阅配置

[[bridges]]
name = "bridge_name_2"
连接配置
[[bridges.entries]]
订阅配置
[[bridges.entries]]
订阅配置
```
通过配置文件结构可以看出，我们能够配置多个桥接，用于连接到不同的远程MQTT服务器。每个桥接连接，也可以配置多组订阅。

#### 插件配置项：
```bash
[[bridges]]
#是否启用，值：true/false, 默认: true
enable = true
#桥接名称
name = "bridge_name_1"
#客户端ID前缀
client_id_prefix = "prefix"
#远程mqtt broker的地址和端口
server = "127.0.0.1:1883"
#连接到远程mqtt broker用户名
username = "rmqtt_u"
#连接到远程mqtt broker用户密码
password = "public"

#连接到远程mqtt broker的并发客户端最大限制（相同订阅关系），建议采用共享订阅
concurrent_client_limit = 5
#连接超时
connect_timeout = "20s"
#心跳间隔
keepalive = "60s"
#自动重连间隔
reconnect_interval = "5s"
#消息过期时间, 0 表示不过期
expiry_interval = "5m"
#使用的MQTT协议版本号，有：v4,v5, 分别对应MQTT 3.1.1, 5.0
mqtt_ver = "v4"

#下面的配置与具体协议版本相关
#清除会话状态
v4.clean_session = true
#遗嘱消息配置，非必须
#消息正文使用的编码方式，支持 plain 与 base64 两种, 默认：plain
v4.last_will = {qos = 0, retain = false, topic = "a/b/c", message = "消息正文", encoding = "plain"}
#或
#连接时清除会话状态
v5.clean_start = true
#会话过期时间,0:表示会话将在网络连接断开时立即结束
v5.session_expiry_interval = "0s"
#限制客户端同时处理QoS为1和QoS为2的消息最大数量
v5.receive_maximum = 16
#客户端和服务端协商最大消息大小
v5.maximum_packet_size = "1M"
#客户端和服务端协商最大主题别名数量
v5.topic_alias_maximum = 0
#遗嘱消息配置，非必须
v5.last_will = {qos = 0, retain = false, topic = "a/b/c", message = "消息正文", encoding = "plain"}

[[bridges.entries]]
#订阅QoS, 值：0,1,2, 默认: 0
remote.qos = 0
#订阅主题
remote.topic = "$share/g/remote/topic1/ingress/#"

#转发QoS, 值：0,1,2，或 不设置（与消息 QoS相同）。
local.qos = 1
#转发主题，支持使用消息主题替换${remote.topic}
local.topic = "local/topic1/ingress/${remote.topic}"
#保持消息，值：true/false, 默认: false
local.retain = true

[[bridges.entries]]
#订阅QoS, 值：0,1,2, 默认: 0
remote.qos = 1
#订阅主题
remote.topic = "$share/g/remote/topic2/ingress"

#转发QoS, 值：0,1,2，或 不设置（与消息 QoS相同）。
#local.qos = 0
#转发主题
local.topic = "local/topic2/ingress"
#保持消息，值：true/false, 默认: false
local.retain = false

```

默认情况下并没有启动此插件，如果要开启此插件，必须在主配置文件“rmqtt.toml”中的“plugins.default_startups”配置中添加“rmqtt-bridge-ingress-mqtt”项，如：
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