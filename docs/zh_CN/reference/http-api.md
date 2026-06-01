[English](../en_US/reference/http-api.md) | [**简体中文**](http-api.md)

# HTTP API 参考

RMQTT HTTP API 提供 Broker 管理的 RESTful 端点，涵盖监控、客户端管理、订阅、消息和插件控制。

## 配置

由 `rmqtt-http-api` 插件提供。

```toml
# rmqtt-http-api.toml
http_laddr = "0.0.0.0:6060"
max_row_limit = 10_000
http_request_log = false
message_expiry_interval = "5m"
prometheus_metrics_cache_interval = "5s"
# http_bearer_token = "your-secret-token"
```

## 端点速查

| 方法 | 路径 | 说明 |
|--------|------|------|
| `GET` | `/api/v1` | API 列表 |
| `GET` | `/api/v1/brokers` | 集群节点信息 |
| `GET` | `/api/v1/health/check` | 健康检查 |
| `GET` | `/api/v1/clients` | 搜索客户端 |
| `DELETE` | `/api/v1/clients/{clientid}` | 踢出客户端 |
| `GET` | `/api/v1/subscriptions` | 订阅列表 |
| `GET` | `/api/v1/routes` | 路由表 |
| `POST` | `/api/v1/mqtt/publish` | 发布消息 |
| `POST` | `/api/v1/mqtt/subscribe` | 订阅主题 |
| `POST` | `/api/v1/mqtt/unsubscribe` | 取消订阅 |
| `GET` | `/api/v1/plugins` | 插件列表 |
| `PUT` | `/api/v1/plugins/{node}/{plugin}/config/reload` | 重载插件配置 |
| `GET` | `/api/v1/stats` | 统计信息 |
| `GET` | `/api/v1/metrics` | 指标（JSON） |
| `GET` | `/api/v1/metrics/prometheus` | 指标（Prometheus 格式） |

完整端点列表共 36 个。详情见英文版文档。

## 许可证

MIT OR Apache-2.0
