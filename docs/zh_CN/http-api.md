# HTTP API

RMQTT 提供了 HTTP API 以实现与外部系统的集成，例如查询客户端信息、发布消息等。

RMQTT 的 HTTP API 服务默认监听 6060 端口，可通过 `etc/plugins/rmqtt-http-api.toml` 配置文件修改监听端口。所有 API 调用均以 `api/v1` 开头。


## 响应码

### HTTP 状态码 (status codes)

RMQTT 接口在调用成功时总是返回 200 OK，响应内容主要以 JSON 格式返回。

可能的状态码如下：

| Status Code | Description                               |
| ---- |-------------------------------------------|
| 200  | 成功，如果需要返回更多数据，将以 JSON 数据格式返回              |
| 400  | 客户端请求无效，例如请求体或参数错误                        |
| 401  | 客户端未通过服务端认证，使用无效的身份验证凭据可能会发生              |
| 404  | 找不到请求的路径或者请求的对象不存在                        |
| 500  | 服务端处理请求时发生内部错误                            |


## API Endpoints

## /api/v1

### GET /api/v1

返回 RMQTT 支持的所有 Endpoints。

**Parameters:** 无

**Success Response Body (JSON):**

| Name             | Type |  Description   |
|------------------| --------- | -------------- |
| []             | Array     | Endpoints 列表 |
| - [0].path   | String    | Endpoint       |
| - [0].name   | String    | Endpoint 名    |
| - [0].method | String    | HTTP Method    |
| - [0].descr  | String    | 描述           |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1"

[{"descr":"Return the basic information of all nodes in the cluster","method":"GET","name":"get_brokers","path":"/brokers/{node}"}, ...]

```

## Broker 基本信息

### GET /api/v1/brokers/{node}

返回集群下所有节点的基本信息。

**Path Parameters:**

| Name | Type | Required | Description                 |
| ---- | --------- | ------------|-----------------------------|
| node | Integer    | False       | 节点ID，如：1 <br/>不指定时返回所有节点的基本信息 |

**Success Response Body (JSON):**

| Name         | Type | Description                                           |
|--------------| --------- |-------------------------------------------------------|
| {}/[]        | Object/Array of Objects | node 参数存在时返回指定节点信息，<br/>不存在时返回所有节点的信息                 |
| .datetime    | String    | 当前时间，格式为 "YYYY-MM-DD HH:mm:ss"                        |
| .node_id     | Integer    | 节点ID                                                  |
| .node_name   | String    | 节点名称                                                  |
| .node_status | String    | 节点状态                                                  |
| .sysdescr    | String    | 软件描述                                                  |
| .uptime      | String    | RMQTT 运行时间，格式为 "D days, H hours, m minutes, s seconds" |
| .version     | String    | RMQTT 版本                                               |


**Examples:**

获取所有节点的基本信息：

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/brokers"

[{"datetime":"2022-07-24 23:01:31","node_id":1,"node_name":"1@127.0.0.1","node_status":"Running","sysdescr":"RMQTT Broker","uptime":"5 days 23 hours, 16 minutes, 3 seconds","version":"rmqtt/0.2.3-20220724094535"}]
```

获取节点 1 的基本信息：

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/brokers/1"

{"datetime":"2022-07-24 23:01:31","node_id":1,"node_name":"1@127.0.0.1","node_status":"Running","sysdescr":"RMQTT Broker","uptime":"5 days 23 hours, 17 minutes, 15 seconds","version":"rmqtt/0.2.3-20220724094535"}
```

## 节点

### GET /api/v1/nodes/{node}

返回节点的状态。

**Path Parameters:**

| Name | Type | Required | Description                 |
| ---- | --------- | ------------|-----------------------------|
| node | Integer    | False       | 节点名字，如：1 <br/>不指定时返回所有节点的信息 |

**Success Response Body (JSON):**

| Name                    | Type                    | Description                                      |
|-------------------------|-------------------------|--------------------------------------------------|
| {}/[]                   | Object/Array of Objects | node 参数存在时返回指定节点信息，<br/>不存在时以 Array 形式返回所有节点的信息  |
| .boottime           | String                  | 操作系统启动时间                                         |
| .connections        | Integer                 | 当前接入此节点的客户端数量                                    |
| .disk_free          | Integer                 | 磁盘可用容量（字节）                                       |
| .disk_total         | Integer                 | 磁盘总容量（字节）                                        |
| .load1              | Float                   | 1 分钟内的 CPU 平均负载                                  |
| .load5              | Float                   | 5 分钟内的 CPU 平均负载                                  |
| .load15             | Float                   | 15 分钟内的 CPU 平均负载                                 |
| .memory_free        | Integer                 | 系统可用内存大小（字节）                                     |
| .memory_total       | Integer                 | 系统总内存大小（字节）                                      |
| .memory_used        | Integer                 | 系统已占用的内存大小   （字节）                                |
| .node_id            | Integer                 | 节点ID                                             |
| .node_name          | String                  | 节点名称                                             |
| .node_status        | String                  | 节点状态                                             |
| .uptime             | String                  | RMQTT 运行时间                                        |
| .version            | String                  | RMQTT 版本                                          |

**Examples:**

获取所有节点的状态：

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/nodes"

[{"boottime":"2022-06-30 05:20:24 UTC","connections":1,"disk_free":77382381568,"disk_total":88692346880,"load1":0.0224609375,"load15":0.0,"load5":0.0263671875,"memory_free":1457954816,"memory_total":2084057088,"memory_used":626102272,"node_id":1,"node_name":"1@127.0.0.1","node_status":"Running","uptime":"5 days 23 hours, 33 minutes, 0 seconds","version":"rmqtt/0.2.3-20220724094535"}]
```

获取指定节点的状态：

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/nodes/1"

{"boottime":"2022-06-30 05:20:24 UTC","connections":1,"disk_free":77382381568,"disk_total":88692346880,"load1":0.0224609375,"load15":0.0,"load5":0.0263671875,"memory_free":1457954816,"memory_total":2084057088,"memory_used":626102272,"node_id":1,"node_name":"1@127.0.0.1","node_status":"Running","uptime":"5 days 23 hours, 33 minutes, 0 seconds","version":"rmqtt/0.2.3-20220724094535"}
```

## 客户端

### GET /api/v1/clients

<span id = "get-clients" />

返回集群下所有客户端的信息。

**Query String Parameters:**

| Name   | Type | Required | Default | Description |
| ------ | --------- | -------- | ------- |  ---- |
| _limit | Integer   | False | 10000   | 一次最多返回的数据条数，未指定时由 `rmqtt-http-api.toml` 插件的配置项 `max_row_limit` 决定 |

| Name            | Type   | Required | Description                     |
| --------------- | ------ | -------- |---------------------------------|
| clientid        | String | False    | 客户端标识符                          |
| username        | String | False    | 客户端用户名                          |
| ip_address      | String | False    | 客户端 IP 地址                       |
| connected       | Bool   | False    | 客户端当前连接状态                       |
| clean_start     | Bool   | False    | 客户端是否使用了全新的会话                   |
| proto_ver       | Integer| False    | 客户端协议版本, 3,4,5                  |
| _like_clientid  | String | False    | 客户端标识符，子串方式模糊查找                 |
| _like_username  | String | False    | 客户端用户名，子串方式模糊查找                 |
| _gte_created_at | Integer| False    | 客户端会话创建时间，大于等于查找                |
| _lte_created_at | Integer| False    | 客户端会话创建时间，小于等于查找                |
| _gte_connected_at | Integer| False    | 客户端连接创建时间，大于等于查找                |
| _lte_connected_at | Integer| False    | 客户端连接创建时间，小于等于查找                |
| _gte_mqueue_len | Integer| False    | 客户端消息队列当前长度， 大于等于查找              |
| _lte_mqueue_len | Integer| False    | 客户端消息队列当前长度， 大于等于查找              |


**Success Response Body (JSON):**

| Name                  | Type | Description                                                                |
|-----------------------| --------- |----------------------------------------------------------------------------|
| []                    | Array of Objects | 所有客户端的信息                                                                   |
| [0].node_id           | Integer    | 客户端所连接的节点ID                                                                |
| [0].clientid          | String    | 客户端标识符                                                                     |
| [0].username          | String    | 客户端连接时使用的用户名                                                               |                                                                  |
| [0].proto_ver         | Integer   | 客户端使用的协议版本                                                                 |
| [0].ip_address        | String    | 客户端的 IP 地址                                                                 |
| [0].port              | Integer   | 客户端的端口                                                                     |                                                          |
| [0].connected_at      | String    | 客户端连接时间，格式为 "YYYY-MM-DD HH:mm:ss"                                          |
| [0].disconnected_at   | String    | 客户端离线时间，格式为 "YYYY-MM-DD HH:mm:ss"，<br/>此字段仅在 `connected` 为 `false` 时有效并被返回 |
| [0].connected         | Boolean   | 客户端是否处于连接状态                                                                |
| [0].keepalive         | Integer   | 保持连接时间，单位：秒                                                                |
| [0].clean_start       | Boolean   | 指示客户端是否使用了全新的会话                                                            |
| [0].expiry_interval   | Integer   | 会话过期间隔，单位：秒                                                                |
| [0].created_at        | String    | 会话创建时间，格式为 "YYYY-MM-DD HH:mm:ss"                                           |
| [0].subscriptions_cnt | Integer   | 此客户端已建立的订阅数量                                                               |
| [0].max_subscriptions | Integer   | 此客户端允许建立的最大订阅数量                                                            |
| [0].inflight          | Integer   | 飞行队列当前长度                                                                   |
| [0].max_inflight      | Integer   | 飞行队列最大长度                                                                   |
| [0].mqueue_len        | Integer   | 消息队列当前长度                                                                   |
| [0].max_mqueue        | Integer   | 消息队列最大长度                                                                   |


**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/clients?_limit=10"

[{"clean_start":true,"clientid":"be82ee31-7220-4cad-a724-aaad9a065012","connected":true,"connected_at":"2022-07-30 18:14:08","created_at":"2022-07-30 18:14:08","disconnected_at":"","expiry_interval":7200,"inflight":0,"ip_address":"183.193.169.110","keepalive":60,"max_inflight":16,"max_mqueue":1000,"max_subscriptions":0,"mqueue_len":0,"node_id":1,"port":10839,"proto_ver":4,"subscriptions_cnt":0,"username":"undefined"}]
```

### GET /api/v1/clients/{clientid}

返回指定客户端的信息

**Path Parameters:**

| Name   | Type | Required | Description |
| ------ | --------- | -------- |  ---- |
| clientid  | String | True | ClientID |

**Success Response Body (JSON):**

| Name | Type | Description |
|------| --------- | ----------- |
| {}   | Array of Objects | 客户端的信息，详细请参见<br/>[GET /api/v1/clients](#get-clients)|

**Examples:**

查询指定客户端

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/clients/example1"

{"clean_start":true,"clientid":"example1","connected":true,"connected_at":"2022-07-30 23:30:43","created_at":"2022-07-30 23:30:43","disconnected_at":"","expiry_interval":7200,"inflight":0,"ip_address":"183.193.169.110","keepalive":60,"max_inflight":16,"max_mqueue":1000,"max_subscriptions":0,"mqueue_len":0,"node_id":1,"port":11232,"proto_ver":4,"subscriptions_cnt":0,"username":"undefined"}
```

### DELETE /api/v1/clients/{clientid}

踢除指定客户端。注意踢除客户端操作会将连接与会话一并终结。

**Path Parameters:**

| Name   | Type | Required | Description |
| ------ | --------- | -------- |  ---- |
| clientid  | String | True | ClientID |

**Success Response Body (String):**

| Name       | Type             | Description |
|------------|------------------|-----------|
| id         | String          | 连接唯一ID    |

**Examples:**

踢除指定客户端

```bash
$ curl -i -X DELETE "http://localhost:6060/api/v1/clients/example1"

1@10.0.4.6:1883/183.193.169.110:10876/example1/dashboard
```

### GET /api/v1/clients/{clientid}/online

检查客户端是否在线

**Path Parameters:**

| Name   | Type | Required | Description |
| ------ | --------- | -------- |  ---- |
| clientid  | String | True | ClientID |

**Success Response Body (JSON):**

| Name | Type | Description |
|------|------|-------------|
| body | Bool | 是否在线        |

**Examples:**

检查客户端是否在线

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/clients/example1/online"

false
```


## 订阅信息

### GET /api/v1/subscriptions

返回集群下所有订阅信息。

**Query String Parameters:**

| Name   | Type | Required | Default | Description                                                                      |
| ------ | --------- | -------- | ------- |----------------------------------------------------------------------------------|
| _limit | Integer   | False | 10000   | 一次最多返回的数据条数，未指定时由 `rmqtt-http-api.toml` 插件的配置项 `max_row_limit` 决定 |

| Name         | Type    | Description |
| ------------ | ------- | ----------- |
| clientid     | String  | 客户端标识符   |
| topic        | String  | 主题，全等查询 |
| qos          | Enum    | 可取值为：`0`,`1`,`2` |
| share        | String  | 共享订阅的组名称 |
| _match_topic | String  | 主题，匹配查询 |


**Success Response Body (JSON):**

| Name             | Type | Description |
|------------------| --------- |-------------|
| []               | Array of Objects | 所有订阅信息      |
| [0].node_id     | Integer    | 节点ID        |
| [0].clientid | String    | 客户端标识符      |
| [0].topic    | String    | 订阅主题        |
| [0].qos      | Integer   | QoS 等级      |
| [0].share      | String   | 共享订阅的组名称      |


**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/subscriptions?_limit=10"

[{"node_id":1,"clientid":"example1","topic":"foo/#","qos":2,"share":null},{"node_id":1,"clientid":"example1","topic":"foo/+","qos":2,"share":"test"}]
```


### GET /api/v1/subscriptions/{clientid}

返回集群下指定客户端的订阅信息。

**Path Parameters:**

| Name   | Type | Required | Description |
| ------ | --------- | -------- |  ---- |
| clientid  | String | True | ClientID |

**Success Response Body (JSON):**

| Name             | Type | Description |
|------------------| --------- |-------------|
| []               | Array of Objects | 所有订阅信息      |
| [0].node_id     | Integer    | 节点ID        |
| [0].clientid | String    | 客户端标识符      |
| [0].topic    | String    | 订阅主题        |
| [0].qos      | Integer   | QoS 等级      |
| [0].share      | String   | 共享订阅的组名称      |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/subscriptions/example1"

[{"node_id":1,"clientid":"example1","topic":"foo/+","qos":2,"share":"test"},{"node_id":1,"clientid":"example1","topic":"foo/#","qos":2,"share":null}]
```


## 路由

### GET /api/v1/routes

返回集群下的所有路由信息。

**Query String Parameters:**

| Name   | Type | Required | Default | Description |
| ------ | --------- | -------- | ------- |  ---- |
| _limit | Integer   | False | 10000   | 一次最多返回的数据条数，未指定时由 `rmqtt-http-api.toml` 插件的配置项 `max_row_limit` 决定 |

**Success Response Body (JSON):**

| Name          | Type | Description |
|---------------| --------- |-------------|
| []            | Array of Objects | 所有路由信息      |
| [0].topic | String    | MQTT 主题     |
| [0].node_id  | Integer    | 节点ID        |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/routes"

[{"node_id":1,"topic":"foo/#"},{"node_id":1,"topic":"foo/+"}]
```

### GET /api/v1/routes/{topic}

返回集群下指定主题的路由信息。

**Path Parameters:**

| Name   | Type | Required | Description |
| ------ | --------- | -------- |  ---- |
| topic  | String   | True | 主题 |

**Success Response Body (JSON):**

| Name      | Type | Description |
|-----------| --------- |-------------|
| []        | Array of Objects | 所有路由信息      |
| [0].topic | String    | MQTT 主题     |
| [0].node_id | Integer    | 节点ID        |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/routes/foo%2f1"

[{"node_id":1,"topic":"foo/#"},{"node_id":1,"topic":"foo/+"}]
```


## 消息发布

### POST /api/v1/mqtt/publish

发布 MQTT 消息。

**Parameters (json):**

| Name     | Type | Required | Default | Description                             |
| -------- | --------- | -------- |--------|-----------------------------------------|
| topic    | String    | Optional |        | 主题，与 `topics` 至少指定其中之一                  |
| topics   | String    | Optional |        | 以 `,` 分割的多个主题，使用此字段能够同时发布消息到多个主题        |
| clientid | String    | Optional | system | 客户端标识符                            |
| payload  | String    | Required |        | 消息正文                                    |
| encoding | String    | Optional | plain  | 消息正文使用的编码方式，目前仅支持 `plain` 与 `base64` 两种 |
| qos      | Integer   | Optional | 0      | QoS 等级                                  |
| retain   | Boolean   | Optional | false  | 是否为保留消息                                 |


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

## 主题订阅

### POST /api/v1/mqtt/subscribe

订阅 MQTT 主题。

**Parameters (json):**

| Name     | Type | Required | Default | Description |
| -------- | --------- | -------- | ------- | ------------ |
| topic    | String    | Optional |         | 主题，与 `topics` 至少指定其中之一 |
| topics   | String    | Optional |         | 以 `,` 分割的多个主题，使用此字段能够同时订阅多个主题 |
| clientid | String    | Required |         | 客户端标识符 |
| qos      | Integer   | Optional | 0       | QoS 等级 |

**Success Response Body (JSON):**

| Name    | Type   | Description               |
|---------|--------|---------------------------|
| {}      | Object |                           |
| {topic} | Bool   | key为主题，值为订阅结果: true/false |


**Examples:**

同时订阅 `foo/a`, `foo/b`, `foo/c` 三个主题

```bash
$ curl -i -X POST "http://localhost:6060/api/v1/mqtt/subscribe" --header 'Content-Type: application/json' -d '{"topics":"foo/a,foo/b,foo/c","qos":1,"clientid":"example1"}'

{"foo/a":true,"foo/c":true,"foo/b":true}
```

### POST /api/v1/mqtt/unsubscribe

取消订阅。

**Parameters (json):**

| Name     | Type | Required | Default | Description  |
| -------- | --------- | -------- | ------- | ------------ |
| topic    | String    | Required |         | 主题         |
| clientid | String    | Required |         | 客户端标识符 |

**Success Response Body (JSON):**

| Name | Type | Description |
|------|------|-------------|
| body | Bool | true/false  |

**Examples:**

取消订阅 `foo/a` 主题

```bash
$ curl -i -X POST "http://localhost:6060/api/v1/mqtt/unsubscribe" --header 'Content-Type: application/json' -d '{"topic":"foo/a","clientid":"example1"}'

true
```

## 插件

### GET /api/v1/plugins

返回集群下的所有插件信息。

**Path Parameters:** 无

**Success Response Body (JSON):**

| Name                  | Type             | Description                      |
|-----------------------|------------------|----------------------------------|
| []                    | Array of Objects | 所有插件信息                           |
| [0].node              | Integer          | 节点ID                             |
| [0].plugins           | Array            | 插件信息，由对象组成的数组，见下文                |
| [0].plugins.name      | String           | 插件名称                             |
| [0].plugins.version   | String           | 插件版本                             |
| [0].plugins.descr     | String           | 插件描述                             |
| [0].plugins.active    | Boolean          | 插件是否启动                           |
| [0].plugins.inited    | Boolean          | 插件是否已经初始化                        |
| [0].plugins.immutable | Boolean          | 插件是否不可变，不可变插件将不能被停止，不有修改配置，不能重启等 |
| [0].plugins.attrs     | Json             | 插件其它附加属性                         |


**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/plugins"

[{"node":1,"plugins":[{"active":false,"attrs":null,"descr":null,"immutable":true,"inited":false,"name":"rmqtt-cluster-raft","version":null},{"active":false,"attrs":null,"descr":null,"immutable":false,"inited":false,"name":"rmqtt-auth-http","version":null},{"active":true,"attrs":null,"descr":"","immutable":true,"inited":true,"name":"rmqtt-acl","version":"0.1.1"},{"active":true,"attrs":null,"descr":"","immutable":false,"inited":true,"name":"rmqtt-counter","version":"0.1.0"},{"active":true,"attrs":null,"descr":"","immutable":false,"inited":true,"name":"rmqtt-http-api","version":"0.1.1"},{"active":false,"attrs":null,"descr":null,"immutable":false,"inited":false,"name":"rmqtt-web-hook","version":null},{"active":false,"attrs":null,"descr":null,"immutable":true,"inited":false,"name":"rmqtt-cluster-broadcast","version":null}]}]
```

### GET /api/v1/plugins/{node}

返回指定节点下的插件信息。

**Path Parameters:** 

| Name | Type | Required | Description                |
| ---- | --------- |----------|----------------------------|
| node | Integer    | True     | 节点名字，如：1 |


**Success Response Body (JSON):**

| Name           | Type             | Description                    |
|----------------|------------------|--------------------------------|
| []             | Array of Objects | 插件信息，由对象组成的数组，见下文      |
| [0].name       | String           | 插件名称                           |
| [0].version    | String           | 插件版本                           |
| [0].descr      | String           | 插件描述                           |
| [0].active     | Boolean          | 插件是否启动                         |
| [0].inited     | Boolean          | 插件是否已经初始化                      |
| [0].immutable  | Boolean          | 插件是否不可变，不可变插件将不能被停止，不有修改配置，不能重启等 |
| [0].attrs      | Json             | 插件其它附加属性                       |


**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/plugins/1"

[{"active":false,"attrs":null,"descr":null,"immutable":true,"inited":false,"name":"rmqtt-cluster-raft","version":null},{"active":false,"attrs":null,"descr":null,"immutable":false,"inited":false,"name":"rmqtt-auth-http","version":null},{"active":true,"attrs":null,"descr":"","immutable":true,"inited":true,"name":"rmqtt-acl","version":"0.1.1"},{"active":true,"attrs":null,"descr":"","immutable":false,"inited":true,"name":"rmqtt-counter","version":"0.1.0"},{"active":true,"attrs":null,"descr":"","immutable":false,"inited":true,"name":"rmqtt-http-api","version":"0.1.1"},{"active":false,"attrs":null,"descr":null,"immutable":false,"inited":false,"name":"rmqtt-web-hook","version":null},{"active":false,"attrs":null,"descr":null,"immutable":true,"inited":false,"name":"rmqtt-cluster-broadcast","version":null}]
```


### GET /api/v1/plugins/{node}/{plugin}

返回指定节点下指定插件名称的插件信息。

**Path Parameters:** 

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | 节点名字，如：1    |
| plugin | String    | True       | 插件名称        |

**Success Response Body (JSON):**

| Name           | Type            | Description                    |
|----------------|-----------------|--------------------------------|
| {}             | Object | 插件信息      |
| {}.name       | String          | 插件名称                           |
| {}.version    | String          | 插件版本                           |
| {}.descr      | String          | 插件描述                           |
| {}.active     | Boolean         | 插件是否启动                         |
| {}.inited     | Boolean         | 插件是否已经初始化                      |
| {}.immutable  | Boolean         | 插件是否不可变，不可变插件将不能被停止，不有修改配置，不能重启等 |
| {}.attrs      | Json            | 插件其它附加属性                       |


**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/plugins/1/rmqtt-web-hook"

{"active":false,"attrs":null,"descr":null,"immutable":false,"inited":false,"name":"rmqtt-web-hook","version":null}
```


### GET /api/v1/plugins/{node}/{plugin}/config

返回指定节点下指定插件名称的插件配置信息。

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | 节点名字，如：1    |
| plugin | String    | True       | 插件名称        |

**Success Response Body (JSON):**

| Name           | Type     | Description |
|----------------|----------|-------------|
| {}             | Object   | 插件配置信息      |



**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/plugins/1/rmqtt-http-api/config"

{"http_laddr":"0.0.0.0:6060","max_row_limit":10000,"workers":1}
```



### PUT /api/v1/plugins/{node}/{plugin}/config/reload

重新载入指定节点下指定插件名称的插件配置信息。

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | 节点名字，如：1    |
| plugin | String    | True       | 插件名称        |

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

加载指定节点下的指定插件。

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | 节点名字，如：1    |
| plugin | String    | True       | 插件名称        |

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

卸载指定节点下的指定插件。

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | 节点名字，如：1    |
| plugin | String    | True       | 插件名称        |

**Success Response Body (JSON):**

| Name | Type | Description |
|------|------|-------------|
| body | Bool | true/false  |



**Examples:**

```bash
$ curl -i -X PUT "http://localhost:6060/api/v1/plugins/1/rmqtt-web-hook/unload"

true
```

## 状态
### GET /api/v1/stats

<span id = "get-stats" />

返回集群下所有状态数据。

**Path Parameters:** 无

**Success Response Body (JSON):**

| Name          | Type             | Description   |
|---------------|------------------| ------------- |
| []            | Array of Objects | 各节点上的状态数据列表 |
| [0].node  | Json Object      | 节点信息 |
| [0].stats | Json Object      | 状态数据，详见下面的 *stats* |

**node:**

| Name          | Type    | Description |
|---------------|---------|-------------|
| id            | Integer | 节点ID       |
| name          | String  | 节点名称      |
| status        | String | 节点状态       |

**stats:**

| Name                       | Type | Description                |
|----------------------------| --------- | -------------------------- |
| connections.count          | Integer   | 当前连接数量               |
| connections.max            | Integer   | 连接数量的历史最大值       |
| sessions.count             | Integer   | 当前会话数量               |
| sessions.max               | Integer   | 会话数量的历史最大值       |
| topics.count               | Integer   | 当前主题数量               |
| topics.max                 | Integer   | 主题数量的历史最大值       |
| subscriptions.count        | Integer   | 当前订阅数量，包含共享订阅 |
| subscriptions.max          | Integer   | 订阅数量的历史最大值       |
| subscriptions_shared.count | Integer   | 当前共享订阅数量           |
| subscriptions_shared.max   | Integer   | 共享订阅数量的历史最大值   |
| routes.count               | Integer   | 当前路由数量               |
| routes.max                 | Integer   | 路由数量的历史最大值       |
| retained.count             | Integer   | 当前保留消息数量           |
| retained.max               | Integer   | 保留消息的历史最大值       |


**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/stats"

[{"node":{"id":1,"name":"1@127.0.0.1","status":"Running"},"stats":{"connections.count":1,"connections.max":2,"retained.count":2,"retained.max":2,"routes.count":3,"routes.max":4,"sessions.count":1,"sessions.max":2,"subscriptions.count":7,"subscriptions.max":8,"subscriptions_shared.count":1,"subscriptions_shared.max":2,"topics.count":3,"topics.max":4}}]
```


### GET /api/v1/stats/{node}

返回集群下指定节点的状态数据。

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | 节点名字，如：1    |

**Success Response Body (JSON):**

| Name          | Type                 | Description        |
|---------------|----------------------|--------------------|
| {}            | Object               | 各节点上的状态数据列表        |
| {}.node  | Json Object          | 节点信息               |
| {}.stats | Json Object          | 状态数据，详见下面的 *stats* |

**node:**

| Name          | Type    | Description |
|---------------|---------|-------------|
| id            | Integer | 节点ID       |
| name          | String  | 节点名称      |
| status        | String | 节点状态       |

**stats:**

| Name | Type | Description |
|------| --------- | ----------- |
| {}   | Json Object | 状态数据，详细请参见<br/>[GET /api/v1/stats](#get-stats)|

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/stats/1"

{"node":{"id":1,"name":"1@127.0.0.1","status":"Running"},"stats":{"connections.count":1,"connections.max":2,"retained.count":2,"retained.max":2,"routes.count":3,"routes.max":4,"sessions.count":1,"sessions.max":2,"subscriptions.count":7,"subscriptions.max":8,"subscriptions_shared.count":1,"subscriptions_shared.max":2,"topics.count":3,"topics.max":4}}
```


### GET /api/v1/stats/sum

汇总集群下所有节点状态数据。

**Path Parameters:** 无

**Success Response Body (JSON):**

| Name          | Type                 | Description        |
|---------------|----------------------|--------------------|
| {}            | Object               | 各节点上的状态数据列表        |
| {}.nodes  | Json Objects          | 节点信息               |
| {}.stats | Json Object          | 状态数据，详见下面的 *stats* |

**nodes:**

| Name        | Type     | Description    |
|-------------|----------|----------------|
| {id}        | Object   | 节点, key为节点ID  |
| {id}.name   | String   | 节点名称           |
| {id}.status | String   | 节点状态           |

**stats:**

| Name | Type | Description |
|------| --------- | ----------- |
| {}   | Json Object | 状态数据，详细请参见<br/>[GET /api/v1/stats](#get-stats)|

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/stats/sum"

{"nodes":{"1":{"name":"1@127.0.0.1","status":"Running"}},"stats":{"connections.count":1,"connections.max":2,"retained.count":2,"retained.max":2,"routes.count":3,"routes.max":4,"sessions.count":1,"sessions.max":2,"subscriptions.count":7,"subscriptions.max":8,"subscriptions_shared.count":1,"subscriptions_shared.max":2,"topics.count":3,"topics.max":4}}
```



## 统计指标
### GET /api/v1/metrics

<span id = "get-metrics" />

返回集群下所有统计指标数据。

**Path Parameters:** 无

**Success Response Body (JSON):**

| Name          | Type             | Description   |
|---------------|------------------| ------------- |
| []            | Array of Objects | 各节点上的统计指标列表 |
| [0].node  | Json Object      | 节点信息 |
| [0].metrics | Json Object      | 监控指标数据，详见下面的 *metrics* |

**node:**

| Name          | Type    | Description |
|---------------|---------|-------------|
| id            | Integer | 节点ID       |
| name          | String  | 节点名称      |

**metrics:**

| Name | Type | Description |
| ----------------| --------- | -------------------- |
| client.auth.anonymous           | Integer   | 匿名登录的客户端数量 |
| client.authenticate             | Integer   | 客户端认证次数 |
| client.connack                  | Integer   | 发送 CONNACK 报文的次数 |
| client.connect                  | Integer   | 客户端连接次数 |
| client.connected                | Integer   | 客户端成功连接次数 |
| client.disconnected             | Integer   | 客户端断开连接次数 |
| client.publish.check.acl        | Integer   | 发布，ACL 规则检查次数 |
| client.subscribe.check.acl      | Integer   | 订阅，ACL 规则检查次数 |
| client.subscribe                | Integer   | 客户端订阅次数 |
| client.unsubscribe              | Integer   | 客户端取消订阅次数 |
| messages.publish                | Integer   | 接收到PUBLISH消息数量 |
| messages.delivered              | Integer   | 内部转发到订阅端消息数量 |
| messages.acked                  | Integer   | 接收的 PUBACK 和 PUBREC 报文数量 |
| messages.dropped                | Integer   | 丢弃的消息总数 |
| session.created                 | Integer   | 创建的会话数量 |
| session.resumed                 | Integer   | 由于 `Clean Session` 或 `Clean Start` 为 `false` 而恢复的会话数量 |
| session.subscribed              | Integer   | 客户端成功订阅次数 |
| session.unsubscribed            | Integer   | 客户端成功取消订阅次数 |
| session.terminated              | Integer   | 终结的会话数量 |


**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/metrics"

[{"metrics":{"client.auth.anonymous":38,"client.authenticate":47,"client.connack":47,"client.connect":47,"client.connected":47,"client.disconnected":46,"client.publish.check.acl":50,"client.subscribe":37,"client.subscribe.check.acl":15,"client.unsubscribe":8,"messages.acked":35,"messages.delivered":78,"messages.dropped":0,"messages.publish":78,"session.created":45,"session.resumed":2,"session.subscribed":15,"session.terminated":42,"session.unsubscribed":8},"node":{"id":1,"name":"1@127.0.0.1"}}]
```


### GET /api/v1/metrics/{node}

返回集群下指定节点的统计指标数据。

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | 节点名字，如：1    |

**Success Response Body (JSON):**

| Name          | Type                 | Description        |
|---------------|----------------------|--------------------|
| {}            | Object               | 各节点上的统计指标列表        |
| {}.node  | Json Object          | 节点信息               |
| {}.metrics | Json Object          | 监控指标数据，详见下面的 *metrics* |

**node:**

| Name          | Type    | Description |
|---------------|---------|-------------|
| id            | Integer | 节点ID       |
| name          | String  | 节点名称      |

**metrics:**

| Name | Type | Description |
|------| --------- | ----------- |
| {}   | Json Object | 统计指标数据，详细请参见<br/>[GET /api/v1/metrics](#get-metrics)|

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/metrics/1"

{"metrics":{"client.auth.anonymous":38,"client.authenticate":47,"client.connack":47,"client.connect":47,"client.connected":47,"client.disconnected":46,"client.publish.check.acl":50,"client.subscribe":37,"client.subscribe.check.acl":15,"client.unsubscribe":8,"messages.acked":35,"messages.delivered":78,"messages.dropped":0,"messages.publish":78,"session.created":45,"session.resumed":2,"session.subscribed":15,"session.terminated":42,"session.unsubscribed":8},"node":{"id":1,"name":"1@127.0.0.1"}}
```


### GET /api/v1/metrics/sum

汇总集群下指定节点的统计指标数据。

**Path Parameters:** 无

**Success Response Body (JSON):**

| Name | Type | Description |
|------| --------- | ----------- |
| {}   | Json Object | 统计指标数据，详细请参见<br/>[GET /api/v1/metrics](#get-metrics)|

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/metrics/sum"

{"client.auth.anonymous":38,"client.authenticate":47,"client.connack":47,"client.connect":47,"client.connected":47,"client.disconnected":46,"client.publish.check.acl":50,"client.subscribe":37,"client.subscribe.check.acl":15,"client.unsubscribe":8,"messages.acked":35,"messages.delivered":78,"messages.dropped":0,"messages.publish":78,"session.created":45,"session.resumed":2,"session.subscribed":15,"session.terminated":42,"session.unsubscribed":8}
```











