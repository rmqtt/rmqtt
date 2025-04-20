[English](../en_US/sys-topic.md)  | 简体中文

# $SYS 系统主题

RMQTT 周期性发布自身运行状态、消息统计、客户端上下线事件到以 $SYS/ 开头的系统主题。

$SYS 主题路径以 $SYS/brokers/{node}/ 开头。{node} 是指产生该 事件 / 消息 所在的节点名称，例如:

```bash
$SYS/brokers/1/stats
$SYS/brokers/1/metrics
```

#### 插件：

```bash
rmqtt-sys-topic
```

#### 插件配置文件：

```bash
plugins/rmqtt-sys-topic.toml
```

#### 插件配置项：

```bash
#$SYS system message publish QoS
publish_qos = 1

#$SYS system message publish period
publish_interval = "1m"

##Message expiration time, 0 means no expiration
message_expiry_interval = "5m"
```

<div style="width:100%;padding:15px;border-left:10px solid #1cc68b;background-color: #d1e3dd; color: #00b173;">
<div style="font-size:1.3em;">提示<br></div>
<font style="color:#435364;font-size:1.1em;">

RMQTT 默认只允许本机的 MQTT 客户端订阅 $SYS 主题，请参照 [内置 ACL](../zh_CN/acl.md) 修改发布订阅 ACL 规则。

RMQTT 中 $SYS 主题中绝大部分数据都可以通过其他更耦合性更低的方式获取，设备上下线状态可通过 [Webhook](../zh_CN/web-hook.md) 获取，节点与集群状态可通过 [HTTP API - 统计指标](http-api.md) 获取。

</font>
</div>


## 客户端上下线事件

| 主题 (Topic) | 说明   |
|------------| ------------- |
| $SYS/brokers/{node}/clients/{clientid}/connected    | 上线事件。当任意客户端上线时，RMQTT 就会发布该主题的消息  |
| $SYS/brokers/{node}/clients/{clientid}/disconnected | 下线事件。当任意客户端下线时，RMQTT 就会发布该主题的消息  |

*connected* 事件消息的 Payload 解析成 JSON 格式如下:
```bash
{
  "node": 1,
  "ipaddress": "127.0.0.1:1883",
  "clientid": "rmqtt-12312431wewr232",
  "username": "foo",
  "keepalive": 60,
  "proto_ver": 4,
  "clean_session": true,
  "connected_at": 1692069106000,
  "session_present": false,
  "time": "2023-08-15 11:11:46.984"
}

```

disconnected 事件消息的 Payload 解析成 JSON 格式如下:

```bash
{
  "node": 1,
  "ipaddress": "127.0.0.1:1883",
  "clientid": "rmqtt-12312431wewr232",
  "username": "foo",
  "disconnected_at": 1692069106000,
  "reason": "Disconnect",
  "time": "2023-08-15 11:11:46.984"
}
```

## 会话创建/销毁事件

| 主题 (Topic) | 说明                                  |
|------------|-------------------------------------|
| $SYS/brokers/{node}/session/{clientid}/created     | 会话创建事件。当任意客户端会话创建时，RMQTT 就会发布该主题的消息 |
| $SYS/brokers/{node}/session/{clientid}/terminated  | 会话销毁事件。当任意客户端会话销毁时，RMQTT 就会发布该主题的消息   |


*created* 事件消息的 Payload 解析成 JSON 格式如下:
```bash
{
  "node": 1,
  "ipaddress": "127.0.0.1:1883",
  "clientid": "rmqtt-12312431wewr232",
  "username": "foo",
  "created_at": 1692069106000,
  "time": "2023-08-15 11:11:46.984"
}

```

*terminated* 事件消息的 Payload 解析成 JSON 格式如下:
```bash
{
  "node": 1,
  "ipaddress": "127.0.0.1:1883",
  "clientid": "rmqtt-12312431wewr232",
  "username": "foo",
  "reason": "Disconnect",
  "time": "2023-08-15 11:11:46.984"
}

```

## 订阅/退订事件

| 主题 (Topic) | 说明                                  |
|------------|-------------------------------------|
| $SYS/brokers/{node}/session/{clientid}/subscribed     | 主题订阅事件。当任意主题订阅成功时，RMQTT 就会发布该主题的消息  |
| $SYS/brokers/{node}/session/{clientid}/unsubscribed  | 主题退订事件。当任意主题退订成功时，RMQTT 就会发布该主题的消息 |


*subscribed* 事件消息的 Payload 解析成 JSON 格式如下:
```bash
{
  "node": 1,
  "ipaddress": "127.0.0.1:1883",
  "clientid": "rmqtt-12312431wewr232",
  "username": "foo",
  "topic": "foo/#"
  "opts": { "qos":1 },
  "time": "2023-08-15 11:11:46.984"
}

```

*unsubscribed* 事件消息的 Payload 解析成 JSON 格式如下:
```bash
{
  "node": 1,
  "ipaddress": "127.0.0.1:1883",
  "clientid": "rmqtt-12312431wewr232",
  "username": "foo",
  "topic": "foo/#"
  "time": "2023-08-15 11:11:46.984"
}

```


## 节点状态数据

| 主题 (Topic)                | 说明     |
|---------------------------|--------|
| $SYS/brokers/{node}/stats | 节点状态数据 |

*stats* 事件消息的 Payload 解析成 JSON 格式如下:

```bash
{
    "connections.count": 0,
    "connections.max": 1,
    "handshakings.count": 0,
    "handshakings.max": 0,
    "handshakings_active.count": 0,
    "handshakings_rate.count": 0,
    "handshakings_rate.max": 0,
    "retained.count": 0,
    "retained.max": 0,
    "routes.count": 0,
    "routes.max": 0,
    "sessions.count": 0,
    "sessions.max": 1,
    "subscriptions.count": 0,
    "subscriptions.max": 0,
    "subscriptions_shared.count": 0,
    "subscriptions_shared.max": 0,
    "topics.count": 0,
    "topics.max": 0
}
```

## 统计指标

| 主题 (Topic)                | 说明     |
|---------------------------|--------|
| $SYS/brokers/{node}/metrics | 节点统计指标 |

*metrics* 事件消息的 Payload 解析成 JSON 格式如下:

```bash
{
    "client.auth.anonymous": 1,
    "client.auth.anonymous.error": 0,
    "client.authenticate": 1,
    "client.connack": 1,
    "client.connack.auth.error": 0,
    "client.connack.error": 0,
    "client.connect": 1,
    "client.connected": 1,
    "client.disconnected": 1,
    "client.handshaking.timeout": 0,
    "client.publish.auth.error": 0,
    "client.publish.check.acl": 0,
    "client.publish.error": 0,
    "client.subscribe": 0,
    "client.subscribe.auth.error": 0,
    "client.subscribe.check.acl": 0,
    "client.subscribe.error": 0,
    "client.unsubscribe": 0,
    "messages.acked": 0,
    "messages.acked.admin": 0,
    "messages.acked.custom": 0,
    "messages.acked.lastwill": 0,
    "messages.acked.retain": 0,
    "messages.acked.system": 0,
    "messages.delivered": 0,
    "messages.delivered.admin": 0,
    "messages.delivered.custom": 0,
    "messages.delivered.lastwill": 0,
    "messages.delivered.retain": 0,
    "messages.delivered.system": 0,
    "messages.dropped": 0,
    "messages.nonsubscribed": 0,
    "messages.nonsubscribed.admin": 0,
    "messages.nonsubscribed.custom": 0,
    "messages.nonsubscribed.lastwill": 0,
    "messages.nonsubscribed.system": 0,
    "messages.publish": 0,
    "messages.publish.admin": 0,
    "messages.publish.custom": 0,
    "messages.publish.lastwill": 0,
    "messages.publish.system": 0,
    "session.created": 1,
    "session.resumed": 0,
    "session.subscribed": 0,
    "session.terminated": 1,
    "session.unsubscribed": 0
}
```









