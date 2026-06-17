[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-http-api

[![crates.io](https://img.shields.io/crates/v/rmqtt-http-api.svg)](https://crates.io/crates/rmqtt-http-api)

RESTful HTTP API 插件。提供 Broker 管理、健康检查、节点信息、客户端/订阅/路由查询、MQTT 操作、插件管理、统计信息和 Prometheus 指标等端点。

## 概述

使用 `salvo` HTTP 框架提供 API 服务端。HTTP 服务器在插件构造（`new()`）时启动。支持热重载：当配置变化需要重启时（监听地址变更），先启动新服务再关闭旧服务。通过 gRPC 转发集群范围内的查询。

## 使用方法

### 构建

在 `rmqttd/Cargo.toml` 中添加依赖：

```toml
rmqtt-http-api = "0.21"
```

需要 `rmqtt` 的 features：`plugin`、`metrics`、`stats`、`grpc`、`shared-subscription`。

### 注册

```rust
rmqtt_http_api::register(&scx, true, false).await?;
// 或指定名称：
rmqtt_http_api::register_named(&scx, "rmqtt-http-api", true, false).await?;
```

参数说明：`(scx, default_startup, immutable)`。

## 配置

配置文件：`rmqtt-http-api.toml`（位于插件配置目录）。通过 `scx.plugins.read_config_default::<PluginConfig>("rmqtt-http-api")` 加载。

| 选项 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `max_row_limit` | `usize` | `10000` | 列表端点返回的最大行数 |
| `http_laddr` | `string` | `"0.0.0.0:6060"` | HTTP 服务器监听地址 |
| `http_request_log` | `bool` | `false` | 是否打印 HTTP 请求日志 |
| `http_reuseaddr` | `bool` | `true` | 启用 `SO_REUSEADDR` socket 选项（仅 Unix） |
| `http_reuseport` | `bool` | `false` | 启用 `SO_REUSEPORT` socket 选项（仅 Unix） |
| `http_bearer_token` | `string` | — | HTTP API Bearer 令牌认证（可选） |
| `message_type` | `u8` | `99` | 插件间 gRPC 通信的消息类型标识符 |
| `message_expiry_interval` | `string` | `"5m"` | 发布操作消息的默认过期时间 |
| `metrics_sample_interval` | `string` | `"5s"` | 指标采样间隔 |
| `prometheus_metrics_cache_interval` | `string` | `"5s"` | Prometheus 指标数据缓存间隔 |

### 配置来源

支持标准 RMQTT 插件配置链：

1. `{plugins.dir}/rmqtt-http-api.toml`（文件，可选——文件缺失时使用默认值）
2. `rmqtt_plugin_rmqtt_http_api_*` 环境变量
3. 通过 `ServerContext::plugins_config_map_add()` 内联配置

### 示例

```toml
# rmqtt-http-api.toml
max_row_limit = 10_000
http_laddr = "0.0.0.0:6060"
http_request_log = false
message_expiry_interval = "5m"
prometheus_metrics_cache_interval = "5s"
```

## API 端点

所有端点都以 `/api/v1` 为前缀。

| 方法 | 路径 | 说明 |
|------|------|------|
| GET | `/` | 列出所有可用 API 端点 |
| **Broker** | | |
| GET | `/brokers` | 返回集群中所有节点的基本信息 |
| GET | `/brokers/{id}` | 返回指定节点的基本信息 |
| **节点** | | |
| GET | `/nodes` | 返回集群中所有节点的状态 |
| GET | `/nodes/{id}` | 返回指定节点的状态 |
| **健康检查** | | |
| GET | `/health/check` | 集群健康检查 |
| GET | `/health/check/{id}` | 指定节点健康检查 |
| **客户端** | | |
| GET | `/clients` | 从集群中搜索客户端信息 |
| GET | `/clients/{clientid}` | 获取指定客户端信息 |
| DELETE | `/clients/{clientid}` | 从集群中踢出客户端 |
| GET | `/clients/{clientid}/online` | 检查客户端是否在线 |
| GET | `/clients/offlines` | 搜索离线客户端信息 |
| DELETE | `/clients/offlines` | 从集群中踢出离线客户端 |
| **订阅** | | |
| GET | `/subscriptions` | 从集群中查询订阅信息 |
| GET | `/subscriptions/{clientid}` | 获取指定客户端的订阅 |
| **路由** | | |
| GET | `/routes` | 返回集群中所有路由信息 |
| GET | `/routes/{topic}` | 获取指定主题的路由信息 |
| **MQTT 操作** | | |
| POST | `/mqtt/publish` | 发布 MQTT 消息 |
| POST | `/mqtt/subscribe` | 为会话订阅 MQTT 主题 |
| POST | `/mqtt/unsubscribe` | 取消订阅 MQTT 主题 |
| **插件** | | |
| GET | `/plugins` | 返回集群中所有插件的信息 |
| GET | `/plugins/{node}` | 返回指定节点的插件信息 |
| GET | `/plugins/{node}/{plugin}` | 获取指定插件的详细信息 |
| GET | `/plugins/{node}/{plugin}/config` | 获取插件的配置 |
| PUT | `/plugins/{node}/{plugin}/config/reload` | 重新加载插件配置 |
| PUT | `/plugins/{node}/{plugin}/load` | 在节点上加载/启动插件 |
| PUT | `/plugins/{node}/{plugin}/unload` | 在节点上卸载/停止插件 |
| **统计** | | |
| GET | `/stats` | 返回集群中所有统计信息 |
| GET | `/stats/sum` | 汇总集群中所有统计信息 |
| GET | `/stats/{id}` | 返回指定节点的统计信息 |
| GET | `/stats/sys` | 返回集群中所有系统统计信息 |
| GET | `/stats/sys/sum` | 汇总集群中所有系统统计信息 |
| GET | `/stats/sys/{id}` | 返回指定节点的系统统计信息 |
| **指标** | | |
| GET | `/metrics` | 返回集群中所有指标 |
| GET | `/metrics/sum` | 汇总集群中所有指标 |
| GET | `/metrics/{id}` | 返回指定节点的指标 |
| GET | `/metrics/prometheus` | 获取集群的 Prometheus 指标 |
| GET | `/metrics/prometheus/sum` | 汇总 Prometheus 指标 |
| GET | `/metrics/prometheus/{id}` | 获取指定节点的 Prometheus 指标 |

### 认证

如果配置了 `http_bearer_token`，所有 API 请求（健康检查除外）需要携带 `Authorization: Bearer <token>` 请求头。

## 依赖

`rmqtt`（features: `plugin`、`metrics`、`stats`、`grpc`、`shared-subscription`）、`salvo`、`tokio`、`serde`、`serde_json`、`anyhow`、`base64`、`futures`

## 许可证

MIT OR Apache-2.0
