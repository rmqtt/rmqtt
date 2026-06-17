[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-p2p-messaging

[![crates.io](https://img.shields.io/crates/v/rmqtt-p2p-messaging.svg)](https://crates.io/crates/rmqtt-p2p-messaging)

RMQTT 的点对点消息插件。支持通过 MQTT 主题实现直接的客户端间消息投递。

## 概述

提供点对点消息机制，一个客户端可以使用包含目标客户端 ID 的主题模式直接向另一个特定客户端发送消息。消息仅投递给预期的接收方客户端，而非广播给所有订阅者。

### 主题格式

插件支持三种 P2P 主题格式模式：

| 模式 | 主题模式 | 描述 |
|------|----------|------|
| `prefix` | `p2p/{clientid}/{topic}` | `p2p` 标识符在主题开头 |
| `suffix` | `{topic}/p2p/{clientid}` | `p2p` 标识符在主题末尾 |
| `both` | 两种模式 | 同时支持前缀和后缀格式 |

当客户端发布到 P2P 主题（例如 `p2p/target-client-id/sensors/temp`）时，消息仅路由到 `target-client-id` 标识的客户端。

## 使用

在 `Cargo.toml` 中添加依赖：

```toml
rmqtt-p2p-messaging = "0.21"
```

在代理启动代码中注册插件：

```rust
rmqtt_p2p_messaging::register(&scx, true, false).await?;
```

## 配置

配置文件：`rmqtt-p2p-messaging.toml`

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `mode` | String | `"prefix"` | P2P 主题格式：`"prefix"`、`"suffix"` 或 `"both"` |

### 默认配置

```toml
mode = "prefix"
```

### 示例：使用前缀模式发布

客户端 A 想发送消息给客户端 B：

- **发布主题**：`p2p/Client-B/sensors/temp`
- **内容**：`{ "temperature": 23.5 }`
- **结果**：只有客户端 B 在 `sensors/temp` 上收到消息

### 示例：使用后缀模式发布

```toml
mode = "suffix"
```

- **发布主题**：`sensors/temp/p2p/Client-B`
- **内容**：`{ "temperature": 23.5 }`
- **结果**：只有客户端 B 收到消息

### 示例：同时使用两种模式

```toml
mode = "both"
```

前缀和后缀主题格式都有效：

- `p2p/Client-B/sensors/temp`
- `sensors/temp/p2p/Client-B`

## 依赖

- `rmqtt`（feature `plugin`）

## 许可证

MIT OR Apache-2.0
