[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-delayed

[![crates.io](https://img.shields.io/crates/v/rmqtt-delayed.svg)](https://crates.io/crates/rmqtt-delayed)

延迟消息发布插件。通过 `$delayed/<interval>/<topic>` 主题前缀实现 MQTT 延迟投递，底层为内存优先级队列。

## 概述

在会话发布路径上拦截主题以 `$delayed/<interval>/` 开头的 PUBLISH 包：

- **主题解析**：剥离 `$delayed/<interval>/` 前缀并提取触发间隔（整数，单位秒），消息随后被调度到真实目标主题。若间隔不是整数，该 PUBLISH 将报错拒绝。
- **每节点优先级队列**：待发消息保存在按触发时间排序的 `BinaryHeap` 中（内存态、节点本地）。
- **后台转发**：后台任务每 500 ms 轮询队列，将到期消息经普通路由路径转发。sender 被丢弃（插件停止/卸载）后，任务在一个 tick 内退出，并按 `publish_immediate` 对待发消息做终结处理（见下文「启用语义」）。
- **容量控制**：待发延迟消息数量由 `publish_max` 限制。溢出决策由插件自身完成——`publish_immediate` 选择立即转发或丢弃。

### 启用语义

加载插件即启用延迟投递特性（`/api/v1/features` 报告 `delayed: true`）；卸载后恢复核心占位实现。卸载时待发消息按 `publish_immediate` 做终结处理：`true` 时**立即转发**，`false` 时经 `message_dropped` hook（reason 为 `DelayedPublishRefused`）**丢弃**。消息仅存于内存，节点重启即丢失。

## 使用方法

### 启用

在 `rmqtt.toml` 的 `plugins.default_startups` 列表中取消注释 `rmqtt-delayed`：

```toml
plugins.default_startups = [
    #"rmqtt-acl",
    #"rmqtt-auth-http",
    #"rmqtt-cluster-broadcast",
    #"rmqtt-cluster-raft",
    "rmqtt-shared-subscription",
    "rmqtt-delayed",        # <- 启用
    "rmqtt-http-api"
]
```

或在 `rmqttd/Cargo.toml` 中添加依赖，或通过 `rmqtt-plugins` 元 crate 启用：

```toml
# 直接依赖
rmqtt-delayed = { version = "0.24" }

# 或通过元 crate
rmqtt-plugins = { version = "0.24", features = ["delayed"] }
```

### 注册

```rust
rmqtt_delayed::register(&scx, true, false).await?;
```

参数说明：`(scx, default_startup, immutable)`。经 `rmqttd` 集成时，是否随启动注册由 `rmqtt-bin/Cargo.toml` 的 `package.metadata.plugins` 条目以及 `rmqtt.toml` 的 `plugins.default_startups` / `plugins.disabled_default_startups` 控制（`rmqtt-delayed` **默认不随启动注册**）。

## 主题格式

```
$delayed/<interval>/<topic>
```

| 组成部分 | 说明 |
|------|------|
| `$delayed` | 固定前缀，标记这是一条延迟发布 |
| `<interval>` | 延迟秒数，整数（如 `60`、`3600`） |
| `<topic>` | 真实目标主题（订阅方按 `<topic>` 匹配接收） |

### 示例

```text
# 60 秒后投递到 a/b/c
$delayed/60/a/b/c

# 1 小时后投递到 x/y
$delayed/3600/x/y
```

消息转发时，`a/b/c` / `x/y` 的订阅方会像收到普通发布一样收到该消息。

## 配置

配置文件：`rmqtt-delayed.toml`（位于插件配置目录）。通过 `scx.plugins.read_config_default::<PluginConfig>("rmqtt-delayed")` 加载，插件 reload 时热生效。

| 选项 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `publish_max` | `usize` | `100_000` | 每节点待发延迟消息的最大数量。 |
| `publish_immediate` | `bool` | `true` | 达到上限时的行为：`true` — 立即作为普通消息转发；`false` — 丢弃消息（触发 `message_dropped` hook，reason 为 `DelayedPublishRefused`）。同一开关也决定插件停止/卸载时待发消息的终结处理（见「启用语义」）。 |

### 配置来源

1. `{plugins.dir}/rmqtt-delayed.toml`（文件，可选——文件缺失时使用默认值）
2. `rmqtt_plugin_rmqtt_delayed_*` 环境变量（将 TOML 键映射为带下划线前缀的环境变量）
3. 通过 `ServerContext::plugins_config_map_add()` 内联配置

### 示例

```toml
# 每节点待发延迟消息的最大数量
#publish_max = 100_000

# 超过 publish_max 时的行为：
# true  - 消息立即作为普通消息转发
# false - 消息被丢弃（触发 message_dropped hook，
#         reason 为 DelayedPublishRefused）
#publish_immediate = true
```

> **迁移说明**：这两个配置原先位于 `rmqtt.toml`（`mqtt.delayed_publish_max` / `mqtt.delayed_publish_immediate`），现已迁入插件配置并更名为 `publish_max` / `publish_immediate`；`rmqtt.toml` 中残留的旧键会被忽略。

## HTTP API

需要启用 `rmqtt-http-api` 插件。可在集群范围内查看待发延迟消息：

| 端点 | 说明 |
|------|------|
| `GET /api/v1/delayed_publishs` | 查询待发延迟消息（仅元数据），支持可选参数 `topic_filter` / `offset` / `limit`。集群范围查询，结果合并后按触发时间排序（最早优先，主题作次序键）。返回主题中 `$delayed/<interval>/` 前缀已剥离。响应：`{ "items": [...], "has_more": bool }`。 |
| `GET /api/v1/delayed_publishs/detail` | 按复合键 `node_id` / `topic` / `expired_time` / `client_id` 获取单条待发延迟消息的完整 payload。消息已触发则返回 404。 |

插件运行时属性（通过插件 attrs API 查看）会报告当前待发数量：`{"pending": <n>}`。

## 限制

- **仅内存态**：待发延迟消息不持久化。节点重启即丢失；插件停止/卸载时按 `publish_immediate` 做终结处理（立即转发，或经 `message_dropped` hook 丢弃）。
- **节点本地**：消息调度在接收该发布的节点上进行，队列不做跨节点同步。集群下 HTTP API 可跨节点聚合视图，但实际投递仍由源节点完成。
- **固定 500 ms 轮询**：实际投递时间可能比计划触发时间晚约 500 ms。

## 依赖

`rmqtt`（features: `plugin`、`stats`、`delayed`）、`tokio`、`async-trait`、`log`、`serde`、`serde_json`、`anyhow`

## 许可证

MIT OR Apache-2.0
