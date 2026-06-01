[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-sys-topic

[![crates.io](https://img.shields.io/crates/v/rmqtt-sys-topic.svg)](https://crates.io/crates/rmqtt-sys-topic)

RMQTT 的系统主题插件。以 `$SYS/` MQTT 主题发布代理指标，用于监控。

## 概述

定期向 `$SYS/` 主题树发布代理统计信息，客户端可订阅以进行实时监控。无需外部监控工具即可了解代理健康状态和运行指标。

### 可用的指标

插件在 `$SYS/` 层级下发布各种代理指标，包括：

- **客户端统计**：连接的客户端数量、断开的会话数量
- **消息统计**：发布、接收和投递的消息计数
- **代理运行时间**：代理启动以来的运行时长
- **版本信息**：RMQTT 版本和构建信息

## 使用

在 `Cargo.toml` 中添加依赖：

```toml
rmqtt-sys-topic = "0.22"
```

在代理启动代码中注册插件：

```rust
rmqtt_sys_topic::register(&scx, true, false).await?;
```

## 配置

配置文件：`rmqtt-sys-topic.toml`

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `publish_qos` | Integer | `1` | `$SYS` 消息的 QoS 级别：`0`、`1` 或 `2` |
| `publish_interval` | String | `"1m"` | `$SYS` 指标发布间隔（例如 `"30s"`、`"1m"`、`"5m"`） |
| `message_expiry_interval` | String | `"5m"` | `$SYS` 消息过期时间（`0` 表示永不过期） |

### 默认配置

```toml
publish_qos = 1
publish_interval = "1m"
message_expiry_interval = "5m"
```

## 依赖

- `rmqtt`（feature `plugin`）

## 许可证

MIT OR Apache-2.0
