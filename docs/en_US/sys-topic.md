English | [简体中文](../zh_CN/sys-topic.md)

# $SYS System Topics

RMQTT periodically publishes its own operational status, message statistics, and client online/offline events to system topics starting with $SYS/.

The $SYS topic path starts with $SYS/brokers/{node}/. {node} refers to the name of the node where the event/message originated, for example:


```bash
$SYS/brokers/1/stats
$SYS/brokers/1/metrics
```

Plugin:

```bash
rmqtt-sys-topic
```

$SYS System Message Plugin Configuration:

```bash
#$SYS system message publish QoS
publish_qos = 1

#$SYS system message publish period
publish_interval = "1m"
```

<div style="width:100%;padding:15px;border-left:10px solid #1cc68b;background-color: #d1e3dd; color: #00b173;">
<div style="font-size:1.3em;">TIP<br></div>
<font style="color:#435364;font-size:1.1em;">

By default, RMQTT only allows MQTT clients from the local machine to subscribe to $SYS topics. Please refer to the [Built-in ACL](../zh_CN/acl.md) for modifying the publish-subscribe ACL rules.
In RMQTT, most of the data within the $SYS topics can be obtained through other less tightly coupled methods. Device online/offline status can be obtained using [Webhooks](../zh_CN/web-hook.md), while node and cluster status can be obtained through the [HTTP API - Statistical Metrics](http-api.md).
</font>
</div>


## Client Online/Offline Events

| Topic | Explanation   |
|------------| ------------- |
| $SYS/brokers/{node}/clients/{clientid}/connected    | Online Event: When any client comes online, RMQTT publishes a message to this topic.  |
| $SYS/brokers/{node}/clients/{clientid}/disconnected | Offline Event: When any client goes offline, RMQTT publishes a message to this topic.  |

*connected* The payload of the event message is parsed into the following JSON format:
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

*disconnected* The payload of the event message is parsed into the following JSON format:

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

## Session Creation/Terminated Events

| Topic | Explanation                                |
|------------|-------------------------------------|
| $SYS/brokers/{node}/session/{clientid}/created     | Session Creation Event: When any client session is created, RMQTT publishes a message to this topic. |
| $SYS/brokers/{node}/session/{clientid}/terminated  | Session Destruction Event: When any client session is destroyed, RMQTT publishes a message to this topic. |


*created* The payload of the event message is parsed into the following JSON format:
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

*terminated* The payload of the event message is parsed into the following JSON format:
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

## Subscription/Unsubscription Events

| Topic | Explanation                                  |
|------------|-------------------------------------|
| $SYS/brokers/{node}/session/{clientid}/subscribed     | Topic Subscription Event: When any topic subscription is successful, RMQTT publishes a message to this topic.  |
| $SYS/brokers/{node}/session/{clientid}/unsubscribed  | Topic Unsubscription Event: When any topic unsubscription is successful, RMQTT publishes a message to this topic. |


*subscribed* The payload of the event message is parsed into the following JSON format:
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

*unsubscribed* The payload of the event message is parsed into the following JSON format:
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


## Message Dropped Event

| Topic | Explanation                                  |
|------------|-------------------------------------|
| $SYS/brokers/{node}/message/dropped     | Message Discard Event: When any message is discarded, RMQTT publishes a message to this topic.  |

*dropped* The payload of the event message is parsed into the following JSON format:
```bash
{
  "from_node": 1,
  "from_ipaddress": "127.0.0.1:1883",
  "from_clientid": "rmqtt-12312431wewr232",
  "from_username": "foo",
  "node": 1,
  "ipaddress": "127.0.0.1:1883",
  "clientid": "rmqtt-12312431wewr232",
  "username": "foo",
  "dup": false,
  "retain": false,
  "qos": 1,
  "topic": "foo/#",
  "packet_id": 3,
  "payload": "dGVzdCAvdGVzdC9sd3QgLi4u",
  "reason": "MessageExpiration",
  "pts": 1692069106000,
  "ts": 1692069107000,
  "time": "2023-08-15 11:11:46.984"
}

```

## Node Status Data

| Topic                | Explanation     |
|---------------------------|--------|
| $SYS/brokers/{node}/stats | Node Status Data |

*stats* The payload of the event message is parsed into the following JSON format:

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

## Statistical Metrics

| Topic                | Explanation     |
|---------------------------|--------|
| $SYS/brokers/{node}/metrics | Node Statistical Metrics |

*metrics* The payload of the event message is parsed into the following JSON format:

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









