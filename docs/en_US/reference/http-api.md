[**English**](http-api.md) | [简体中文](../../zh_CN/reference/http-api.md)

# HTTP API Reference

The RMQTT HTTP API provides RESTful management endpoints for broker monitoring, client management, subscriptions, messaging, and plugin control.

## Configuration

The HTTP API is provided by the `rmqtt-http-api` plugin.

```toml
# rmqtt-http-api.toml
# Listen address
http_laddr = "0.0.0.0:6060"

# Maximum number of rows returned in list queries
max_row_limit = 10_000

# Log HTTP requests
http_request_log = false

# Message expiry interval
message_expiry_interval = "5m"

# Prometheus metrics cache interval
prometheus_metrics_cache_interval = "5s"

# Optional Bearer token authentication
# http_bearer_token = "your-secret-token"
```

### Authentication (Optional)

If `http_bearer_token` is set, all requests must include:

```
Authorization: Bearer <your-secret-token>
```

## Base URL

All endpoints are under `/api/v1/`.

---

## 1. API Introspection

List all available endpoints.

```
GET /api/v1
```

---

## 2. Broker & Node Info

### List Brokers

```
GET /api/v1/brokers
GET /api/v1/brokers/{id}
```

Returns cluster node information.

### List Nodes

```
GET /api/v1/nodes
GET /api/v1/nodes/{id}
```

Returns node status.

### Health Check

```
GET /api/v1/health/check
GET /api/v1/health/check/{id}
```

Returns `{"code": 0, "msg": "ok"}` if healthy.

---

## 3. Client Management

### Search Clients

```
GET /api/v1/clients
```

Query parameters:

| Parameter | Type | Description |
|-----------|------|-------------|
| `_limit` | `u64` | Max results |
| `clientid` | `string` | Exact client ID |
| `username` | `string` | Exact username |
| `ip_address` | `string` | Client IP |
| `connected` | `bool` | Connection status |
| `clean_start` | `bool` | Clean session flag |
| `session_present` | `bool` | Session present |
| `proto_ver` | `u8` | Protocol version (3, 4, 5) |
| `_like_clientid` | `string` | Client ID pattern match |
| `_like_username` | `string` | Username pattern match |
| `_gte_created_at` | `i64` | Created after (timestamp) |
| `_lte_created_at` | `i64` | Created before (timestamp) |
| `_gte_connected_at` | `i64` | Connected after (timestamp) |
| `_lte_connected_at` | `i64` | Connected before (timestamp) |
| `_gte_mqueue_len` | `usize` | Message queue length >= |
| `_lte_mqueue_len` | `usize` | Message queue length <= |

### Get Client

```
GET /api/v1/clients/{clientid}
```

### Kick Client (Disconnect)

```
DELETE /api/v1/clients/{clientid}
```

Kicks a connected client from the broker.

### Check Online Status

```
GET /api/v1/clients/{clientid}/online
```

### Search Offline Clients

```
GET /api/v1/clients/offlines
```

### Kick All Offline Clients

```
DELETE /api/v1/clients/offlines
```

Returns the count of kicked clients.

---

## 4. Subscriptions

### Query Subscriptions

```
GET /api/v1/subscriptions
```

Lists all active subscriptions across the cluster.

### Get Client Subscriptions

```
GET /api/v1/subscriptions/{clientid}
```

---

## 5. Routes

### List Routes

```
GET /api/v1/routes
```

### Get Route for Topic

```
GET /api/v1/routes/{topic}
```

---

## 6. MQTT Operations

### Publish Message

```
POST /api/v1/mqtt/publish
```

Request body (JSON):

```json
{
  "topic": "test/topic",
  "payload": "hello",
  "qos": 1,
  "retain": false,
  "encoding": "plain",
  "properties": {
    "user_properties": { "key": "value" }
  }
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `topic` | `string` | (required) | MQTT topic |
| `payload` | `string` | `""` | Message payload |
| `qos` | `u8` | `0` | QoS level (0, 1, 2) |
| `retain` | `bool` | `false` | Retain message |
| `encoding` | `string` | `"plain"` | `"plain"` or `"base64"` |
| `properties` | `object` | `null` | MQTT v5 properties |

### Subscribe

```
POST /api/v1/mqtt/subscribe
```

```json
{
  "clientid": "client1",
  "topic": "test/#",
  "qos": 1
}
```

### Unsubscribe

```
POST /api/v1/mqtt/unsubscribe
```

```json
{
  "clientid": "client1",
  "topic": "test/#"
}
```

---

## 7. Plugin Management

### List All Plugins

```
GET /api/v1/plugins
```

Returns all plugins across all cluster nodes.

### List Node Plugins

```
GET /api/v1/plugins/{node}
```

### Get Plugin Info

```
GET /api/v1/plugins/{node}/{plugin}
```

### Get Plugin Config

```
GET /api/v1/plugins/{node}/{plugin}/config
```

### Reload Plugin Config

```
PUT /api/v1/plugins/{node}/{plugin}/config/reload
```

### Load Plugin

```
PUT /api/v1/plugins/{node}/{plugin}/load
```

### Unload Plugin

```
PUT /api/v1/plugins/{node}/{plugin}/unload
```

---

## 8. Statistics

### Node Stats

```
GET /api/v1/stats
GET /api/v1/stats/{id}
GET /api/v1/stats/sum
```

### System Stats

```
GET /api/v1/stats/sys
GET /api/v1/stats/sys/{id}
GET /api/v1/stats/sys/sum
```

---

## 9. Metrics

### JSON Metrics

```
GET /api/v1/metrics
GET /api/v1/metrics/{id}
GET /api/v1/metrics/sum
```

### Prometheus Metrics

```
GET /api/v1/metrics/prometheus
GET /api/v1/metrics/prometheus/{id}
GET /api/v1/metrics/prometheus/sum
```

Returns metrics in Prometheus text format (`text/plain`).

---

## License

MIT OR Apache-2.0
