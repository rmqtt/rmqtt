[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-bridge-origin

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-origin.svg)](https://crates.io/crates/rmqtt-bridge-origin)

桥接来源识别插件。通过检查客户端 ID 中的标记子串来识别客户端是否来自桥接连接。

## 概述

检查客户端 ID 中是否包含配置的标记子串，将客户端分类为入站桥接、出站桥接或普通客户端。识别的桥接来源存储在 `session.extra_attrs` 的可配置键中，供其他插件使用（如防循环或路由决策）。

## 使用方式

```rust
rmqtt_bridge_origin::register(&scx, true, false).await?;
```

## 配置

文件：`rmqtt-bridge-origin.toml`

| 选项 | 类型 | 默认值 | 说明 |
|--------|------|---------|------|
| `ingress_marker` | `string` | `":ingress:"` | 识别入站桥接客户端的子串 |
| `egress_marker` | `string` | `":egress:"` | 识别出站桥接客户端的子串 |
| `attr_key` | `string` | `"bridge_origin"` | 在 `session.extra_attrs` 中存储来源的键名 |

## 依赖

`rmqtt`（feature `plugin`）

## 许可证

MIT OR Apache-2.0
