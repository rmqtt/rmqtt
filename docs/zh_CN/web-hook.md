[English](../en_US/web-hook.md)  | 简体中文

# WebHook

WebHook 是由 [rmqtt-web-hook](https://github.com/rmqtt/rmqtt/tree/master/rmqtt-plugins/rmqtt-web-hook) 插件提供的 **将 RMQTT 中的钩子事件通知到某个 Web 服务** 的功能。

WebHook 的内部实现是基于 [钩子]，但它更靠近顶层一些。它通过在钩子上的挂载回调函数，获取到 RMQTT 中的各种事件，并转发至 rmqtt-web-hook 中配置的 Web 服务器。

以 客户端成功接入(client_connected) 事件为例，其事件的传递流程如下：

```bash
    Client      |    RMQTT     |  rmqtt-web-hook |   HTTP       +------------+
  =============>| - - - - - - -> - - - - - - - ->===========>  | Web Server |
                |    Broker    |                |  Request     +------------+
```

<div style="width:100%;padding:15px;border-left:10px solid #1cc68b;background-color: #d1e3dd; color: #00b173;">
<div style="font-size:1.3em;">提示<br></div>
<font style="color:#435364;font-size:1.1em;">
WebHook 对于事件的处理是单向的，**它仅支持将 RMQTT 中的事件推送给 Web 服务，并不关心 Web 服务的返回**。
借助 Webhook 可以完成设备在线、上下线记录，订阅与消息存储、消息送达确认等诸多业务。
</font>
</div>


## 配置项

Webhook 的配置文件位于 `etc/plugins/rmqtt-web-hook.toml` 。

HTTP 请求Body内容为JSON格式, 消息Payload属性通过BASE64编码。

```bash
##--------------------------------------------------------------------
## rmqtt-web-hook
##--------------------------------------------------------------------
##
#    Method: POST
#    Body: <JSON>
#    Payload: BASE64
#

## web hook general config
# 由于WebHook使用单独的Tokio运行时环境启动，因此需要指定此运行时环境允许的核心工作线程数。
worker_threads = 3
# 消息队列容量，
queue_capacity = 300_000
# 并行发送WebHook Http请求数
concurrency_limit = 128
# 默认WebHook URL地址，可以指定多个地址
http_urls = ["http://127.0.0.1:5656/mqtt/webhook"] #default urls
# HTTP请求超时时间
http_timeout = "8s"

# 此配置项设置最大重试失效时间，如果http请求失败，将在大约2、4、7、11、18或42秒后重试，
retry_max_elapsed_time = "60s"
# 重试因子，默认: 2.5
retry_multiplier = 2.5

```


## 触发规则

在 `etc/plugins/rmqtt-web-hook.toml` 可配置触发规则，其配置的格式如下：

```bash
规则:
rule.<Event> = [<Rule 1>, <Rule 2>, ..., <Rule n>]

如: 
rule.<Event> = [{action=<Action>, urls=[...], topics=[...]}]

例:
rule.session_created = [{action = "session_created" } ]
rule.session_terminated = [{action = "session_terminated" } ]
rule.session_subscribed = [{action = "session_subscribed" , topics=["x/y/z", "foo/#"] } ]
rule.session_unsubscribed = [{action = "session_unsubscribed" , topics=["x/y/z", "foo/#"] } ]

rule.client_connect = [{action = "client_connect"}]
rule.client_connack = [{action = "client_connack"} ]
rule.client_connected = [{action = "client_connected"}, {action = "client_connected_2", urls = ["http://127.0.0.1:5657/mqtt/webhook"] } ]
rule.client_disconnected = [{action = "client_disconnected" } ]
rule.client_subscribe = [{action = "client_subscribe", topics=["x/y/z", "foo/#"]} ]
rule.client_unsubscribe = [{action = "client_unsubscribe", topics=["x/y/z", "foo/#"] } ]

rule.message_publish = [{action = "message_publish" }]
rule.message_delivered = [{action = "message_delivered", topics=["x/y/z", "foo/#"] } ]
rule.message_acked = [{action = "message_acked", topics=["x/y/z", "foo/#"] } ]
rule.message_dropped = [{action = "message_dropped" } ]

```
### Event 触发事件

目前支持以下事件：

| 名称                 | 说明         | 执行时机                                            |
| -------------------- | ------------ |-------------------------------------------------|
| session_created    | 会话创建 | 完成会话创建后                                         |
| session_terminated | 会话结束 | 会话结束后                                           |
| session_subscribed   | 会话订阅主题 | 完成订阅操作后                                         |
| session_unsubscribed | 会话取消订阅 | 完成取消订阅操作后                                       |
| client_connect       | 处理连接报文 | 服务端收到客户端的连接报文时                                  |
| client_connack       | 下发连接应答 | 服务端准备下发连接应答报文时                                  |
| client_connected     | 成功接入     | 客户端认证完成并成功接入系统后                                 |
| client_disconnected  | 连接断开     | 客户端连接层在准备关闭时                                    |
| client_subscribe     | 订阅主题     | 收到订阅报文后，执行 `ACL` 鉴权前                            |
| client_unsubscribe   | 取消订阅     | 收到取消订阅报文后                                       |
| message_publish      | 消息发布     | 服务端在发布（路由）消息前                                   |
| message_delivered    | 消息投递     | 消息准备投递到客户端前                                     |
| message_acked        | 消息回执     | 服务端在收到客户端发回的消息 ACK 后                            |
| message_dropped      | 消息丢弃     | 消息未能成功转发                                        |

### [Rule]

同一个事件可以配置多个触发规则。

### Rule

触发规则，其值为一个Toml对象，其中可用的 Key 有：

- action：字符串，默认情况下与Event名称相同，也支持修改，比如：client_connected_2
- topics：字符串数组，表示主题过滤器列表，操作的主题只有与该列表中任一主题匹配才能触发事件的转发
- urls：url地址数组，非必须，不指定时，使用默认http_urls配置，否则将替换默认http_urls配置

例如，我们只将与 `a/b/c` 和 `foo/#` 主题匹配的消息转发到 Web 服务器上，其配置应该为：

```bash
rule.message_publish = [{action = "message_publish", topics=["a/b/c", "foo/#"] }]
```

这样 Webhook 仅会转发与 `a/b/c` 和 `foo/#` 主题匹配的消息，例如 `foo/bar` 等，而不是转发 `a/b/d` 或 `fo/bar`。


## Webhook 事件参数

事件触发时 Webhook 会按照配置将每个事件组成一个 HTTP 请求发送到 `url` 所配置的 Web 服务器上。其请求格式为：

```bash
URL: <url>      # 来自于配置中的 `url` 字段
Method: POST    # 固定为 POST 方法

Body: <JSON>    # Body 为 JSON 格式字符串
```

对于不同的事件，请求 Body 体内容有所不同，下表列举了各个事件中 Body 的参数列表：

**session_created**

| Key        |  类型   | 说明  |
| ---------- | ------- | ----- |
| action     | string  | 事件名称<br>默认为："session_created" |
| node       | integer  | 节点ID |
| ipaddress  | string  | 客户端源 IP 地址和端口 |
| clientid   | string  | 客户端 ClientId |
| username   | string  | 客户端 Username，不存在时该值为 "undefined" |
| created_at | integer | 会话创建时间, 单位：毫秒 |


**session_terminated**

| Key          |  类型   | 说明                             |
|--------------| ------- |--------------------------------|
| action       | string  | 事件名称<br>默认为："session_terminated" |
| node         | integer  | 节点ID |
| ipaddress    | string  | 客户端源 IP 地址和端口 |
| clientid     | string  | 客户端 ClientId                   |
| username     | string  | 客户端 Username，不存在时该值为 "undefined" |
| reason       | string  | 原因                             |

**session_subscribed**

| Key         |  类型   | 说明  |
| ----------- | ------- | ----- |
| action      | string  | 事件名称<br>默认为："session_subscribed" |
| node         | integer  | 节点ID |
| ipaddress    | string  | 客户端源 IP 地址和端口 |
| clientid    | string  | 客户端 ClientId |
| username    | string  | 客户端 Username，不存在时该值为 "undefined" |
| topic       | string  | 订阅的主题 |
| opts        | json    | 订阅参数 |

opts 包含

| Key  | 类型 | 说明 |
| ---- | ---- | ---- |
| qos  | enum | QoS 等级，可取 `0` `1` `2` |

**session_unsubscribed**

| Key         |  类型   | 说明  |
| ----------- | ------- | ----- |
| action      | string  | 事件名称<br>默认为："session_unsubscribed" |
| node         | integer  | 节点ID |
| ipaddress    | string  | 客户端源 IP 地址和端口 |
| clientid    | string  | 客户端 ClientId |
| username    | string  | 客户端 Username，不存在时该值为 "undefined" |
| topic       | string  | 取消订阅的主题 |


**client_connect**

| Key        | 类型   | 说明                               |
| ---------- |------|----------------------------------|
| action     | string | 事件名称<br>默认为："client_connect"     |
| node         | integer | 节点ID                             |
| ipaddress    | string | 客户端源 IP 地址和端口                    |
| clientid   | string | 客户端 ClientId                     |
| username   | string | 客户端 Username，不存在时该值为 "undefined" |
| keepalive  | integer | 客户端申请的心跳保活时间                     |
| proto_ver  | integer | 协议版本号                            |
| clean_session  | bool | 保持会话标记(MQTT 3.1, 3.1.1)          |
| clean_start  | bool | 连接时清除会话标记(MQTT 5.0)              |

**client_connack**

| Key        | 类型   | 说明                                                           |
| ---------- |------|--------------------------------------------------------------|
| action     | string | 事件名称<br>默认为："client_connack"                                 |
| node         | integer | 节点ID                                                         |
| ipaddress    | string | 客户端源 IP 地址和端口                                                |
| clientid   | string | 客户端 ClientId                                                 |
| username   | string | 客户端 Username，不存在时该值为 "undefined"                             |
| keepalive  | integer | 客户端申请的心跳保活时间                                                 |
| proto_ver  | integer | 协议版本号                                                        |
| clean_session  | bool | 保持会话标记(MQTT 3.1, 3.1.1)          |
| clean_start  | bool | 连接时清除会话标记(MQTT 5.0)              |
| conn_ack   | string  | "Connection Accepted" 表示成功，其它表示失败的原因 |


**client_connected**

| Key        | 类型   | 说明                               |
| ---------- |------|----------------------------------|
| action     | string | 事件名称<br>默认为："client_connected"   |
| node         | integer | 节点ID                             |
| ipaddress    | string | 客户端源 IP 地址和端口                    |
| clientid   | string | 客户端 ClientId                     |
| username   | string | 客户端 Username，不存在时该值为 "undefined" |
| keepalive  | integer | 客户端申请的心跳保活时间                     |
| proto_ver  | integer | 协议版本号                            |
| clean_session  | bool | 保持会话标记(MQTT 3.1, 3.1.1)          |
| clean_start  | bool | 连接时清除会话标记(MQTT 5.0)              |
| connected_at| integer | 时间戳(毫秒)                          |
| session_present| bool | 是否持久会话                           |


**client_disconnected**

| Key         |  类型   | 说明                                |
| ----------- | ------- |-----------------------------------|
| action      | string  | 事件名称<br>默认为："client_disconnected" |
| node         | integer | 节点ID                              |
| ipaddress    | string | 客户端源 IP 地址和端口                     |
| clientid    | string  | 客户端 ClientId                      |
| username    | string  | 客户端 Username，不存在时该值为 "undefined"  |
| disconnected_at  | integer  | 时间戳(毫秒)                           |
| reason      | string  | 断开原因                              |

**client_subscribe**

| Key         |  类型   | 说明  |
| ----------- | ------- | ----- |
| action      | string  | 事件名称<br>默认为："client_subscribe" |
| node         | integer  | 节点ID |
| ipaddress    | string  | 客户端源 IP 地址和端口 |
| clientid    | string  | 客户端 ClientId |
| username    | string  | 客户端 Username，不存在时该值为 "undefined" |
| topic       | string  | 订阅的主题 |
| opts        | json    | 订阅参数 |

opts 包含

| Key  | 类型 | 说明 |
| ---- | ---- | ---- |
| qos  | enum | QoS 等级，可取 `0` `1` `2` |


**client_unsubscribe**

| Key         |  类型   | 说明  |
| ----------- | ------- | ----- |
| action      | string  | 事件名称<br>默认为："client_unsubscribe" |
| node         | integer  | 节点ID |
| ipaddress    | string  | 客户端源 IP 地址和端口 |
| clientid    | string  | 客户端 ClientId |
| username    | string  | 客户端 Username，不存在时该值为 "undefined" |
| topic       | string  | 取消订阅的主题 |

**message_publish**

| Key               |  类型   | 说明                                  |
|-------------------| ------- |-------------------------------------|
| action            | string  | 事件名称<br>默认为："message_publish"       |
| from_node         | integer | 发布端节点ID                             |
| from_ipaddress    | string  | 发布端源 IP 地址和端口                       |
| from_clientid     | string  | 发布端 ClientId                        |
| from_username     | string  | 发布端 Username，不存在时该值为 "undefined"    |
| dup               | bool    | 是否为 Dup 消息                          |
| retain            | bool    | 是否为 Retain 消息                       |
| qos               | enum    | QoS 等级，可取 `0` `1` `2`               |
| topic             | string  | 消息主题                                |
| packet_id         | string  | 消息ID                                |
| payload           | string  | 消息 Payload                          |
| ts                | integer | 接收到Publish消息时的时间戳(毫秒)               |


**message_delivered**

| Key            |  类型   | 说明                               |
|----------------| ------- |----------------------------------|
| action         | string  | 事件名称<br>默认为："message_delivered"  |
| from_node      | integer | 发布端节点ID                          |
| from_ipaddress | string  | 发布端源 IP 地址和端口                    |
| from_clientid  | string  | 发布端 ClientId                     |
| from_username  | string  | 发布端 Username，不存在时该值为 "undefined" |
| node           | integer | 节点ID                             |
| ipaddress      | string  | 客户端源 IP 地址和端口                    |
| clientid       | string  | 客户端 ClientId                     |
| username       | string  | 客户端 Username，不存在时该值为 "undefined" |
| dup            | bool    | 是否为 Dup 消息                       |
| retain         | bool    | 是否为 Retain 消息                    |
| qos            | enum    | QoS 等级，可取 `0` `1` `2`            |
| topic          | string  | 消息主题                             |
| packet_id      | string  | 消息ID                             |
| payload        | string  | 消息 Payload                       |
| pts            | integer | 接收到Publish消息时的时间戳(毫秒)            |
| ts             | integer | 生成此hook消息时的时间戳(毫秒)               |

**message_acked**

| Key            |  类型   | 说明  |
| -------------- | ------- | ----- |
| action         | string  | 事件名称<br>默认为："message_acked" |
| from_node      | integer | 发布端节点ID                          |
| from_ipaddress | string  | 发布端源 IP 地址和端口                    |
| from_clientid  | string  | 发布端 ClientId                     |
| from_username  | string  | 发布端 Username，不存在时该值为 "undefined" |
| node           | integer | 节点ID                             |
| ipaddress      | string  | 客户端源 IP 地址和端口                    |
| clientid       | string  | 客户端 ClientId                     |
| username       | string  | 客户端 Username，不存在时该值为 "undefined" |
| dup            | bool    | 是否为 Dup 消息                       |
| retain         | bool    | 是否为 Retain 消息                    |
| qos            | enum    | QoS 等级，可取 `0` `1` `2`            |
| topic          | string  | 消息主题                             |
| packet_id      | string  | 消息ID                             |
| payload        | string  | 消息 Payload                       |
| pts            | integer | 接收到Publish消息时的时间戳(毫秒)       |
| ts             | integer | 生成此hook消息时的时间戳(毫秒)         |


**message_dropped**

| Key             |  类型   | 说明                                |
|-----------------| ------- |-----------------------------------|
| action          | string  | 事件名称<br>默认为："message_dropped"     |
| from_node       | integer | 发布端节点ID                           |
| from_ipaddress  | string  | 发布端源 IP 地址和端口                     |
| from_clientid   | string  | 发布端 ClientId                      |
| from_username   | string  | 发布端 Username，不存在时该值为 "undefined"  |
| node            | integer | 节点ID (非必须)                        |
| ipaddress       | string  | 客户端源 IP 地址和端口 (非必须)               |
| clientid        | string  | 客户端 ClientId (非必须)                |
| username        | string  | 客户端 Username，不存在时该值为 "undefined" (非必须) |
| dup             | bool    | 是否为 Dup 消息                        |
| retain          | bool    | 是否为 Retain 消息                     |
| qos             | enum    | QoS 等级，可取 `0` `1` `2`             |
| topic           | string  | 消息主题                              |
| packet_id       | string  | 消息ID                              |
| payload         | string  | 消息 Payload                        |
| reason          | string  | 消息丢弃原因                            |
| pts             | integer | 接收到Publish消息时的时间戳(毫秒)             |
| ts              | integer | 生成此hook消息时的时间戳(毫秒)                |




