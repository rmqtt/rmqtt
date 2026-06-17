[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-topic-rewrite

[![crates.io](https://img.shields.io/crates/v/rmqtt-topic-rewrite.svg)](https://crates.io/crates/rmqtt-topic-rewrite)

RMQTT 的主题重写插件。使用可配置规则重写/映射 MQTT 主题过滤器和主题名称。

## 概述

应用可配置的重写规则，在发布时转换主题名称，在订阅/取消订阅时转换主题过滤器。这允许在不更改客户端行为的情况下进行内部主题命名空间重新映射。

### 使用场景

- **命名空间迁移**：将旧的主题结构映射到新的结构，无需更新客户端
- **多租户隔离**：为主题添加租户/客户端标识符前缀
- **协议桥接**：在不同主题命名约定之间进行转换
- **访问控制**：在敏感主题到达某些客户端之前重写它们

### 规则工作原理

每条规则指定：

- `action`：何时应用规则（`publish`、`subscribe` 或 `all`）
- `source_topic_filter`：要匹配的原始主题过滤器
- `dest_topic`：重写后的目标主题（支持 `$N` 捕获组和 `${clientid}`/`${username}` 占位符）
- `regex`：可选的正则表达式，用于从源主题中提取捕获组

## 使用

在 `Cargo.toml` 中添加依赖：

```toml
rmqtt-topic-rewrite = "0.21"
```

在代理启动代码中注册插件：

```rust
rmqtt_topic_rewrite::register(&scx, true, false).await?;
```

## 配置

配置文件：`rmqtt-topic-rewrite.toml`

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `rules` | Table 数组 | `[]` | 主题重写规则列表 |

### 规则字段

| 字段 | 类型 | 描述 |
|------|------|------|
| `action` | String | 应用范围：`"publish"`、`"subscribe"` 或 `"all"` |
| `source_topic_filter` | String | 匹配原始主题的主题过滤器 |
| `dest_topic` | String | 重写后的目标主题。支持 `$1`、`$2`... 捕获组和 `${clientid}`、`${username}` |
| `regex` | String (可选) | 从源主题提取捕获组的正则表达式 |

### 变量占位符

| 占位符 | 描述 |
|--------|------|
| `${clientid}` | 替换为客户端的客户端 ID |
| `${username}` | 替换为客户端的用户名 |
| `$1`、`$2`、... | 正则表达式匹配的捕获组 |

### 默认规则（全部注释）

默认配置包含注释的示例，展示了各种重写模式：

```toml
rules = [
    # 所有操作：通过正则表达式提取段并重新映射
    # { action = "all", source_topic_filter = "x/+/#", dest_topic = "xx/$1/$2", regex = "^x/(.+)/(.+)$" },

    # 直接主题映射（无正则表达式）
    # { action = "all", source_topic_filter = "x/y/1", dest_topic = "xx/y/1" },

    # 使用正则表达式捕获的通配符订阅重写
    # { action = "all", source_topic_filter = "x/y/#", dest_topic = "xx/y/$1", regex = "^x/y/(.+)$" },

    # 客户端 ID 替换
    # { action = "all", source_topic_filter = "iot/cid/#", dest_topic = "iot/${clientid}/$1", regex = "^iot/cid/(.+)$" },

    # 用户名替换
    # { action = "all", source_topic_filter = "iot/uname/#", dest_topic = "iot/${username}/$1", regex = "^iot/uname/(.+)$" },

    # 多段重新映射
    # { action = "all", source_topic_filter = "a/+/+/+", dest_topic = "aa/$1/$2/$3", regex = "^a/(.+)/(.+)/(.+)$" }
]
```

### 示例：生效的规则

```toml
rules = [
    # 重写发布主题："sensor/+/temp" → "telemetry/$1/temp"
    { action = "publish", source_topic_filter = "sensor/+/temp", dest_topic = "telemetry/$1/temp", regex = "^sensor/(.+)/temp$" },

    # 重写订阅主题："device/#" → "devices/${clientid}/#"
    { action = "subscribe", source_topic_filter = "device/#", dest_topic = "devices/${clientid}/$1", regex = "^device/(.+)$" },

    # 所有操作的直接映射
    { action = "all", source_topic_filter = "old/metrics/#", dest_topic = "new/metrics/$1", regex = "^old/metrics/(.+)$" }
]
```

## 依赖

- `rmqtt`（feature `plugin`）

## 许可证

MIT OR Apache-2.0
