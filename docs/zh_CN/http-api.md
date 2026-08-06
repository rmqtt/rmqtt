[English](../en_US/http-api.md)  | 简体中文

# HTTP API

RMQTT 提供了 HTTP API 以实现与外部系统的集成，例如查询客户端信息、发布消息等。

RMQTT 的 HTTP API 服务默认监听 6060 端口，可通过 `etc/plugins/rmqtt-http-api.toml` 配置文件修改监听端口。所有 API 调用均以 `api/v1` 开头。

#### 插件：

```bash
rmqtt-http-api
```

#### 插件配置文件：

```bash
plugins/rmqtt-http-api.toml
```

#### 插件配置项：

```bash
##--------------------------------------------------------------------
## rmqtt-http-api
##--------------------------------------------------------------------

# See more keys and their definitions at https://github.com/rmqtt/rmqtt/blob/master/docs/en_US/http-api.md

## Max Row Limit
max_row_limit = 10_000
## HTTP Listener address
http_laddr = "0.0.0.0:6060"
## HTTP bearer token for API authentication.
## When set, all HTTP API requests must include an `Authorization: Bearer <token>` header.
## When not set (default), no authentication is required.
#http_bearer_token = "public"

## Enable TCP SO_REUSEADDR on the HTTP listener.
## Default: true
# http_reuseaddr = true

## Enable TCP SO_REUSEPORT on the HTTP listener.
## Default: false
# http_reuseport = false

## Indicates whether to print HTTP request logs
http_request_log = false

## Metrics sample interval for collecting and caching internal metrics.
## Default: "5s"
# metrics_sample_interval = "5s"

## gRPC message type identifier for HTTP API messages.
## Default: 99
# message_type = 99

##Message expiration time, 0 means no expiration
message_expiry_interval = "5m"

## Prometheus metrics data caching interval.
## Default: "5s"
prometheus_metrics_cache_interval = "5s"

## Dashboard static directory (optional).
## By default (when unset), the Dashboard SPA is served from assets embedded
## directly into the binary via rust-embed at compile time, so no external
## files are needed.
## If set, the plugin loads the Dashboard SPA from this external directory
## at the `/dashboard/` path instead, allowing swapping the frontend build
## without recompiling the binary.
# dashboard_static_dir = "/path/to/dashboard/dist"

##─── Stats/Metrics History Persistence (optional) ───────────────────────
## When `storage` is configured, the plugin periodically snapshots Stats
## and Metrics, converts them to JSON, and writes them to the backend with
## TTL-based expiration. History query APIs
## (`/api/v1/stats/history`, `/api/v1/metrics/history`, etc.) become available.
## To disable, omit the entire `storage` section.

##─── Redb backend ──────────────────────────────────────────────────────
storage.type = "redb"
storage.redb.path = "/var/log/rmqtt/.cache/http-api-history/{node}.redb"

##─── Sled backend ──────────────────────────────────────────────────────
#storage.type = "sled"
#storage.sled.path = "/var/log/rmqtt/.cache/http-api-history/{node}.sled"
#storage.sled.cache_capacity = "1G"

##─── Redis backend ──────────────────────────────────────────────────────
# storage.type = "redis"
# storage.redis.url = "redis://127.0.0.1:6379/"
# storage.redis.prefix = "http-api-history-{node}"

##─── Redis Cluster backend ──────────────────────────────────────────────
# storage.type = "redis-cluster"
# storage.redis-cluster.urls = ["redis://127.0.0.1:6380/", "redis://127.0.0.1:6381/"]
# storage.redis-cluster.prefix = "http-api-history-{node}"

##─── Flush interval (how often to snapshot Stats/Metrics) ───────────────
## Default: "5s"
# flush_interval = "5s"

##─── History retention (TTL for each data point) ────────────────────────
## Default: "7d"
# history_retention = "7d"
```

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

| Name           | Type | Description                                            |
|----------------| --------- |--------------------------------------------------------|
| {}/[]          | Object/Array of Objects | node 参数存在时返回指定节点信息，<br/>不存在时返回所有节点的信息                  |
| .datetime      | String    | 当前时间，格式为 "YYYY-MM-DD HH:mm:ss"                         |
| .node_id       | Integer    | 节点ID                                                   |
| .node_name     | String    | 节点名称                                                   |
| .running       | Bool    | 节点是否正常                                                   |
| .sysdescr      | String    | 软件描述                                                   |
| .uptime        | String    | RMQTT 运行时间，格式为 "D days, H hours, m minutes, s seconds" |
| .version       | String    | RMQTT 版本                                               |
| .rustc_version | String    | RUSTC 版本                                               |


**Examples:**

获取所有节点的基本信息：

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/brokers"

[{"datetime":"2022-07-24 23:01:31","node_id":1,"node_name":"1@127.0.0.1","running":true,"sysdescr":"RMQTT Broker","uptime":"5 days 23 hours, 16 minutes, 3 seconds","version":"rmqtt/0.21.0"}]
```

获取节点 1 的基本信息：

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/brokers/1"

{"datetime":"2022-07-24 23:01:31","node_id":1,"node_name":"1@127.0.0.1","running":true,"sysdescr":"RMQTT Broker","uptime":"5 days 23 hours, 17 minutes, 15 seconds","version":"rmqtt/0.21.0"}
```

## 节点

### GET /api/v1/nodes/{node}

返回节点的状态。

**Path Parameters:**

| Name | Type | Required | Description                 |
| ---- | --------- | ------------|-----------------------------|
| node | Integer    | False       | 节点ID，如：1 <br/>不指定时返回所有节点的信息 |

**Success Response Body (JSON):**

| Name            | Type                    | Description                                     |
|-----------------|-------------------------|-------------------------------------------------|
| {}/[]           | Object/Array of Objects | node 参数存在时返回指定节点信息，<br/>不存在时以 Array 形式返回所有节点的信息 |
| .boottime       | String                  | 操作系统启动时间                                        |
| .connections    | Integer                 | 当前接入此节点的客户端数量                                   |
| .disk_free      | Integer                 | 磁盘可用容量（字节）                                      |
| .disk_total     | Integer                 | 磁盘总容量（字节）                                       |
| .load1          | Float                   | 1 分钟内的 CPU 平均负载                                 |
| .load5          | Float                   | 5 分钟内的 CPU 平均负载                                 |
| .load15         | Float                   | 15 分钟内的 CPU 平均负载                                |
| .memory_free    | Integer                 | 系统可用内存大小（字节）                                    |
| .memory_total   | Integer                 | 系统总内存大小（字节）                                     |
| .memory_used    | Integer                 | 系统已占用的内存大小 （字节）                                 |
| .node_id        | Integer                 | 节点ID                                            |
| .node_name      | String                  | 节点名称                                            |
| .running        | Bool                    | 节点是否正常                                          |
| .uptime         | String                  | RMQTT 运行时间                                      |
| .version        | String                  | RMQTT 版本                                        |
| .rustc_version  | String                  | RUSTC 版本                                        |

**Examples:**

获取所有节点的状态：

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/nodes"

[{"boottime":"2022-06-30 05:20:24 UTC","connections":1,"disk_free":77382381568,"disk_total":88692346880,"load1":0.0224609375,"load15":0.0,"load5":0.0263671875,"memory_free":1457954816,"memory_total":2084057088,"memory_used":626102272,"node_id":1,"node_name":"1@127.0.0.1","running":true,"uptime":"5 days 23 hours, 33 minutes, 0 seconds","version":"rmqtt/0.21.0","rustc_version":"1.85.0"}]
```

获取指定节点的状态：

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/nodes/1"

{"boottime":"2022-06-30 05:20:24 UTC","connections":1,"disk_free":77382381568,"disk_total":88692346880,"load1":0.0224609375,"load15":0.0,"load5":0.0263671875,"memory_free":1457954816,"memory_total":2084057088,"memory_used":626102272,"node_id":1,"node_name":"1@127.0.0.1","running":true,"uptime":"5 days 23 hours, 33 minutes, 0 seconds","version":"rmqtt/0.21.0","rustc_version":"1.85.0"}
```

## 功能支持

### GET /api/v1/features

返回集群各节点的功能支持状态及一致性汇总。功能支持状态由各功能的 trait 实现（`enable()` / `is_supported()`）决定。

**Parameters:** 无

**Success Response Body (JSON):**

| Name          | Type | Description |
|---------------|------|-------------|
| consistent    | Bool | 所有可达节点功能状态是否完全一致；`false` 说明存在节点配置漂移或插件加载失败 |
| node_count    | Integer | 参与一致性比较的节点数量 |
| conflicts     | Array | 取值不一致的字段（按值分组列出节点）；`consistent` 为 `true` 时为空数组 |
| - conflicts[i].feature | String | 功能名称，如 `retain` |
| - conflicts[i].values  | Array | 取值分组，每组包含 `value`（Bool）与 `node_ids`（Integer Array） |
| nodes         | Array | 逐节点明细；不可达节点为错误字符串且不参与一致性比较 |
| - nodes[i].node_id    | Integer | 节点ID |
| - nodes[i].node_name  | String | 节点名称 |
| - nodes[i].features   | Object | 六项功能支持状态：`retain`、`message_storage`、`session_storage`、`delayed`、`shared_subscription`、`auto_subscription` |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/features"
```

```json
{
  "consistent": false,
  "node_count": 3,
  "conflicts": [
    {
      "feature": "retain",
      "values": [
        { "value": true,  "node_ids": [1, 2] },
        { "value": false, "node_ids": [3] }
      ]
    }
  ],
  "nodes": [
    {
      "node_id": 1,
      "node_name": "rmqtt@127.0.0.1",
      "features": {
        "retain": true,
        "message_storage": false,
        "session_storage": false,
        "delayed": true,
        "shared_subscription": true,
        "auto_subscription": false
      }
    }
  ]
}
```

> 说明：检测到不一致时后端会输出 `features inconsistent across cluster` 警告日志。单节点查询使用 `GET /api/v1/features/{node}`，直接返回该节点的 `FeaturesInfo` 对象（不含一致性汇总）。

### GET /api/v1/features/{node}

返回指定节点的功能支持状态。

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | -------- |-------------|
| node | Integer    | True      | 节点ID，如：1 |

**Success Response Body (JSON):**

| Name          | Type | Description |
|---------------|------|-------------|
| node_id       | Integer | 节点ID |
| node_name     | String | 节点名称 |
| features      | Object | 六项功能支持状态：`retain`、`message_storage`、`session_storage`、`delayed`、`shared_subscription`、`auto_subscription` |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/features/1"

{"node_id":1,"node_name":"rmqtt@127.0.0.1","features":{"retain":true,"message_storage":false,"session_storage":false,"delayed":true,"shared_subscription":true,"auto_subscription":false}}
```

## 健康检查

### GET /api/v1/health/check

返回集群所有节点的健康状态。

**Parameters:** 无

**Success Response Body (JSON):**

| Name                      | Type             | Description       |
|---------------------------|------------------|-------------------|
| {}                        | Object           | 健康检查信息           |
| {}.status                 | String           | 集群整体状态: "Running" 或 "Degraded" |
| {}.nodes                  | Object           | 各节点健康状态，key 为节点ID |
| {}.nodes.{id}             | Json Object      | 节点健康状态详细信息        |
| {}.nodes.{id}.name        | String           | 节点名称              |
| {}.nodes.{id}.running     | Bool             | 节点是否正常运行          |
| {}.nodes.{id}.uptime      | String           | 节点运行时长            |
| {}.nodes.{id}.status      | String           | 节点状态              |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/health/check"

{"status":"Running","nodes":{"1":{"name":"1@127.0.0.1","running":true,"uptime":"5d 23h 33m","status":"Running"}}}
```

### GET /api/v1/health/check/{node}

查询指定节点的健康状态。

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | 节点ID，如：1    |

**Success Response Body (JSON):**

| Name          | Type    | Description |
|---------------|---------|-------------|
| {}            | Object  | 节点健康状态     |
| .name         | String  | 节点名称       |
| .running      | Bool    | 节点是否正常运行   |
| .uptime       | String  | 节点运行时长     |
| .status       | String  | 节点状态       |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/health/check/1"

{"name":"1@127.0.0.1","running":true,"uptime":"5d 23h 33m","status":"Running"}
```

## 客户端

### GET /api/v1/clients

<span id = "get-clients" />

返回集群下所有客户端的信息。

**Query String Parameters:**

| Name   | Type | Required | Default | Description |
| ------ | --------- | -------- | ------- |  ---- |
| _limit | Integer   | False | 10000   | 一次最多返回的数据条数，未指定时由 `rmqtt-http-api.toml` 插件的配置项 `max_row_limit` 决定 |

| Name            | Type   | Required | Description         |
| --------------- | ------ | -------- |---------------------|
| clientid        | String | False    | 客户端标识符，全等查询       |
| username        | String | False    | 客户端用户名，全等查询       |
| ip_address      | String | False    | 客户端 IP 地址，全等查询    |
| connected       | Bool   | False    | 客户端当前连接状态          |
| clean_start     | Bool   | False    | 客户端是否使用了全新的会话    |
| session_present | Bool   | False    | 客户端是否连接到已经存在的会话    |
| proto_ver       | Integer| False    | 客户端协议版本, 3,4,5      |
| _like_clientid  | String | False    | 客户端标识符，子串方式模糊查找     |
| _like_username  | String | False    | 客户端用户名，子串方式模糊查找     |
| _gte_created_at | String | False    | 客户端会话创建时间，大于等于查找。<br/>格式为 `"YYYY-MM-DD HH:mm:ss"`（如 `"2026-07-29 21:25:37"`），<br/>也支持 Unix 秒级时间戳（如 `1690000000`）    |
| _lte_created_at | String | False    | 客户端会话创建时间，小于等于查找。<br/>格式为 `"YYYY-MM-DD HH:mm:ss"`（如 `"2026-07-29 21:25:37"`），<br/>也支持 Unix 秒级时间戳（如 `1690000000`）    |
| _gte_connected_at | String | False    | 客户端连接创建时间，大于等于查找。<br/>格式为 `"YYYY-MM-DD HH:mm:ss"`（如 `"2026-07-29 21:25:37"`），<br/>也支持 Unix 秒级时间戳（如 `1690000000`）    |
| _lte_connected_at | String | False    | 客户端连接创建时间，小于等于查找。<br/>格式为 `"YYYY-MM-DD HH:mm:ss"`（如 `"2026-07-29 21:25:37"`），<br/>也支持 Unix 秒级时间戳（如 `1690000000`）    |
| _gte_mqueue_len | Integer| False    | 客户端消息队列当前长度， 大于等于查找 |
| _lte_mqueue_len | Integer| False    | 客户端消息队列当前长度， 小于等于查找 |

**Success Response Body (JSON):**

| Name                    | Type             | Description                                                                |
|-------------------------|------------------|----------------------------------------------------------------------------|
| []                      | Array of Objects | 所有客户端的信息                                                                   |
| [0].node_id             | Integer          | 客户端所连接的节点ID                                                                |
| [0].clientid            | String           | 客户端标识符                                                                     |
| [0].username            | String           | 客户端连接时使用的用户名                                                               | 
| [0].superuser           | Boolean          | 客户端是否为超级用户                                                                 |
| [0].proto_ver           | Integer          | 客户端使用的协议版本                                                                 |
| [0].ip_address          | String           | 客户端的 IP 地址                                                                 |
| [0].port                | Integer          | 客户端的端口                                                                     | 
| [0].connected_at        | String           | 客户端连接时间，格式为 "YYYY-MM-DD HH:mm:ss"                                          |
| [0].disconnected_at     | String           | 客户端离线时间，格式为 "YYYY-MM-DD HH:mm:ss"，<br/>此字段仅在 `connected` 为 `false` 时有效并被返回 |
| [0].disconnected_reason | String           | 客户端离线原因                                                                    |
| [0].connected           | Boolean          | 客户端是否处于连接状态                                                                |
| [0].keepalive           | Integer          | 保持连接时间，单位：秒                                                                |
| [0].clean_start         | Boolean          | 指示客户端是否使用了全新的会话                                                            |
| [0].session_present     | Boolean          | 客户端是否连接到现有会话                                                                |
| [0].expiry_interval     | Integer          | 会话过期间隔，单位：秒                                                                |
| [0].created_at          | String           | 会话创建时间，格式为 "YYYY-MM-DD HH:mm:ss"                                           |
| [0].subscriptions_cnt   | Integer          | 此客户端已建立的订阅数量                                                               |
| [0].max_subscriptions   | Integer          | 此客户端允许建立的最大订阅数量                                                            |
| [0].inflight            | Integer          | 飞行队列当前长度                                                                   |
| [0].max_inflight        | Integer          | 飞行队列最大长度                                                                   |
| [0].mqueue_len          | Integer          | 消息队列当前长度                                                                   |
| [0].max_mqueue          | Integer          | 消息队列最大长度                                                                   |
| [0].last_will           | Json             | 遗嘱消息, 例如：{ "message": "dGVzdCAvdGVzdC9sd3QgLi4u", "qos": 1, "retain": false, "topic": "/test/lwt" } |


**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/clients?_limit=10"

[{"clean_start":true,"session_present":true,"clientid":"be82ee31-7220-4cad-a724-aaad9a065012","connected":true,"connected_at":"2022-07-30 18:14:08","created_at":"2022-07-30 18:14:08","disconnected_at":"","expiry_interval":7200,"inflight":0,"ip_address":"183.193.169.110","keepalive":60,"max_inflight":16,"max_mqueue":1000,"max_subscriptions":0,"mqueue_len":0,"node_id":1,"port":10839,"proto_ver":4,"subscriptions_cnt":0,"superuser":false,"username":"undefined"}]
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
| {}   | Object | 客户端的信息，详细字段请参见<br/>[GET /api/v1/clients](#get-clients)|

**Examples:**

查询指定客户端

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/clients/example1"

{"clean_start":true,"session_present":true,"clientid":"example1","connected":true,"connected_at":"2022-07-30 23:30:43","created_at":"2022-07-30 23:30:43","disconnected_at":"","expiry_interval":7200,"inflight":0,"ip_address":"183.193.169.110","keepalive":60,"max_inflight":16,"max_mqueue":1000,"max_subscriptions":0,"mqueue_len":0,"node_id":1,"port":11232,"proto_ver":4,"subscriptions_cnt":0,"superuser":false,"username":"undefined"}
```

### GET /api/v1/clients/offlines

返回集群下所有离线客户端的信息。参数及响应与 [GET /api/v1/clients](#get-clients) 相同，但仅返回 `connected` 为 `false` 的客户端。

**Query String Parameters:** 同 [GET /api/v1/clients](#get-clients)

**Success Response Body (JSON):** 同 [GET /api/v1/clients](#get-clients)

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/clients/offlines"

[{"clean_start":false,"session_present":false,"clientid":"example1","connected":false,"connected_at":"","created_at":"2022-07-30 18:14:08","disconnected_at":"2022-07-30 23:30:43","disconnected_reason":"normal","expiry_interval":7200,"inflight":0,"ip_address":"183.193.169.110","keepalive":60,"max_inflight":16,"max_mqueue":1000,"max_subscriptions":0,"mqueue_len":0,"node_id":1,"port":10839,"proto_ver":4,"subscriptions_cnt":0,"superuser":false,"username":"undefined"}]
```

### DELETE /api/v1/clients/{clientid}

踢除指定客户端。注意踢除客户端操作会将连接与会话一并终结。

**Path Parameters:**

| Name   | Type | Required | Description |
| ------ | --------- | -------- |  ---- |
| clientid  | String | True | ClientID |

**Success Response Body (String):**

直接返回连接唯一标识的字符串，格式为 `{node_id}@{ip}:{port}/{clientid}/{username}`。

**Examples:**

踢除指定客户端

```bash
$ curl -i -X DELETE "http://localhost:6060/api/v1/clients/example1"

1@10.0.4.6:1883/183.193.169.110:10876/example1/dashboard
```

### DELETE /api/v1/clients/offlines

批量踢除集群下所有满足查询条件的离线客户端。

**Query String Parameters:** 同 [GET /api/v1/clients](#get-clients)（注意：`connected` 参数会被强制设为 `false`）

**Success Response Body (JSON):**

| Name    | Type    | Description    |
|---------|---------|----------------|
| count   | Integer | 成功踢除的客户端数量 |

**Examples:**

```bash
$ curl -i -X DELETE "http://localhost:6060/api/v1/clients/offlines?clientid=example1"

{"count":1}
```

### GET /api/v1/clients/{clientid}/online

检查客户端是否在线

**Path Parameters:**

| Name   | Type | Required | Description |
| ------ | --------- | -------- |  ---- |
| clientid  | String | True | ClientID |

**Success Response Body (JSON):**

直接返回布尔值 `true` 或 `false`，表示客户端是否在线。

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
| clientid     | String  | 客户端标识符，全等查询   |
| topic        | String  | 主题，全等查询 |
| qos          | Enum    | 可取值为：`0`,`1`,`2` |
| share        | String  | 共享订阅的组名称 |
| _match_topic | String  | 主题，通配符匹配查询 |

**Success Response Body (JSON):**

| Name            | Type             | Description |
|-----------------|------------------|-------------|
| []              | Array of Objects | 所有订阅信息      |
| [0].node_id     | Integer          | 节点ID        |
| [0].clientid    | String           | 客户端标识符      |
| [0].client_addr | String           | 客户端IP地址和端口  |
| [0].topic       | String           | 订阅主题        |
| [0].qos         | Integer          | QoS 等级      |
| [0].share       | String           | 共享订阅的组名称    |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/subscriptions?_limit=10"

[{"node_id":1,"clientid":"example1","topic":"foo/#","qos":2,"share":null},{"node_id":1,"clientid":"example1","topic":"foo/+","qos":2,"share":"test"}]
```

### GET /api/v1/subscriptions/{clientid}

返回指定客户端的订阅信息。

**Path Parameters:**

| Name   | Type | Required | Description |
| ------ | --------- | -------- |  ---- |
| clientid  | String | True | ClientID |

**Success Response Body (JSON):**

| Name            | Type             | Description |
|-----------------|------------------|-------------|
| []              | Array of Objects | 所有订阅信息      |
| [0].node_id     | Integer          | 节点ID        |
| [0].clientid    | String           | 客户端标识符      |
| [0].client_addr | String           | 客户端IP地址和端口  |
| [0].topic       | String           | 订阅主题        |
| [0].qos         | Integer          | QoS 等级      |
| [0].share       | String           | 共享订阅的组名称      |

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

## 保留消息

### GET /api/v1/retains

查询保留消息。保留消息在集群中通过广播保持各节点同步，单节点查询即覆盖全集群。

**Query String Parameters:**

| Name          | Type    | Required | Default       | Description |
|---------------|---------|----------|---------------|-------------|
| topic_filter  | String  | False    | `#`           | 主题过滤器，支持 `#` / `+` 通配；为空或 `#` 时走全量分页 |
| offset        | Integer | False    | 0             | 分页偏移量 |
| limit         | Integer | False    | `max_row_limit` | 每页条数，超出 `max_row_limit` 时收敛 |

**Success Response Body (JSON):**

| Name                     | Type | Description |
|--------------------------|------|-------------|
| items                    | Array | 保留消息列表 |
| - items[i].topic         | String | 主题 |
| - items[i].msg_id        | Integer | 消息ID |
| - items[i].from          | Object | 发布者信息（`id.node_id` / `id.client_id`） |
| - items[i].publish       | Object | 消息内容，`payload` 为 base64 编码 |
| - items[i].publish.qos   | Integer | QoS 等级 |
| - items[i].publish.retain | Bool | retain 标记 |
| - items[i].publish.create_time | Integer | 发布时间（毫秒时间戳） |
| - items[i].remaining_ttl | Integer/Null | 剩余存活时间（秒）；全量分页路径返回，过滤路径为 `null` |
| has_more                 | Bool | 是否还有更多数据 |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/retains?topic_filter=%2Fiot%2Fb%2Fx&offset=0&limit=10"
```

```json
{
  "items": [
    {
      "topic": "/iot/b/x",
      "msg_id": 1024,
      "from": { "typ": "client", "id": { "node_id": 1, "client_id": "c1" } },
      "publish": {
        "topic": "/iot/b/x",
        "qos": 1,
        "retain": true,
        "dup": false,
        "payload": "<base64 编码>",
        "create_time": 1780000000000,
        "properties": null
      },
      "remaining_ttl": 3599
    }
  ],
  "has_more": false
}
```

> 说明：`topic_filter=#`（全量）路径由存储层分页并附带 `remaining_ttl`（剩余秒数）；指定 `topic_filter` 的过滤路径在内存分页，`remaining_ttl` 为 `null`。查询需要启用 `rmqtt-retainer` 插件。

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
| properties | Object   | Optional |        | 发布属性 (MQTT v5)<br/>可选子字段：<br/>- `message_expiry_interval`: Integer, 消息过期间隔(秒)<br/>- `topic_alias`: Integer<br/>- `response_topic`: String<br/>- `correlation_data`: String (Base64)<br/>- `user_properties`: Object |

**Success Response Body (String):**

成功返回字符串 `ok`。

**Examples:**

```bash
$ curl -i -X POST "http://localhost:6060/api/v1/mqtt/publish" --header 'Content-Type: application/json' -d '{"topic":"foo/1","payload":"Hello World","qos":1,"retain":false,"clientid":"example"}'

ok

$ curl -i -X POST "http://localhost:6060/api/v1/mqtt/publish" --header 'Content-Type: application/json' -d '{"topic":"foo/1","payload":"SGVsbG8gV29ybGQ=","qos":1,"encoding":"base64"}'

ok

$ curl -i -X POST "http://localhost:6060/api/v1/mqtt/publish" --header 'Content-Type: application/json' -d '{"topics":"foo/1,foo/2,foo/3","payload":"Hello","qos":0}'

ok

$ curl -i -X POST "http://localhost:6060/api/v1/mqtt/publish" --header 'Content-Type: application/json' -d '{"topic":"foo/1","payload":"Hello","qos":2,"retain":true,"properties":{"message_expiry_interval":3600,"response_topic":"res/foo","user_properties":{"key1":"val1"}}}'

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
| {topic} | Bool / String | key 为主题，值为订阅结果: `true`(成功) / `false`(失败)<br/>当订阅失败时，值可能为错误描述字符串 |

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

**Success Response Body:**

本地节点取消订阅成功时返回 JSON `true`；如果会话在其他节点上，则返回文本 `ok`。

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
| [0].plugins.authors   | String           | 插件作者                             |
| [0].plugins.homepage  | String           | 插件主页                             |
| [0].plugins.license   | String           | 插件许可证                            |
| [0].plugins.repository| String           | 插件仓库                             |
| [0].plugins.active    | Boolean          | 插件是否启动                           |
| [0].plugins.inited    | Boolean          | 插件是否已经初始化                        |
| [0].plugins.immutable | Boolean          | 插件是否不可变，不可变插件将不能被停止，不能修改配置，不能重启等 |
| [0].plugins.attrs     | Json             | 插件其它附加属性                         |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/plugins"

[{"node":1,"plugins":[{"active":false,"attrs":null,"descr":null,"immutable":true,"inited":false,"name":"rmqtt-cluster-raft","version":null},{"active":false,"attrs":null,"descr":null,"immutable":false,"inited":false,"name":"rmqtt-auth-http","version":null},{"active":true,"attrs":null,"descr":"","immutable":true,"inited":true,"name":"rmqtt-acl","version":"0.21.0"},{"active":true,"attrs":null,"descr":"","immutable":false,"inited":true,"name":"rmqtt-counter","version":"0.21.0"},{"active":true,"attrs":null,"descr":"","immutable":false,"inited":true,"name":"rmqtt-http-api","version":"0.21.0"},{"active":false,"attrs":null,"descr":null,"immutable":false,"inited":false,"name":"rmqtt-web-hook","version":null},{"active":false,"attrs":null,"descr":null,"immutable":true,"inited":false,"name":"rmqtt-cluster-broadcast","version":null}]}]
```

### GET /api/v1/plugins/{node}

返回指定节点下的插件信息。

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- |----------|-------------|
| node | Integer    | True     | 节点ID，如：1    |

**Success Response Body (JSON):**

| Name           | Type             | Description                    |
|----------------|------------------|--------------------------------|
| []             | Array of Objects | 插件信息，由对象组成的数组，见下文      |
| [0].name       | String           | 插件名称                           |
| [0].version    | String           | 插件版本                           |
| [0].descr      | String           | 插件描述                           |
| [0].authors    | String           | 插件作者                           |
| [0].homepage   | String           | 插件主页                           |
| [0].license    | String           | 插件许可证                          |
| [0].repository | String           | 插件仓库                           |
| [0].active     | Boolean          | 插件是否启动                         |
| [0].inited     | Boolean          | 插件是否已经初始化                      |
| [0].immutable  | Boolean          | 插件是否不可变，不可变插件将不能被停止，不能修改配置，不能重启等 |
| [0].attrs      | Json             | 插件其它附加属性                       |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/plugins/1"

[{"active":false,"attrs":null,"descr":null,"immutable":true,"inited":false,"name":"rmqtt-cluster-raft","version":null},{"active":false,"attrs":null,"descr":null,"immutable":false,"inited":false,"name":"rmqtt-auth-http","version":null},{"active":true,"attrs":null,"descr":"","immutable":true,"inited":true,"name":"rmqtt-acl","version":"0.21.0"},{"active":true,"attrs":null,"descr":"","immutable":false,"inited":true,"name":"rmqtt-counter","version":"0.21.0"},{"active":true,"attrs":null,"descr":"","immutable":false,"inited":true,"name":"rmqtt-http-api","version":"0.21.0"},{"active":false,"attrs":null,"descr":null,"immutable":false,"inited":false,"name":"rmqtt-web-hook","version":null},{"active":false,"attrs":null,"descr":null,"immutable":true,"inited":false,"name":"rmqtt-cluster-broadcast","version":null}]
```

### GET /api/v1/plugins/{node}/{plugin}

返回指定节点下指定插件名称的插件信息。

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | 节点ID，如：1    |
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
| {}.immutable  | Boolean         | 插件是否不可变，不可变插件将不能被停止，不能修改配置，不能重启等 |
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
| node | Integer    | True       | 节点ID，如：1    |
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
| node | Integer    | True       | 节点ID，如：1    |
| plugin | String    | True       | 插件名称        |

**Success Response Body:**

成功返回 JSON `true`。

**Examples:**

```bash
$ curl -i -X PUT "http://localhost:6060/api/v1/plugins/1/rmqtt-http-api/config/reload"

true
```

### PUT /api/v1/plugins/{node}/{plugin}/load

加载指定节点下的指定插件。

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | 节点ID，如：1    |
| plugin | String    | True       | 插件名称        |

**Success Response Body:**

成功返回 JSON `true`。

**Examples:**

```bash
$ curl -i -X PUT "http://localhost:6060/api/v1/plugins/1/rmqtt-web-hook/load"

true
```

### PUT /api/v1/plugins/{node}/{plugin}/unload

卸载指定节点下的指定插件。

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | 节点ID，如：1    |
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
| running        | Bool | 节点是否运行中       |

**stats:**

| Name                       | Type    | Description               |
|----------------------------|---------|---------------------------|
| connections.count          | Integer | 当前连接数量                  |
| connections.max            | Integer | 连接数量的历史最大值                |
| handshakings.count         | Integer | 当前握手的连接数量                |
| handshakings.max           | Integer | 当前握手的连接数量的历史最大值              |
| handshakings_active.count  | Integer | 当前正在执行握手操作的连接数量              |
| handshakings_rate.count    | Integer | 连接握手速率                  |
| handshakings_rate.max      | Integer | 连接握手速率的历史最大值                |
| sessions.count             | Integer | 当前会话数量                  |
| sessions.max               | Integer | 会话数量的历史最大值                |
| topics.count               | Integer | 当前主题数量                  |
| topics.max                 | Integer | 主题数量的历史最大值                |
| subscriptions.count        | Integer | 当前订阅数量，包含共享订阅            |
| subscriptions.max          | Integer | 订阅数量的历史最大值                |
| subscriptions_shared.count | Integer | 当前共享订阅数量                  |
| subscriptions_shared.max   | Integer | 共享订阅数量的历史最大值              |
| routes.count               | Integer | 当前路由数量                  |
| routes.max                 | Integer | 路由数量的历史最大值                |
| retained.count             | Integer | 当前保留消息数量                  |
| retained.max               | Integer | 保留消息的历史最大值                |
| delayed_publishs.count     | Integer | 当前延迟发布消息数量                |
| delayed_publishs.max       | Integer | 延迟发布消息数量的历史最大值            |
| forwards.count             | Integer | 当前转发消息数量                 |
| forwards.max               | Integer | 转发消息数量的历史最大值              |
| in_inflights.count         | Integer | 当前入方向飞行消息数量（待 ACK）          |
| in_inflights.max           | Integer | 入方向飞行消息数量的历史最大值           |
| out_inflights.count        | Integer | 当前出方向飞行消息数量（待 ACK）          |
| out_inflights.max          | Integer | 出方向飞行消息数量的历史最大值           |
| message_queues.count       | Integer | 当前消息队列数量                 |
| message_queues.max         | Integer | 消息队列数量的历史最大值              |
| message_storages.count     | Integer | 当前消息存储数量（-1 表示未启用存储模块）     |
| message_storages.max       | Integer | 消息存储数量的历史最大值              |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/stats"

[{"node":{"id":1,"name":"1@127.0.0.1","running":true},"stats":{"connections.count":1,"connections.max":2,"retained.count":2,"retained.max":2,"routes.count":3,"routes.max":4,"sessions.count":1,"sessions.max":2,"subscriptions.count":7,"subscriptions.max":8,"subscriptions_shared.count":1,"subscriptions_shared.max":2,"topics.count":3,"topics.max":4}}]
```

### GET /api/v1/stats/{node}

返回集群下指定节点的状态数据。

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | 节点ID，如：1    |

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
| running        | Bool | 节点是否运行中       |

**stats:**

| Name | Type | Description |
|------| --------- | ----------- |
| {}   | Json Object | 状态数据，详细请参见<br/>[GET /api/v1/stats](#get-stats)|

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/stats/1"

{"node":{"id":1,"name":"1@127.0.0.1","running":true},"stats":{"connections.count":1,"connections.max":2,"retained.count":2,"retained.max":2,"routes.count":3,"routes.max":4,"sessions.count":1,"sessions.max":2,"subscriptions.count":7,"subscriptions.max":8,"subscriptions_shared.count":1,"subscriptions_shared.max":2,"topics.count":3,"topics.max":4}}
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
| {id}        | Object   | 节点, key 为节点ID  |
| {id}.name   | String   | 节点名称           |
| {id}.running | Bool    | 节点是否运行中        |

**stats:**

| Name | Type | Description |
|------| --------- | ----------- |
| {}   | Json Object | 状态数据，详细请参见<br/>[GET /api/v1/stats](#get-stats)|

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/stats/sum"

{"nodes":{"1":{"name":"1@127.0.0.1","running":true}},"stats":{"connections.count":1,"connections.max":2,"retained.count":2,"retained.max":2,"routes.count":3,"routes.max":4,"sessions.count":1,"sessions.max":2,"subscriptions.count":7,"subscriptions.max":8,"subscriptions_shared.count":1,"subscriptions_shared.max":2,"topics.count":3,"topics.max":4}}
```

### GET /api/v1/stats/sys

返回集群下所有节点的系统状态数据。响应格式与 [GET /api/v1/stats](#get-stats) 相同，但 stats 字段使用系统级 JSON 序列化表示。

**Path Parameters:** 无

**Success Response Body (JSON):** 同 [GET /api/v1/stats](#get-stats)

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/stats/sys"
```

### GET /api/v1/stats/sys/{node}

返回集群下指定节点的系统状态数据。

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | 节点ID，如：1    |

**Success Response Body (JSON):** 同 [GET /api/v1/stats](#get-stats)

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/stats/sys/1"
```

### GET /api/v1/stats/sys/sum

汇总集群下所有节点的系统状态数据。

**Path Parameters:** 无

**Success Response Body (JSON):** 同 [GET /api/v1/stats/sum](#get-statssum)

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/stats/sys/sum"
```

### GET /api/v1/stats/history

查询集群下所有节点的状态历史数据。需要启用历史存储配置。

**Query String Parameters:**

| Name         | Type    | Required | Default | Description           |
|--------------|---------|----------|---------|-----------------------|
| minutes      | Integer | Optional | 5       | 查询最近N分钟的数据          |
| hours        | Integer | Optional |         | 查询最近N小时的数据（与 minutes/days 三选一） |
| days         | Integer | Optional |         | 查询最近N天的数据（与 minutes/hours 三选一） |
| limit        | Integer | Optional | 1000    | 最多返回的数据点数 |
| merge_window | Integer | Optional |         | 合并窗口（秒），大于0时按窗口粒度合并数据 |

**Success Response Body (JSON):**

| Name      | Type              | Description          |
|-----------|-------------------|----------------------|
| from      | Integer           | 查询起始时间戳（毫秒）       |
| to        | Integer           | 查询结束时间戳（毫秒）       |
| nodes     | Object            | 各节点的历史数据，key 为节点ID |
| nodes.{id}| Object            | 节点历史数据             |
| .from     | Integer           | 该节点数据的起始时间戳        |
| .to       | Integer           | 该节点数据的结束时间戳        |
| .node     | Integer           | 节点ID                |
| .count    | Integer           | 数据点数量              |
| .data     | Array             | 历史数据点数组，每个元素包含 `ts` (时间戳) 和 stats 字段 |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/stats/history?minutes=10&limit=100"

{"from":1700000000000,"to":1700000600000,"nodes":{"1":{"from":1700000000000,"to":1700000600000,"node":1,"count":120,"data":[{"ts":1700000000000,"connections.count":1,"sessions.count":1,...},...]}}}
```

### GET /api/v1/stats/history/{node}

查询集群下指定节点的状态历史数据。

**Path Parameters:**

| Name | Type    | Required | Description    |
|------|---------|----------|----------------|
| node | Integer | True     | 节点ID，如：1    |

**Query String Parameters:** 同 [GET /api/v1/stats/history](#get-statshistory)

**Success Response Body (JSON):**

| Name   | Type    | Description        |
|--------|---------|--------------------|
| from   | Integer | 查询起始时间戳（毫秒）      |
| to     | Integer | 查询结束时间戳（毫秒）      |
| node   | Integer | 节点ID              |
| count  | Integer | 数据点数量             |
| data   | Array   | 历史数据点数组，每个元素包含 `ts` (时间戳) 和 stats 字段 |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/stats/history/1?hours=1&limit=200"
```

### GET /api/v1/stats/history/sum

汇总集群下所有节点的状态历史数据（按时间戳求和数值字段）。

**Query String Parameters:** 同 [GET /api/v1/stats/history](#get-statshistory)

**Success Response Body (JSON):**

| Name       | Type    | Description         |
|------------|---------|---------------------|
| from       | Integer | 查询起始时间戳（毫秒）       |
| to         | Integer | 查询结束时间戳（毫秒）       |
| node_count | Integer | 参与汇总的节点数量         |
| count      | Integer | 数据点数量              |
| data       | Array   | 汇总后的数据点数组，每个元素包含 `ts` 和所有节点的汇总数值 |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/stats/history/sum?minutes=30&limit=500"
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

| Name                            | Type    | Description                      |
|---------------------------------|---------|----------------------------------|
| client.auth.anonymous           | Integer | 匿名登录的客户端数量                       |
| client.auth.anonymous.error     | Integer | 匿名登录失败的客户端数量                     |
| client.authenticate             | Integer | 客户端认证次数                          |
| client.connack                  | Integer | 发送 CONNACK 报文的次数                 |
| client.connack.auth.error       | Integer | 发送连接认证失败的 CONNACK 报文的次数          |
| client.connack.error            | Integer | 发送连接失败的 CONNACK 报文的次数            |
| client.connect                  | Integer | 客户端连接次数                          |
| client.connected                | Integer | 客户端成功连接次数                        |
| client.disconnected             | Integer | 客户端断开连接次数                        |
| client.handshaking.timeout      | Integer | 连接握手超时次数                         |
| client.publish.auth.error       | Integer | 发布，ACL 规则检查失败次数                  |
| client.publish.check.acl        | Integer | 发布，ACL 规则检查次数                    |
| client.publish.error            | Integer | 发布，失败次数                          |
| client.subscribe.auth.error     | Integer | 订阅，ACL 规则检查失败次数                  |
| client.subscribe.error          | Integer | 订阅，失败次数                          |
| client.subscribe.check.acl      | Integer | 订阅，ACL 规则检查次数                    |
| client.subscribe                | Integer | 客户端订阅次数                          |
| client.unsubscribe              | Integer | 客户端取消订阅次数                        |
| messages.publish                | Integer | 接收到PUBLISH消息数量                   |
| messages.publish.admin          | Integer | 通过HTTP-API发布的消息                   |
| messages.publish.bridge         | Integer | 通过 Bridge 桥接发布的消息                 |
| messages.publish.custom         | Integer | 通过MQTT客户端发布的消息                   |
| messages.publish.lastwill       | Integer | 遗嘱消息                              |
| messages.publish.retain         | Integer | 转发的保留消息                          |
| messages.publish.system         | Integer | 系统主题消息($SYS/#)                    |
| messages.delivered              | Integer | 向订阅端转发的消息数                       |
| messages.delivered.admin        | Integer | 通过HTTP-API发布的消息转发数                |
| messages.delivered.bridge       | Integer | 通过 Bridge 桥接发布的消息转发数              |
| messages.delivered.custom       | Integer | 通过MQTT客户端发布的消息转发数                |
| messages.delivered.lastwill     | Integer | 遗嘱消息转发数                          |
| messages.delivered.retain       | Integer | 转发的保留消息转发数                       |
| messages.delivered.system       | Integer | 系统主题消息转发数                        |
| messages.acked                  | Integer | 接收的 PUBACK 和 PUBREC 报文数量           |
| messages.acked.admin            | Integer | 通过HTTP-API发布的消息的 ACK 数量           |
| messages.acked.bridge           | Integer | 通过 Bridge 桥接发布的消息的 ACK 数量         |
| messages.acked.custom           | Integer | 通过MQTT客户端发布的消息的 ACK 数量           |
| messages.acked.lastwill         | Integer | 遗嘱消息的 ACK 数量                      |
| messages.acked.retain           | Integer | 转发的保留消息的 ACK 数量                   |
| messages.acked.system           | Integer | 系统主题消息的 ACK 数量                    |
| messages.nonsubscribed          | Integer | 未找到订阅关系的PUBLISH消息数量              |
| messages.nonsubscribed.admin    | Integer | 通过HTTP-API发布的无订阅消息数量              |
| messages.nonsubscribed.bridge   | Integer | 通过 Bridge 桥接发布的无订阅消息数量            |
| messages.nonsubscribed.custom   | Integer | 通过MQTT客户端发布的无订阅消息数量              |
| messages.nonsubscribed.lastwill | Integer | 遗嘱消息中无订阅的消息数量                    |
| messages.nonsubscribed.system   | Integer | 系统主题中无订阅的消息数量                    |
| messages.dropped                | Integer | 丢弃的消息总数                           |
| session.created                 | Integer | 创建的会话数量                           |
| session.resumed                 | Integer | 由于 `Clean Session` 或 `Clean Start` 为 `false` 而恢复的会话数量 |
| session.subscribed              | Integer | 客户端成功订阅次数                         |
| session.unsubscribed            | Integer | 客户端成功取消订阅次数                       |
| session.terminated              | Integer | 终结的会话数量                           |

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
| node | Integer    | True       | 节点ID，如：1    |

**Success Response Body (JSON):**

| Name          | Type                 | Description            |
|---------------|----------------------|------------------------|
| {}            | Object               | 统计指标信息                   |
| {}.node  | Json Object          | 节点信息                   |
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

汇总集群下所有节点的统计指标数据。

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

### GET /api/v1/metrics/history

查询集群下所有节点的指标历史数据。需要启用历史存储配置。

**Query String Parameters:**

| Name         | Type    | Required | Default | Description           |
|--------------|---------|----------|---------|-----------------------|
| minutes      | Integer | Optional | 5       | 查询最近N分钟的数据          |
| hours        | Integer | Optional |         | 查询最近N小时的数据          |
| days         | Integer | Optional |         | 查询最近N天的数据          |
| limit        | Integer | Optional | 1000    | 最多返回的数据点数          |
| merge_window | Integer | Optional |         | 合并窗口（秒）             |

**Success Response Body (JSON):**

| Name       | Type              | Description          |
|------------|-------------------|----------------------|
| from       | Integer           | 查询起始时间戳（毫秒）       |
| to         | Integer           | 查询结束时间戳（毫秒）       |
| nodes      | Object            | 各节点的历史数据，key 为节点ID |
| nodes.{id} | Object            | 节点历史数据             |
| .from      | Integer           | 该节点数据的起始时间戳        |
| .to        | Integer           | 该节点数据的结束时间戳        |
| .node      | Integer           | 节点ID                |
| .count     | Integer           | 数据点数量              |
| .data      | Array             | 历史数据点数组            |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/metrics/history?minutes=10&limit=100"
```

### GET /api/v1/metrics/history/{node}

查询集群下指定节点的指标历史数据。

**Path Parameters:**

| Name | Type    | Required | Description    |
|------|---------|----------|----------------|
| node | Integer | True     | 节点ID，如：1    |

**Query String Parameters:** 同 [GET /api/v1/metrics/history](#get-metricshistory)

**Success Response Body (JSON):**

| Name   | Type    | Description        |
|--------|---------|--------------------|
| from   | Integer | 查询起始时间戳（毫秒）      |
| to     | Integer | 查询结束时间戳（毫秒）      |
| node   | Integer | 节点ID              |
| count  | Integer | 数据点数量             |
| data   | Array   | 历史数据点数组，每个元素包含 `ts` (时间戳) 和 metrics 字段 |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/metrics/history/1?minutes=30"
```

### GET /api/v1/metrics/history/sum

汇总集群下所有节点的指标历史数据。

**Query String Parameters:** 同 [GET /api/v1/metrics/history](#get-metricshistory)

**Success Response Body (JSON):**

| Name       | Type    | Description         |
|------------|---------|---------------------|
| from       | Integer | 查询起始时间戳（毫秒）       |
| to         | Integer | 查询结束时间戳（毫秒）       |
| node_count | Integer | 参与汇总的节点数量         |
| count      | Integer | 数据点数量              |
| data       | Array   | 汇总后的数据点数组         |

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/metrics/history/sum?minutes=60"
```

### GET /api/v1/metrics/prometheus

<span id = "get-prometheus" />

以 *prometheus* 格式返回集群中所有节点的状态数据和统计指标数据。

**Path Parameters:** 无

**Success Response Body (TEXT):**

**Examples:**

```bash
$ curl -i -X GET "http://localhost:6060/api/v1/metrics/prometheus"

# HELP rmqtt_metrics All metrics data
# TYPE rmqtt_metrics gauge
rmqtt_metrics{item="client.auth.anonymous",node="1"} 0
rmqtt_metrics{item="client.auth.anonymous",node="2"} 2
...
# HELP rmqtt_nodes All nodes status
# TYPE rmqtt_nodes gauge
rmqtt_nodes{item="disk_free",node="1"} 46307106816
...
# HELP rmqtt_stats All status data
# TYPE rmqtt_stats gauge
rmqtt_stats{item="connections.count",node="1"} 1
...
```

### GET /api/v1/metrics/prometheus/{node}

以 *prometheus* 格式返回集群中指定节点的状态数据和统计指标数据。

**Path Parameters:**

| Name | Type | Required | Description |
| ---- | --------- | ------------|-------------|
| node | Integer    | True       | 节点ID，如：1    |

**Success Response Body (TEXT):**

详见 [GET /api/v1/metrics/prometheus](#get-prometheus) 

### GET /api/v1/metrics/prometheus/sum

以 *prometheus* 格式返回集群中所有节点汇总的状态数据和统计指标数据。

**Path Parameters:** 无

**Success Response Body (TEXT):**

详见 [GET /api/v1/metrics/prometheus](#get-prometheus) 
