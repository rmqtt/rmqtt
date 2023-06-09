English | [简体中文](../zh_CN/web-hook.md)

# WebHook

WebHook is a feature provided by the [rmqtt-web-hook](https://github.com/rmqtt/rmqtt/tree/master/rmqtt-plugins/rmqtt-web-hook) plugin, which allows for the notification of hook events in RMQTT to a web service.

WebHook is internally implemented based on [hooks], but it is closer to the top layer. It captures various events in RMQTT by attaching callback functions to hooks and forwards them to the web server configured in rmqtt-web-hook.

Taking the example of the "client.connected" event, the event propagation process is as follows:

```bash
    Client      |    RMQTT     |  rmqtt-web-hook |   HTTP       +------------+
  =============>| - - - - - - -> - - - - - - - ->===========>  | Web Server |
                |    Broker    |                |  Request     +------------+
```

<div style="width:100%;padding:15px;border-left:10px solid #1cc68b;background-color: #d1e3dd; color: #00b173;">
<div style="font-size:1.3em;">TIP<br></div>
<font style="color:#435364;font-size:1.1em;">
WebHook's event handling is unidirectional, as it only supports pushing events from RMQTT to a web service and does not concern itself with the response from the web service.
With the help of WebHook, various business tasks can be accomplished, such as recording device online/offline status, subscription and message storage, message delivery confirmation, and more.
</font>
</div>


## Configuration Options

The configuration file for WebHook is located at [etc/plugins/rmqtt-web-hook.toml](https://github.com/rmqtt/rmqtt/blob/master/rmqtt-plugins/rmqtt-web-hook.toml).

The HTTP request body content is in JSON format, and the message payload attribute is encoded in BASE64.

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
# Since WebHook starts a separate Tokio runtime environment, it is necessary to specify the number of core worker threads allowed for this runtime.
worker_threads = 3
# The message queue capacity.
queue_capacity = 300_000
# The maximum number of parallel WebHook HTTP requests to send.
concurrency_limit = 128
# The default WebHook URL address(es) that can be specified with multiple addresses.
http_urls = ["http://127.0.0.1:5656/mqtt/webhook"] #default urls
# The timeout duration for HTTP requests.
http_timeout = "8s"

# This configuration option sets the maximum retry backoff time. If an HTTP request fails, it will be retried approximately 2, 4, 7, 11, 18, or 42 seconds later.
retry_max_elapsed_time = "60s"
# The retry factor, defaulting to 2.5.
retry_multiplier = 2.5

```


## Trigger Rules

In `etc/plugins/rmqtt-web-hook.toml`, trigger rules can be configured with the following format:

```bash
Rule:
rule.<Event> = [<Rule 1>, <Rule 2>, ..., <Rule n>]

rule.<Event> = [{action=<Action>, urls=[...], topics=[...]}]

For example:
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
### Trigger Event

Currently, the following events are supported:

| Name                | Description        | Execution Occurrence                                    |
| ------------------- | ------------------ | ------------------------------------------------------- |
| session_created     | Session created    | After the session creation is completed                  |
| session_terminated  | Session terminated | After the session is terminated                          |
| session_subscribed  | Session subscribed | After the subscription operation is completed            |
| session_unsubscribed| Session unsubscribed | After the unsubscription operation is completed          |
| client_connect      | Handle CONNECT     | When the server receives a CONNECT packet from the client |
| client_connack      | Send CONNACK       | When the server is ready to send a CONNACK packet         |
| client_connected    | Client connected   | After the client has successfully authenticated and connected to the system |
| client_disconnected | Connection closed  | When the client connection is being closed                |
| client_subscribe    | Subscribe to topic | After receiving a SUBSCRIBE packet, before executing the ACL authorization |
| client_unsubscribe  | Unsubscribe from topic | After receiving an UNSUBSCRIBE packet                |
| message_publish     | Publish message    | Before the server publishes (routes) the message          |
| message_delivered   | Message delivered  | Before delivering the message to the client               |
| message_acked       | Message acknowledged | After the server receives an ACK for the message from the client |
| message_dropped     | Message dropped    | When the message fails to be successfully forwarded       |

### [Rule]

Multiple trigger rules can be configured for the same event.

### Rule

A trigger rule is a TOML object with the following available keys:

- action: a string that defaults to the event name, but can be modified (e.g., client_connected_2)
- topics: an array of strings representing topic filter lists. Only messages with topics matching any of the filters in this list will trigger the event forwarding.
- urls: an array of URL addresses. It is optional, and when not specified, it uses the default http_urls configuration. If specified, it replaces the default http_urls configuration.

For example, if we want to forward messages with topics `a/b/c` and `foo/#` to a web server, the configuration should be as follows:

```bash
rule.message_publish = [{action = "message_publish", topics=["a/b/c", "foo/#"] }]
```

In this case, WebHook will only forward messages that match the `a/b/c` and `foo/#` topics, such as `foo/bar`, but it will not forward messages with topics like `a/b/d` or `fo/bar`.

## WebHook event parameters

When an event is triggered, WebHook will send an HTTP request to the web server configured in the `url` parameter. The request format is as follows:
```bash
URL: <url>      # Taken from the `url` field in the configuration
Method: POST    # Fixed to POST method

Body: <JSON>    # Body is a JSON-formatted string
```

For different events, the request body content varies. The following table lists the parameter lists for the request body in each event:

**session_created**

| Key        | Type    | Description                                      |
| ---------- | ------- | ------------------------------------------------ |
| action     | string  | Event name<br>Default value: "session_created"   |
| node       | integer | Node ID                                          |
| ipaddress  | string  | Client's source IP address and port               |
| clientid   | string  | Client ID                                        |
| username   | string  | Client username. If it doesn't exist, the value is "undefined" |
| created_at | integer | Session creation time, in milliseconds           |



**session_terminated**

| Key          | Type    | Description                                      |
|--------------| ------- | ------------------------------------------------ |
| action       | string  | Event name<br>Default value: "session_terminated" |
| node         | integer | Node ID                                          |
| ipaddress    | string  | Client's source IP address and port               |
| clientid     | string  | Client ID                                        |
| username     | string  | Client username. If it doesn't exist, the value is "undefined" |
| reason       | string  | Reason for session termination                    |

**session_subscribed**

| Key         | Type    | Description                                      |
| ----------- | ------- | ------------------------------------------------ |
| action      | string  | Event name<br>Default value: "session_subscribed" |
| node         | integer | Node ID                                          |
| ipaddress    | string  | Client's source IP address and port               |
| clientid    | string  | Client ID                                        |
| username    | string  | Client username. If it doesn't exist, the value is "undefined" |
| topic       | string  | Subscribed topic                                 |
| opts        | json    | Subscription options                             |

opts include

| Key  | Type | Description |
| ---- | ---- | ----------- |
| qos  | enum | QoS level, can be `0`, `1`, `2` |

**session_unsubscribed**

| Key         | Type    | Description                                          |
| ----------- | ------- | ---------------------------------------------------- |
| action      | string  | Event name<br>Default value: "session_unsubscribed"   |
| node         | integer | Node ID                                              |
| ipaddress    | string  | Client's source IP address and port                   |
| clientid    | string  | Client ID                                            |
| username    | string  | Client username. If it doesn't exist, the value is "undefined" |
| topic       | string  | Unsubscribed topic                                   |

**client_connect**

| Key        | Type    | Description                                        |
| ---------- | ------- | -------------------------------------------------- |
| action     | string  | Event name<br>Default value: "client_connect"       |
| node         | integer | Node ID                                            |
| ipaddress    | string  | Client's source IP address and port                 |
| clientid   | string  | Client ID                                          |
| username   | string  | Client username. If it doesn't exist, the value is "undefined" |
| keepalive  | integer | Requested keepalive time by the client              |
| proto_ver  | integer | Protocol version number                            |
| clean_session  | bool | Session persistence flag (MQTT 3.1, 3.1.1)         |
| clean_start  | bool | Clear session flag upon connection (MQTT 5.0)      |

**client_connack**

| Key           | Type    | Description                                        |
| ------------- | ------- |--------------------------------------------------- |
| action        | string  | Event name<br>Default: "client_connack"              |
| node          | integer | Node ID                                            |
| ipaddress     | string  | Source IP address and port of the client               |
| clientid      | string  | Client ID                                          |
| username      | string  | Client Username; "undefined" if it doesn't exist     |
| keepalive     | integer | Requested keep-alive time by the client               |
| proto_ver     | integer | Protocol version number                             |
| clean_session | bool    | Session persistence flag (MQTT 3.1, 3.1.1)          |
| clean_start   | bool    | Session clean start flag (MQTT 5.0)                 |
| conn_ack      | string  | "Connection Accepted" if successful; otherwise, indicates the reason for failure |


**client_connected**

| Key           | Type    | Description                                        |
| ------------- | ------- |--------------------------------------------------- |
| action        | string  | Event name<br>Default: "client_connected"           |
| node          | integer | Node ID                                            |
| ipaddress     | string  | Source IP address and port of the client               |
| clientid      | string  | Client ID                                          |
| username      | string  | Client Username; "undefined" if it doesn't exist     |
| keepalive     | integer | Requested keep-alive time by the client               |
| proto_ver     | integer | Protocol version number                             |
| clean_session | bool    | Session persistence flag (MQTT 3.1, 3.1.1)          |
| clean_start   | bool    | Session clean start flag (MQTT 5.0)                 |
| connected_at  | integer | Timestamp in milliseconds when the connection was established |
| session_present | bool  | Indicates whether a persistent session is present     |


**client_disconnected**

| Key             | Type    | Description                                        |
| --------------- | ------- |--------------------------------------------------- |
| action          | string  | Event name<br>Default: "client_disconnected"        |
| node            | integer | Node ID                                            |
| ipaddress       | string  | Source IP address and port of the client               |
| clientid        | string  | Client ID                                          |
| username        | string  | Client Username; "undefined" if it doesn't exist     |
| disconnected_at | integer | Timestamp in milliseconds when the disconnection occurred |
| reason          | string  | Reason for disconnection                            |

**client_subscribe**

| Key          | Type    | Description                                      |
| ------------ | ------- |------------------------------------------------- |
| action       | string  | Event name<br>Default: "client_subscribe"         |
| node         | integer | Node ID                                          |
| ipaddress    | string  | Source IP address and port of the client           |
| clientid     | string  | Client ID                                        |
| username     | string  | Client Username; "undefined" if it doesn't exist   |
| topic        | string  | Subscribed topic                                 |
| opts         | json    | Subscription options                             |

opts contains

| Key  | Type | Description                          |
| ---- | ---- | ------------------------------------ |
| qos  | enum | QoS level; can be `0`, `1`, or `2`    |


**client_unsubscribe**

| Key          | Type    | Description                                      |
| ------------ | ------- |------------------------------------------------- |
| action       | string  | Event name<br>Default: "client_unsubscribe"       |
| node         | integer | Node ID                                          |
| ipaddress    | string  | Source IP address and port of the client           |
| clientid     | string  | Client ID                                        |
| username     | string  | Client Username; "undefined" if it doesn't exist   |
| topic        | string  | Topic to unsubscribe from                        |

**message_publish**

| Key               | Type    | Description                                      |
| ----------------- | ------- |------------------------------------------------- |
| action            | string  | Event name<br>Default: "message_publish"          |
| from_node         | integer | Node ID of the publishing client                  |
| from_ipaddress    | string  | Source IP address and port of the publishing client  |
| from_clientid     | string  | Client ID of the publishing client                |
| from_username     | string  | Username of the publishing client; "undefined" if it doesn't exist |
| dup               | bool    | Indicates if the message is a duplicate           |
| retain            | bool    | Indicates if the message should be retained       |
| qos               | enum    | QoS level; can be `0`, `1`, or `2`                |
| topic             | string  | Topic of the message                             |
| packet_id         | string  | Message ID                                       |
| payload           | string  | Message payload                                  |
| ts                | integer | Timestamp in milliseconds when the Publish message was received |


**message_delivered**

| Key            | Type    | Description                                      |
| -------------- | ------- | -------------------------------------------------|
| action         | string  | Event name<br>Default: "message_delivered"        |
| from_node      | integer | Node ID of the publishing client                   |
| from_ipaddress | string  | Source IP address and port of the publishing client|
| from_clientid  | string  | Client ID of the publishing client                |
| from_username  | string  | Username of the publishing client; "undefined" if it doesn't exist |
| node           | integer | Node ID                                          |
| ipaddress      | string  | Source IP address and port of the client           |
| clientid       | string  | Client ID                                        |
| username       | string  | Client Username; "undefined" if it doesn't exist   |
| dup            | bool    | Indicates if the message is a duplicate           |
| retain         | bool    | Indicates if the message should be retained       |
| qos            | enum    | QoS level; can be `0`, `1`, or `2`                |
| topic          | string  | Topic of the message                             |
| packet_id      | string  | Message ID                                       |
| payload        | string  | Message payload                                  |
| pts            | integer | Timestamp in milliseconds when the Publish message was received |
| ts             | integer | Timestamp in milliseconds when this hook message was generated |


**message_acked**

| Key            | Type    | Description                                      |
| -------------- | ------- | -------------------------------------------------|
| action         | string  | Event name<br>Default: "message_acked"           |
| from_node      | integer | Node ID of the publishing client                   |
| from_ipaddress | string  | Source IP address and port of the publishing client|
| from_clientid  | string  | Client ID of the publishing client                |
| from_username  | string  | Username of the publishing client; "undefined" if it doesn't exist |
| node           | integer | Node ID                                          |
| ipaddress      | string  | Source IP address and port of the client           |
| clientid       | string  | Client ID                                        |
| username       | string  | Client Username; "undefined" if it doesn't exist   |
| dup            | bool    | Indicates if the message is a duplicate           |
| retain         | bool    | Indicates if the message should be retained       |
| qos            | enum    | QoS level; can be `0`, `1`, or `2`                |
| topic          | string  | Topic of the message                             |
| packet_id      | string  | Message ID                                       |
| payload        | string  | Message payload                                  |
| pts            | integer | Timestamp in milliseconds when the Publish message was received |
| ts             | integer | Timestamp in milliseconds when this hook message was generated |


**message_dropped**

| Key             | Type    | Description                                      |
|-----------------| ------- | -------------------------------------------------|
| action          | string  | Event name<br>Default: "message_dropped"          |
| from_node       | integer | Node ID of the publishing client                   |
| from_ipaddress  | string  | Source IP address and port of the publishing client|
| from_clientid   | string  | Client ID of the publishing client                |
| from_username   | string  | Username of the publishing client; "undefined" if it doesn't exist |
| node            | integer | Node ID (optional)                                |
| ipaddress       | string  | Source IP address and port of the client (optional)|
| clientid        | string  | Client ID(optional)                              |
| username        | string  | Client Username; "undefined" if it doesn't exist (optional) |
| dup             | bool    | Indicates if the message is a duplicate           |
| retain          | bool    | Indicates if the message should be retained       |
| qos             | enum    | QoS level; can be `0`, `1`, or `2`                |
| topic           | string  | Topic of the message                             |
| packet_id       | string  | Message ID                                       |
| payload         | string  | Message payload                                  |
| reason          | string  | Reason for dropping the message                   |
| pts             | integer | Timestamp in milliseconds when the Publish message was received |
| ts              | integer | Timestamp in milliseconds when this hook message was generated |





