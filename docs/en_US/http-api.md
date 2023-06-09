English | [简体中文](../zh_CN/http-api.md)

# HTTP API

RMQTT Broker provides HTTP APIs for integration with external systems,such as querying client information, publishing
messages.

RMQTT Broker's HTTP API service listens on port 6060 by default. You can modify the listening port through the
configuration file of etc/plugins/rmqtt-http-api.toml. All API calls start with api/v1.

## Response code

### HTTP status codes

The RMQTT Broker interface always returns 200 OK when the call is successful, and the response content is returned in
JSON format.

The possible status codes are as follows:

| Status Code | Description |
| ---- | ----------------------- |
| 200  | Succeed, and the returned JSON data will provide more information |
| 400  | Invalid client request, such as wrong request body or parameters |
| 401  | Client authentication failed , maybe because of invalid authentication credentials |
| 404  | The requested path cannot be found or the requested object does not exist |
| 500  | An internal error occurred while the server was processing the request |

## API Endpoints

## /api/v1

### GET /api/v1

Return all Endpoints supported by RMQTT Broker.

**Parameters:** None

**Success Response Body (JSON):**

| Name             | Type |  Description   |
|------------------| --------- | -------------- |
| []             | Array     | Endpoints list |
| - [0].path   | String    | Endpoint       |
| - [0].name   | String    | Endpoint name    |
| - [0].method | String    | HTTP Method    |
| - [0].descr  | String    | Description      |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1"

[{"descr":"Return the basic information of all nodes in the cluster","method":"GET","name":"get_brokers","path":"/brokers/{node}"}, ...]

```

## Broker Basic Information

### GET /api/v1/brokers/{node}

Return basic information of all nodes in the cluster.

**Path Parameters:**

| Name | Type | Required | Description                                                                  |
| ---- | --------- | ------------|------------------------------------------------------------------------------|
| node | Integer    | False       | Node ID，such as 1. <br/>If not specified, returns all node basic information |

**Success Response Body (JSON):**

| Name         | Type | Description                                                                                                                   |
|--------------| --------- |-------------------------------------------------------------------------------------------------------------------------------|
| {}/[]        | Object/Array of Objects | Returns the information of the specified node when the parameter exists, <br/>otherwise, returns the information of all nodes |
| .datetime    | String    | Current time, in the format of "YYYY-MM-DD HH:mm:ss"                                                                          |
| .node_id     | Integer    | Node ID                                                                                                                       |
| .node_name   | String    | Node name                                                                                                                     |
| .node_status | String    | Node status                                                                                                                          |
| .sysdescr    | String    | Software description                                                                                                                         |
| .uptime      | String    | RMQTT Broker runtime, in the format of "D days, H hours, m minutes, s seconds"                                                                       |
| .version     | String    | RMQTT Broker version                                                                                                                      |

**Examples:**

Get the basic information of all nodes:

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/brokers"

[{"datetime":"2022-07-24 23:01:31","node_id":1,"node_name":"1@127.0.0.1","node_status":"Running","sysdescr":"RMQTT Broker","uptime":"5 days 23 hours, 16 minutes, 3 seconds","version":"rmqtt/0.2.3-20220724094535"}]
```

Get the basic information of node 1 :

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/brokers/1"

{"datetime":"2022-07-24 23:01:31","node_id":1,"node_name":"1@127.0.0.1","node_status":"Running","sysdescr":"RMQTT Broker","uptime":"5 days 23 hours, 17 minutes, 15 seconds","version":"rmqtt/0.2.3-20220724094535"}
```

## Node

### GET /api/v1/nodes/{node}

Return the status of the node.

**Path Parameters:**

| Name | Type | Required | Description                                                             |
| ---- | --------- | ------------|-------------------------------------------------------------------------|
| node | Integer    | False       | Node ID, such as 1. <br/>If not specified, returns all node information |

**Success Response Body (JSON):**

| Name                    | Type                    | Description                                                                                                         |
|-------------------------|-------------------------|---------------------------------------------------------------------------------------------------------------------|
| {}/[]                   | Object/Array of Objects | Returns node information when node parameter exists,<br/>otherwise, returns information about all nodes in an Array |
| .boottime           | String                  | OS startup time                                                                                                     |
| .connections        | Integer                 | Number of clients currently connected to this node                                                                  |
| .disk_free          | Integer                 | Disk usable capacity (bytes)                                                                                        |
| .disk_total         | Integer                 | Total disk capacity (bytes)                                                                                         |
| .load1              | Float                   | CPU average load in 1 minute                                                                                        |
| .load5              | Float                   | CPU average load in 5 minute                                                                                        |
| .load15             | Float                   | CPU average load in 15 minute                                                                                       |
| .memory_free        | Integer                 | System free memory size (bytes)                                                                                     |
| .memory_total       | Integer                 | Total system memory size (bytes)                                                                                    |
| .memory_used        | Integer                 | Used system memory size (bytes)                                                                                     |
| .node_id            | Integer                 | Node ID                                                                                                             |
| .node_name          | String                  | Node name                                                                                                           |
| .node_status        | String                  | Node status                                                                                                         |
| .uptime             | String                  | RMQTT Broker runtime, in the format of "D days, H hours, m minutes, s seconds"                                                                                                               |
| .version            | String                  | RMQTT Broker version                                                                                                            |

**Examples:**

Get the status of all nodes:

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/nodes"

[{"boottime":"2022-06-30 05:20:24 UTC","connections":1,"disk_free":77382381568,"disk_total":88692346880,"load1":0.0224609375,"load15":0.0,"load5":0.0263671875,"memory_free":1457954816,"memory_total":2084057088,"memory_used":626102272,"node_id":1,"node_name":"1@127.0.0.1","node_status":"Running","uptime":"5 days 23 hours, 33 minutes, 0 seconds","version":"rmqtt/0.2.3-20220724094535"}]
```

Get the status of the specified node:

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/nodes/1"

{"boottime":"2022-06-30 05:20:24 UTC","connections":1,"disk_free":77382381568,"disk_total":88692346880,"load1":0.0224609375,"load15":0.0,"load5":0.0263671875,"memory_free":1457954816,"memory_total":2084057088,"memory_used":626102272,"node_id":1,"node_name":"1@127.0.0.1","node_status":"Running","uptime":"5 days 23 hours, 33 minutes, 0 seconds","version":"rmqtt/0.2.3-20220724094535"}
```

## Client

### GET /api/v1/clients

<span id = "get-clients" />

Returns the information of all clients under the cluster.

**Query String Parameters:**

| Name   | Type | Required | Default | Description                                                                                                                                                             |
| ------ | --------- | -------- | ------- |-------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| _limit | Integer   | False | 10000   | The maximum number of data items returned at one time. If not specified, it is determined by the configuration item `max_row_limit` of the` rmqtt-http-api.toml` plugin |

| Name            | Type   | Required | Description                     |
| --------------- | ------ | -------- |---------------------------------|
| clientid        | String | False    | Client identifier                    |
| username        | String | False    | Client username                         |
| ip_address      | String | False    | Client IP address                      |
| connected       | Bool   | False    | The current connection status of the client     |
| clean_start     | Bool   | False    | Whether the client uses a new session            |
| session_present | Bool   | False    | Whether the client is connected to an existing session    |
| proto_ver       | Integer| False    | Client protocol version             |
| _like_clientid  | String | False    | Fuzzy search of client identifier by substring method                  |
| _like_username  | String | False    | Client user name, fuzzy search by substring                 |
| _gte_created_at | Integer| False    | Search client session creation time by greater than or equal method      |
| _lte_created_at | Integer| False    | Search client session creation time by less than or equal method          |
| _gte_connected_at | Integer| False    | Search client connection creation time by greater than or equal method    |
| _lte_connected_at | Integer| False    | Search client connection creation time by less than or equal method  |
| _gte_mqueue_len | Integer| False    | Current length of message queue by greater than or equal method  |
| _lte_mqueue_len | Integer| False    | Current length of message queue by less than or equal method |

**Success Response Body (JSON):**

| Name                    | Type             | Description                                                                                                                       |
|-------------------------|------------------|-----------------------------------------------------------------------------------------------------------------------------------|
| []                      | Array of Objects | Information for all clients                                                                                                       |
| [0].node_id             | Integer          | ID of the node to which the client is connected                                                                                   |
| [0].clientid            | String           | Client identifier                                                                                                                 |
| [0].username            | String           | User name of client when connecting                                                                                               |
| [0].proto_ver           | Integer          | Protocol version used by the client                                                                                               |
| [0].ip_address          | String           | Client's IP address                                                                                                               |
| [0].port                | Integer          | Client port                                                                                                                       | 
| [0].connected_at        | String           | Client connection time, in the format of "YYYY-MM-DD HH:mm:ss"                                                                    |
| [0].disconnected_at     | String           | Client offline time, in the formatof "YYYY-MM-DD HH:mm:ss"，<br/>This field is only valid and returned when `connected` is` false` |
| [0].disconnected_reason | String           | Client offline reason                                                                    |
| [0].connected           | Boolean          | Whether the client is connected                                                                                                   |
| [0].keepalive           | Integer          | keepalive time, with the unit of second                                                                                           |
| [0].clean_start         | Boolean          | Indicate whether the client is using a brand new session                                                                          |
| [0].expiry_interval     | Integer          | Session expiration interval, with the unit of second                                                                              |
| [0].created_at          | String           | Session creation time, in the format "YYYY-MM-DD HH:mm:ss"                                                                        |
| [0].subscriptions_cnt   | Integer          | Number of subscriptions established by this client                                                                                |
| [0].max_subscriptions   | Integer          | Maximum number of subscriptions allowed by this client                                                                            |
| [0].inflight            | Integer          | Current length of inflight                                                                                                        |
| [0].max_inflight        | Integer          | Maximum length of inflight                                                                                                        |
| [0].mqueue_len          | Integer          | Current length of message queue                                                                                                   |
| [0].max_mqueue          | Integer          | Maximum length of message queue                                                                                                   |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/clients?_limit=10"

[{"clean_start":true,"clientid":"be82ee31-7220-4cad-a724-aaad9a065012","connected":true,"connected_at":"2022-07-30 18:14:08","created_at":"2022-07-30 18:14:08","disconnected_at":"","expiry_interval":7200,"inflight":0,"ip_address":"183.193.169.110","keepalive":60,"max_inflight":16,"max_mqueue":1000,"max_subscriptions":0,"mqueue_len":0,"node_id":1,"port":10839,"proto_ver":4,"subscriptions_cnt":0,"username":"undefined"}]
```

### GET /api/v1/clients/{clientid}

Returns information for the specified client

**Path Parameters:**

| Name   | Type | Required | Description |
| ------ | --------- | -------- |  ---- |
| clientid  | String | True | ClientID |

**Success Response Body (JSON):**

| Name | Type | Description |
|------| --------- | ----------- |
| {}   | Array of Objects | Client information, for details, see<br/>[GET /api/v1/clients](#get-clients)|

**Examples:**

Query the specified client

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/clients/example1"

{"clean_start":true,"clientid":"example1","connected":true,"connected_at":"2022-07-30 23:30:43","created_at":"2022-07-30 23:30:43","disconnected_at":"","expiry_interval":7200,"inflight":0,"ip_address":"183.193.169.110","keepalive":60,"max_inflight":16,"max_mqueue":1000,"max_subscriptions":0,"mqueue_len":0,"node_id":1,"port":11232,"proto_ver":4,"subscriptions_cnt":0,"username":"undefined"}
```

### DELETE /api/v1/clients/{clientid}

Kick out the specified client. Note that this operation will terminate the connection with the session.

**Path Parameters:**

| Name   | Type | Required | Description |
| ------ | --------- | -------- |  ---- |
| clientid  | String | True | ClientID |

**Success Response Body (String):**

| Name       | Type             | Description |
|------------|------------------|-----------|
| id         | String          | Connection Unique ID  |

**Examples:**

Kick out the specified client

```bash
$ curl -i -X DELETE "http://localhost:6060/api/v1/clients/example1"

1@10.0.4.6:1883/183.193.169.110:10876/example1/dashboard
```

### GET /api/v1/clients/{clientid}/online

Check if the client is online

**Path Parameters:**

| Name   | Type | Required | Description |
| ------ | --------- | -------- |  ---- |
| clientid  | String | True | ClientID |

**Success Response Body (JSON):**

| Name | Type | Description |
|------|------|-------------|
| body | Bool | is online   |

**Examples:**

Check if the client is online

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/clients/example1/online"

false
```

## Subscription Information

### GET /api/v1/subscriptions

Returns all subscription information under the cluster.

**Query String Parameters:**

| Name   | Type | Required | Default | Description                                                                                                  |
| ------ | --------- | -------- | ------- |--------------------------------------------------------------------------------------------------------------|
| _limit | Integer   | False | 10000   | The maximum number of data items returned at one time, if not specified, it is determined by the configuration item `max_row_limit` of the `rmqtt-http-api.toml` plugin |

| Name         | Type    | Description |
| ------------ | ------- | ----------- |
| clientid     | String  | Client identifier    |
| topic        | String  | congruent query  |
| qos          | Enum    | Possible values are `0`,`1`,`2` |
| share        | String  | Shared subscription group name |
| _match_topic | String  | Match query |

**Success Response Body (JSON):**

| Name            | Type             | Description |
|-----------------|------------------|-------------|
| []              | Array of Objects | All subscription information      |
| [0].node_id     | Integer          | Node ID     |
| [0].clientid    | String           | Client identifier      |
| [0].client_addr | String           | Client IP address and port  |
| [0].topic       | String           | Subscribe to topic        |
| [0].qos         | Integer          | QoS level      |
| [0].share       | String           | Shared subscription group name    |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/subscriptions?_limit=10"

[{"node_id":1,"clientid":"example1","topic":"foo/#","qos":2,"share":null},{"node_id":1,"clientid":"example1","topic":"foo/+","qos":2,"share":"test"}]
```

### GET /api/v1/subscriptions/{clientid}

Return the subscription information of the specified client in the cluster.

**Path Parameters:**

| Name   | Type | Required | Description |
| ------ | --------- | -------- |  ---- |
| clientid  | String | True | ClientID |

**Success Response Body (JSON):**

| Name            | Type             | Description |
|-----------------|------------------|-------------|
| []              | Array of Objects | All subscription information      |
| [0].node_id     | Integer          | Node ID     |
| [0].clientid    | String           | Client identifier      |
| [0].client_addr | String           | Client IP address and port  |
| [0].topic       | String           | Subscribe to topic        |
| [0].qos         | Integer          | QoS level      |
| [0].share       | String           | Shared subscription group name    |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/subscriptions/example1"

[{"node_id":1,"clientid":"example1","topic":"foo/+","qos":2,"share":"test"},{"node_id":1,"clientid":"example1","topic":"foo/#","qos":2,"share":null}]
```

## Routes

### GET /api/v1/routes

List all routes

**Query String Parameters:**

| Name   | Type | Required | Default | Description |
| ------ | --------- | -------- | ------- |  ---- |
| _limit | Integer   | False | 10000   | The maximum number of data items returned at one time, if not specified, it is determined by the configuration item `max_row_limit` of the `rmqtt-http-api.toml` plugin |

**Success Response Body (JSON):**

| Name          | Type | Description |
|---------------| --------- |-------------|
| []            | Array of Objects | All routes information      |
| [0].topic | String    | MQTT Topic  |
| [0].node_id  | Integer    | Node ID     |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/routes"

[{"node_id":1,"topic":"foo/#"},{"node_id":1,"topic":"foo/+"}]
```

### GET /api/v1/routes/{topic}

List all routes of a topic.

**Path Parameters:**

| Name   | Type | Required | Description |
| ------ | --------- | -------- |-------------|
| topic  | String   | True | Topic       |

**Success Response Body (JSON):**

| Name      | Type | Description            |
|-----------| --------- |------------------------|
| []        | Array of Objects | All routes information |
| [0].topic | String    | MQTT Topic             |
| [0].node_id | Integer    | Node ID                |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/routes/foo%2f1"

[{"node_id":1,"topic":"foo/#"},{"node_id":1,"topic":"foo/+"}]
```

## Publish message

### POST /api/v1/mqtt/publish

Publish MQTT message。

**Parameters (json):**

| Name     | Type | Required | Default | Description                             |
| -------- | --------- | -------- |--------|-----------------------------------------|
| topic    | String    | Optional |        | For topic and topics, with at least one of them specified                  |
| topics   | String    | Optional |        | Multiple topics separated by `,`. This field is used to publish messages to multiple topics at the same time       |
| clientid | String    | Optional | system | Client identifier                            |
| payload  | String    | Required |        | Message body                                    |
| encoding | String    | Optional | plain  | The encoding used in the message body. Currently only plain and base64 are supported |
| qos      | Integer   | Optional | 0      | QoS level                                  |
| retain   | Boolean   | Optional | false  | Whether it is a retained message                                 |

**Success Response Body (JSON):**

| Name | Type   | Description |
|------|--------|-------------|
| body | String | ok          |

**Examples:**

```bash
$ curl -i -X POST "http://localhost:6060/api/v1/mqtt/publish" --header 'Content-Type: application/json' -d '{"topic":"foo/1","payload":"Hello World","qos":1,"retain":false,"clientid":"example"}'

ok

$ curl -i -X POST "http://localhost:6060/api/v1/mqtt/publish" --header 'Content-Type: application/json' -d '{"topic":"foo/1","payload":"SGVsbG8gV29ybGQ=","qos":1,"encoding":"base64"}'

ok
```

## Subscribe to topic

### POST /api/v1/mqtt/subscribe

Subscribe to MQTT topic

**Parameters (json):**

| Name     | Type | Required | Default | Description |
| -------- | --------- | -------- | ------- | ------------ |
| topic    | String    | Optional |         | For topic and topics, with at least one of them specified |
| topics   | String    | Optional |         | Multiple topics separated by `,`. This field is used to subscribe to multiple topics at the same time |
| clientid | String    | Required |         | Client identifier |
| qos      | Integer   | Optional | 0       | QoS level |

**Success Response Body (JSON):**

| Name    | Type   | Description                                                        |
|---------|--------|--------------------------------------------------------------------|
| {}      | Object |                                                                    |
| {topic} | Bool   | Key is topic name，The value is the subscription result: true/false |

**Examples:**

Subscribe to the three topics `foo/a`, `foo/b`, `foo/c`

```bash
$ curl -i -X POST "http://localhost:6060/api/v1/mqtt/subscribe" --header 'Content-Type: application/json' -d '{"topics":"foo/a,foo/b,foo/c","qos":1,"clientid":"example1"}'

{"foo/a":true,"foo/c":true,"foo/b":true}
```

### POST /api/v1/mqtt/unsubscribe

Unsubscribe.

**Parameters (json):**

| Name     | Type | Required | Default | Description |
| -------- | --------- | -------- | ------- |-------------|
| topic    | String    | Required |         | Topic       |
| clientid | String    | Required |         | Client identifier      |

**Success Response Body (JSON):**

| Name | Type | Description |
|------|------|-------------|
| body | Bool | true/false  |

**Examples:**

Unsubscribe from `foo/a` topic

```bash
$ curl -i -X POST "http://localhost:6060/api/v1/mqtt/unsubscribe" --header 'Content-Type: application/json' -d '{"topic":"foo/a","clientid":"example1"}'

true
```

## plugins

### GET /api/v1/plugins

Returns information of all plugins in the cluster.

**Path Parameters:** 无

**Success Response Body (JSON):**

| Name                  | Type             | Description                                                                                                         |
|-----------------------|------------------|---------------------------------------------------------------------------------------------------------------------|
| []                    | Array of Objects | All plugin information                                                                                              |
| [0].node              | Integer          | Node ID                                                                                                             |
| [0].plugins           | Array            | Plugin information, an array of objects, see below                                                                  |
| [0].plugins.name      | String           | Plugin name                                                                                                         |
| [0].plugins.version   | String           | Plugin version                                                                                                      |
| [0].plugins.descr     | String           | Plugin description                                                                                                  |
| [0].plugins.active    | Boolean          | Whether the plugin is active                                                                                        |
| [0].plugins.inited    | Boolean          | Whether the plugin is initialized                                                                                   |
| [0].plugins.immutable | Boolean          | Whether the plugin is immutable, Immutable plugins will not be able to be stopped, config modified, restarted, etc. |
| [0].plugins.attrs     | Json             | Other additional properties of the plugin              |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/plugins"

[{"node":1,"plugins":[{"active":false,"attrs":null,"descr":null,"immutable":true,"inited":false,"name":"rmqtt-cluster-raft","version":null},{"active":false,"attrs":null,"descr":null,"immutable":false,"inited":false,"name":"rmqtt-auth-http","version":null},{"active":true,"attrs":null,"descr":"","immutable":true,"inited":true,"name":"rmqtt-acl","version":"0.1.1"},{"active":true,"attrs":null,"descr":"","immutable":false,"inited":true,"name":"rmqtt-counter","version":"0.1.0"},{"active":true,"attrs":null,"descr":"","immutable":false,"inited":true,"name":"rmqtt-http-api","version":"0.1.1"},{"active":false,"attrs":null,"descr":null,"immutable":false,"inited":false,"name":"rmqtt-web-hook","version":null},{"active":false,"attrs":null,"descr":null,"immutable":true,"inited":false,"name":"rmqtt-cluster-broadcast","version":null}]}]
```

### GET /api/v1/plugins/{node}

Return the plugin information under the specified node

**Path Parameters:**

| Name | Type | Required | Description         |
| ---- | --------- |----------|---------------------|
| node | Integer    | True     | Node ID, Such as: 1 |

**Success Response Body (JSON):**

| Name           | Type             | Description                    |
|----------------|------------------|--------------------------------|
| []             | Array of Objects | Plugin information, an array of objects, see below   |
| [0].name       | String           | Plugin name                       |
| [0].version    | String           | Plugin version                      |
| [0].descr      | String           | Plugin description                  |
| [0].active     | Boolean          | Whether the plugin is active                        |
| [0].inited     | Boolean          | Whether the plugin is initialized                 |
| [0].immutable  | Boolean          | Whether the plugin is immutable, Immutable plugins will not be able to be stopped, config modified, restarted, etc. |
| [0].attrs      | Json             | Other additional properties of the plugin       |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/plugins/1"

[{"active":false,"attrs":null,"descr":null,"immutable":true,"inited":false,"name":"rmqtt-cluster-raft","version":null},{"active":false,"attrs":null,"descr":null,"immutable":false,"inited":false,"name":"rmqtt-auth-http","version":null},{"active":true,"attrs":null,"descr":"","immutable":true,"inited":true,"name":"rmqtt-acl","version":"0.1.1"},{"active":true,"attrs":null,"descr":"","immutable":false,"inited":true,"name":"rmqtt-counter","version":"0.1.0"},{"active":true,"attrs":null,"descr":"","immutable":false,"inited":true,"name":"rmqtt-http-api","version":"0.1.1"},{"active":false,"attrs":null,"descr":null,"immutable":false,"inited":false,"name":"rmqtt-web-hook","version":null},{"active":false,"attrs":null,"descr":null,"immutable":true,"inited":false,"name":"rmqtt-cluster-broadcast","version":null}]
```

### GET /api/v1/plugins/{node}/{plugin}

Returns the plugin information of the specified plugin name under the specified node.

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | Node ID, Such as: 1    |
| plugin | String    | True       | Plugin name        |

**Success Response Body (JSON):**

| Name           | Type            | Description                    |
|----------------|-----------------|--------------------------------|
| {}             | Object | 插件信息      |
| {}.name       | String          | Plugin name     |
| {}.version    | String          | Plugin version                          |
| {}.descr      | String          | Plugin description               |
| {}.active     | Boolean         | Whether the plugin is active           |
| {}.inited     | Boolean         | Whether the plugin is initialized          |
| {}.immutable  | Boolean         | Whether the plugin is immutable, Immutable plugins will not be able to be stopped, config modified, restarted, etc. |
| {}.attrs      | Json            | Other additional properties of the plugin  |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/plugins/1/rmqtt-web-hook"

{"active":false,"attrs":null,"descr":null,"immutable":false,"inited":false,"name":"rmqtt-web-hook","version":null}
```

### GET /api/v1/plugins/{node}/{plugin}/config

Returns the plugin configuration information of the specified plugin name under the specified node.

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | Node ID, Such as: 1    |
| plugin | String    | True       | Plugin name        |

**Success Response Body (JSON):**

| Name           | Type     | Description |
|----------------|----------|-------------|
| {}             | Object   | Plugin configuration information      |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/plugins/1/rmqtt-http-api/config"

{"http_laddr":"0.0.0.0:6060","max_row_limit":10000,"workers":1}
```

### PUT /api/v1/plugins/{node}/{plugin}/config/reload

Reloads the plugin configuration information of the specified plugin name under the specified node.

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | Node ID, Such as: 1    |
| plugin | String    | True       | Plugin name        |

**Success Response Body (String):**

| Name | Type   | Description |
|------|--------|-------------|
| body | String | ok          |

**Examples:**

```bash
$ curl -i -X PUT "http://localhost:6060/api/v1/plugins/1/rmqtt-http-api/config/reload"

ok
```

### PUT /api/v1/plugins/{node}/{plugin}/load

Load the specified plugin under the specified node.

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | Node ID, Such as: 1    |
| plugin | String    | True       | Plugin name        |

**Success Response Body (String):**

| Name | Type   | Description |
|------|--------|-------------|
| body | String | ok          |

**Examples:**

```bash
$ curl -i -X PUT "http://localhost:6060/api/v1/plugins/1/rmqtt-web-hook/load"

ok
```

### PUT /api/v1/plugins/{node}/{plugin}/unload

Unload the specified plugin under the specified node.

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | Node ID, Such as: 1    |
| plugin | String    | True       | Plugin name        |

**Success Response Body (JSON):**

| Name | Type | Description |
|------|------|-------------|
| body | Bool | true/false  |

**Examples:**

```bash
$ curl -i -X PUT "http://localhost:6060/api/v1/plugins/1/rmqtt-web-hook/unload"

true
```

## Stats

### GET /api/v1/stats

<span id = "get-stats" />

Return all status data in the cluster.

**Path Parameters:** None

**Success Response Body (JSON):**

| Name          | Type             | Description   |
|---------------|------------------| ------------- |
| []            | Array of Objects | List of status data on each node |
| [0].node  | Json Object      | Node information |
| [0].stats | Json Object      | Status data, see  *stats* below |

**node:**

| Name          | Type    | Description |
|---------------|---------|-------------|
| id            | Integer | Node ID     |
| name          | String  | Node name        |
| status        | String | Node status        |

**stats:**

| Name                       | Type | Description                |
|----------------------------| --------- | -------------------------- |
| connections.count          | Integer   | Number of current connections |
| connections.max            | Integer   | Historical maximum number of connections |
| sessions.count             | Integer   | Number of current sessions |
| sessions.max               | Integer   | Historical maximum number of sessions |
| topics.count               | Integer   | Number of current topics |
| topics.max                 | Integer   | Historical maximum number of topics |
| subscriptions.count        | Integer   | Number of current subscriptions, including shared subscriptions |
| subscriptions.max          | Integer   | Historical maximum number of subscriptions |
| subscriptions_shared.count | Integer   | Number of current shared subscriptions |
| subscriptions_shared.max   | Integer   | Historical maximum number of shared subscriptions |
| routes.count               | Integer   | Number of current routes |
| routes.max                 | Integer   | Historical maximum number of routes |
| retained.count             | Integer   | Number of currently retained messages |
| retained.max               | Integer   | Historical maximum number of retained messages |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/stats"

[{"node":{"id":1,"name":"1@127.0.0.1","status":"Running"},"stats":{"connections.count":1,"connections.max":2,"retained.count":2,"retained.max":2,"routes.count":3,"routes.max":4,"sessions.count":1,"sessions.max":2,"subscriptions.count":7,"subscriptions.max":8,"subscriptions_shared.count":1,"subscriptions_shared.max":2,"topics.count":3,"topics.max":4}}]
```

### GET /api/v1/stats/{node}

Returns status data on the specified node.

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | Node ID, Such as: 1    |

**Success Response Body (JSON):**

| Name          | Type                 | Description                      |
|---------------|----------------------|----------------------------------|
| {}            | Object               | List of status data on each node |
| {}.node  | Json Object          | Node information                 |
| {}.stats | Json Object          | Status data, see  *stats* below    |

**node:**

| Name          | Type    | Description |
|---------------|---------|-------------|
| id            | Integer | Node ID       |
| name          | String  | Node name      |
| status        | String | Node status       |

**stats:**

| Name | Type | Description   |
|------| --------- |---------------|
| {}   | Json Object | Status data, see [GET /api/v1/stats](#get-stats) for details |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/stats/1"

{"node":{"id":1,"name":"1@127.0.0.1","status":"Running"},"stats":{"connections.count":1,"connections.max":2,"retained.count":2,"retained.max":2,"routes.count":3,"routes.max":4,"sessions.count":1,"sessions.max":2,"subscriptions.count":7,"subscriptions.max":8,"subscriptions_shared.count":1,"subscriptions_shared.max":2,"topics.count":3,"topics.max":4}}
```

### GET /api/v1/stats/sum

Summarize the status data of all nodes in the cluster.

**Path Parameters:** None

**Success Response Body (JSON):**

| Name      | Type                 | Description                            |
|-----------|----------------------|----------------------------------------|
| {}        | Object               | Status summary on each node            |
| {}.nodes  | Json Objects         | Node information                       |
| {}.stats | Json Object          | Status summary data, see *stats* below |

**nodes:**

| Name        | Type     | Description    |
|-------------|----------|----------------|
| {id}        | Object   | Node, key is the node ID |
| {id}.name   | String   | Node name      |
| {id}.status | String   | Node status    |

**stats:**

| Name | Type | Description                                                           |
|------| --------- |-----------------------------------------------------------------------|
| {}   | Json Object | Status summary data, see [GET /api/v1/stats](#get-stats) for details  |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/stats/sum"

{"nodes":{"1":{"name":"1@127.0.0.1","status":"Running"}},"stats":{"connections.count":1,"connections.max":2,"retained.count":2,"retained.max":2,"routes.count":3,"routes.max":4,"sessions.count":1,"sessions.max":2,"subscriptions.count":7,"subscriptions.max":8,"subscriptions_shared.count":1,"subscriptions_shared.max":2,"topics.count":3,"topics.max":4}}
```

## Metrics

### GET /api/v1/metrics

<span id = "get-metrics" />

Returns all statistical metrics under the cluster

**Path Parameters:** None

**Success Response Body (JSON):**

| Name          | Type             | Description                              |
|---------------|------------------|------------------------------------------|
| []            | Array of Objects | List of statistical metrics on each node |
| [0].node  | Json Object      | Node information                         |
| [0].metrics | Json Object      | Metrics, see *metrics* below             |

**node:**

| Name          | Type    | Description |
|---------------|---------|-------------|
| id            | Integer | Node ID       |
| name          | String  | Node name      |

**metrics:**

| Name | Type | Description                                                                  |
| ----------------| --------- |------------------------------------------------------------------------------|
| client.auth.anonymous           | Integer   | Number of clients who log in anonymously                                     |
| client.authenticate             | Integer   | Number of client authentications                                             |
| client.connack                  | Integer   | Number of CONNACK packet sent                                                |
| client.connect                  | Integer   | Number of client connections                                                 |
| client.connected                | Integer   | Number of successful client connections                                      |
| client.disconnected             | Integer   | Number of client disconnects                                                 |
| client.publish.check.acl        | Integer   | Number of ACL rule checks                                                    |
| client.subscribe.check.acl      | Integer   | Number of ACL rule checks                                                    |
| client.subscribe                | Integer   | Number of client subscriptions                                               |
| client.unsubscribe              | Integer   | Number of client unsubscriptions                                             |
| messages.publish                | Integer   | Number of received PUBLISH packet                                            |
| messages.delivered              | Integer   | Number of messages sent to the client                                        |
| messages.acked                  | Integer   | Number of received PUBACK and PUBREC packet                                  |
| messages.dropped                | Integer   | total number of messages dropped                                             |
| session.created                 | Integer   | Number of sessions created                                                   |
| session.resumed                 | Integer   | Number of sessions resumed because `Clean Session` or `Clean Start` is false |
| session.subscribed              | Integer   | Number of successful client subscriptions                                    |
| session.unsubscribed            | Integer   | Number of successful client unsubscriptions                                  |
| session.terminated              | Integer   | Number of terminated sessions       |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/metrics"

[{"metrics":{"client.auth.anonymous":38,"client.authenticate":47,"client.connack":47,"client.connect":47,"client.connected":47,"client.disconnected":46,"client.publish.check.acl":50,"client.subscribe":37,"client.subscribe.check.acl":15,"client.unsubscribe":8,"messages.acked":35,"messages.delivered":78,"messages.dropped":0,"messages.publish":78,"session.created":45,"session.resumed":2,"session.subscribed":15,"session.terminated":42,"session.unsubscribed":8},"node":{"id":1,"name":"1@127.0.0.1"}}]
```

### GET /api/v1/metrics/{node}

Returns statistical metrics data of the specified node under the cluster.

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | Node ID, Such as: 1    |

**Success Response Body (JSON):**

| Name          | Type                | Description        |
|---------------|---------------------|--------------------|
| {}            | Object         | Statistical metrics information      |
| {}.node  | Json Object         | Node information       |
| {}.metrics | Json Object       | Metrics, see *metrics* below |

**node:**

| Name          | Type    | Description |
|---------------|---------|-------------|
| id            | Integer | Node ID       |
| name          | String  | Node name      |

**metrics:**

| Name | Type | Description                                                                                                              |
|------| --------- |--------------------------------------------------------------------------------------------------------------------------|
| {}   | Json Object | Statistical metrics data, see [GET /api/v1/metrics](#get-metrics) for details |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/metrics/1"

{"metrics":{"client.auth.anonymous":38,"client.authenticate":47,"client.connack":47,"client.connect":47,"client.connected":47,"client.disconnected":46,"client.publish.check.acl":50,"client.subscribe":37,"client.subscribe.check.acl":15,"client.unsubscribe":8,"messages.acked":35,"messages.delivered":78,"messages.dropped":0,"messages.publish":78,"session.created":45,"session.resumed":2,"session.subscribed":15,"session.terminated":42,"session.unsubscribed":8},"node":{"id":1,"name":"1@127.0.0.1"}}
```

### GET /api/v1/metrics/sum

Summarize the statistical metrics data of all nodes under the cluster.

**Path Parameters:** None

**Success Response Body (JSON):**

| Name | Type | Description                                                                    |
|------| --------- |--------------------------------------------------------------------------------|
| {}   | Json Object | Statistical metrics data, see [GET /api/v1/metrics](#get-metrics) for details  |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/metrics/sum"

{"client.auth.anonymous":38,"client.authenticate":47,"client.connack":47,"client.connect":47,"client.connected":47,"client.disconnected":46,"client.publish.check.acl":50,"client.subscribe":37,"client.subscribe.check.acl":15,"client.unsubscribe":8,"messages.acked":35,"messages.delivered":78,"messages.dropped":0,"messages.publish":78,"session.created":45,"session.resumed":2,"session.subscribed":15,"session.terminated":42,"session.unsubscribed":8}
```

















