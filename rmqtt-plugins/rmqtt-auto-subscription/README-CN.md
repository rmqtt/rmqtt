[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-auto-subscription

[![crates.io](https://img.shields.io/crates/v/rmqtt-auto-subscription.svg)](https://crates.io/crates/rmqtt-auto-subscription)

RMQTT 的自动订阅插件。在客户端连接时自动将其订阅到预定义的主题过滤器。

## 概述

当客户端连接时，插件自动将其订阅到配置的主题过滤器，无需客户端发送 SUBSCRIBE 数据包。适用于：

- 强制客户端接收特定的系统或遥测主题
- 确保所有客户端具有基准订阅集
- 通过在代理上预配置订阅来减少客户端端逻辑

每个订阅条目支持可配置的 QoS、No Local、Retain as Published 和 Retain Handling 选项（MQTTv5 属性）。

### 变量插值

主题过滤器表达式可以使用 `${clientid}` 表示客户端 ID，使用 `${username}` 表示客户端用户名，实现每个客户端的动态订阅主题。

## 使用

在 `Cargo.toml` 中添加依赖：

```toml
rmqtt-auto-subscription = "0.21"
```

在代理启动代码中注册插件：

```rust
rmqtt_auto_subscription::register(&scx, true, false).await?;
```

## 配置

配置文件：`rmqtt-auto-subscription.toml`

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `subscribes` | Table 数组 | *(见下方)* | 客户端连接时要应用的订阅列表 |

### 订阅条目字段

`subscribes` 数组中的每个条目支持：

| 字段 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `topic_filter` | String | *(必填)* | 要订阅的主题过滤器。支持 `${clientid}` 和 `${username}` 占位符 |
| `qos` | Integer | `1` | QoS 级别：`0`、`1` 或 `2` |
| `no_local` | Boolean | `false` | MQTTv5：不转发客户端自己发布的消息 |
| `retain_as_published` | Boolean | `false` | MQTTv5：转发时保留 RETAIN 标志 |
| `retain_handling` | Integer | `0` | MQTTv5：保留消息处理策略（`0`、`1` 或 `2`） |

### 默认订阅

```toml
subscribes = [
    { topic_filter = "x/+/#", qos = 1, no_local = false, retain_as_published = false, retain_handling = 0 },
    { topic_filter = "foo/${clientid}/#", qos = 1, no_local = false, retain_as_published = false, retain_handling = 0 },
    { topic_filter = "iot/${username}/#", qos = 1 }
]
```

### 占位符变量

| 占位符 | 描述 |
|--------|------|
| `${clientid}` | 替换为连接客户端的客户端 ID |
| `${username}` | 替换为连接客户端的用户名 |

### 示例：设备特定订阅

```toml
subscribes = [
    # 所有客户端订阅公共遥测主题
    { topic_filter = "telemetry/#", qos = 1 },

    # 每个客户端订阅自己的设备特定控制主题
    { topic_filter = "control/${clientid}/#", qos = 2 },

    # 订阅用户特定的配置主题
    { topic_filter = "config/${username}/#", qos = 1 }
]
```

## 依赖

- `rmqtt`（feature `plugin`）

## 许可证

MIT OR Apache-2.0
