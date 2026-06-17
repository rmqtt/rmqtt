[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-web-hook

[![crates.io](https://img.shields.io/crates/v/rmqtt-web-hook.svg)](https://crates.io/crates/rmqtt-web-hook)

Webhook 插件。将 MQTT 事件以 HTTP POST 通知发送到外部系统。支持 15 种钩子事件类型。

## 概述

采用异步生产者-消费者架构。处理器将事件序列化为 JSON，通过 mpsc 通道发送。后台任务消费并分发到配置的 URL。支持 HTTP POST 和本地文件输出。HTTP 失败时使用指数退避重试。

### 事件

**会话**: session_created、session_terminated、session_subscribed、session_unsubscribed

**客户端**: client_connect、client_connack、client_connected、client_disconnected、client_subscribe、client_unsubscribe

**消息**: message_publish、message_delivered、message_acked、message_dropped、offline_message

## 使用

```toml
[dependencies]
rmqtt-web-hook = "0.21"
```

```rust
rmqtt_web_hook::register(&scx, true, false).await?;
```

## 配置

文件：`rmqtt-web-hook.toml`

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `queue_capacity` | integer | `300_000` | 最大排队任务数 |
| `concurrency_limit` | integer | `128` | 最大并发执行任务数 |
| `urls` | array of string | `["file:///var/log/rmqtt/hook.log"]` | HTTP POST 或文件输出 URL |
| `http_timeout` | string | `"8s"` | HTTP 请求超时 |
| `retry_max_elapsed_time` | string | `"60s"` | 最大重试耗时 |
| `retry_multiplier` | float | `2.5` | 指数退避乘数 |
| `rule.session_created` | array of rule | `[{action = "session_created"}]` | 会话创建钩子规则 |
| `rule.session_terminated` | array of rule | `[{action = "session_terminated"}]` | 会话终止钩子规则 |
| `rule.session_subscribed` | array of rule | `[{action = "session_subscribed"}]` | 会话订阅钩子规则 |
| `rule.session_unsubscribed` | array of rule | `[{action = "session_unsubscribed"}]` | 会话取消订阅钩子规则 |
| `rule.client_connect` | array of rule | `[{action = "client_connect"}]` | 客户端连接钩子规则 |
| `rule.client_connack` | array of rule | `[{action = "client_connack"}]` | 客户端连接确认钩子规则 |
| `rule.client_connected` | array of rule | `[{action = "client_connected"}]` | 客户端已连接钩子规则 |
| `rule.client_disconnected` | array of rule | `[{action = "client_disconnected"}]` | 客户端断开连接钩子规则 |
| `rule.client_subscribe` | array of rule | `[{action = "client_subscribe"}]` | 客户端订阅钩子规则 |
| `rule.client_unsubscribe` | array of rule | `[{action = "client_unsubscribe"}]` | 客户端取消订阅钩子规则 |
| `rule.message_publish` | array of rule | `[{action = "message_publish", topics = ["#", "$SYS/#"]}]` | 消息发布钩子规则 |
| `rule.message_delivered` | array of rule | `[{action = "message_delivered", topics = ["#", "$SYS/#"]}]` | 消息投递钩子规则 |
| `rule.message_acked` | array of rule | `[{action = "message_acked", topics = ["#", "$SYS/#"]}]` | 消息确认钩子规则 |
| `rule.message_dropped` | array of rule | `[{action = "message_dropped"}]` | 消息丢弃钩子规则 |
| `rule.offline_message` | array of rule | `[{action = "offline_message", topics = ["#", "$SYS/#"]}]` | 离线消息钩子规则 |

每个 `rule.*` 是规则对象数组。每个规则支持：

| 规则字段 | 类型 | 描述 |
|----------|------|------|
| `action` | string | 匹配事件类型的动作名称 |
| `topics` | array of string | （可选）主题过滤，仅匹配的主题触发 |

示例：

```toml
rule.session_created = [{action = "session_created"}]
rule.message_publish = [{action = "message_publish", topics = ["#", "$SYS/#"]}]
```

## 依赖

`rmqtt`（feature `plugin`）、`reqwest`（rustls-tls、json）

## 许可证

MIT OR Apache-2.0
