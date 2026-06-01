[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-counter

[![crates.io](https://img.shields.io/crates/v/rmqtt-counter.svg)](https://crates.io/crates/rmqtt-counter)

指标计数器插件。通过 15 个 Hook 回调以 `Priority::MAX` 优先级追踪 MQTT 事件。

## 追踪的事件

**客户端**：`ClientConnect`、`ClientAuthenticate`、`ClientConnack`、`ClientConnected`、`ClientDisconnected`
**会话**：`SessionCreated`、`SessionTerminated`、`SessionSubscribed`、`SessionUnsubscribed`
**订阅**：`ClientSubscribe`、`ClientUnsubscribe`、`ClientSubscribeCheckAcl`、`MessagePublishCheckAcl`
**消息**：`MessagePublish`、`MessageDelivered`、`MessageAcked`、`MessageDropped`、`MessageNonsubscribed`

消息按 QoS 级别（0、1、2）和来源类型（Custom、Admin、System、LastWill、Bridge）进一步细分。

## 使用方法

### 构建

需要 `rmqtt` 的 features：`plugin`、`metrics`。

```toml
rmqtt-counter = "0.22"
```

### 注册

```rust
rmqtt_counter::register(&scx, true, false).await?;
// 或指定名称：
rmqtt_counter::register_named(&scx, "rmqtt-counter", true, false).await?;
```

参数说明：`(scx, default_startup, immutable)`。

此插件没有配置文件（无 `cfg` 字段）。所有计数器通过 `scx.metrics.*` 访问。

## 依赖

`rmqtt`（features: `plugin`、`metrics`）

## 许可证

MIT OR Apache-2.0
