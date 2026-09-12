[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-acl

[![crates.io](https://img.shields.io/crates/v/rmqtt-acl.svg)](https://crates.io/crates/rmqtt-acl)

基于文件的访问控制列表（ACL）插件。通过 allow/deny 规则控制客户端的连接、发布和订阅权限，支持按用户名、IP 地址、客户端 ID 和主题过滤。

## 概述

`rmqtt-acl` 实现了一个基于规则的 ACL 引擎。规则按顺序评估，第一条匹配的规则决定结果。如果没有规则匹配，默认拒绝访问。插件注册了 5 个 Hook 回调，覆盖认证、ACL 检查和客户端生命周期事件。

**注意**：本插件同时挂载了 `ClientAuthenticate`（连接认证阶段）。默认规则集以 `["allow", "all"]` 结尾，其省略的动作列表示**包含 CONNECT 在内的所有操作**——启用自定义认证插件前，请先阅读[与认证插件的交互](#与认证插件的交互)。

## 使用方法

### 构建

在 `rmqttd/Cargo.toml` 中添加依赖：

```toml
rmqtt-acl = "0.21"
```

或通过 `rmqtt-plugins` 元 crate 启用：

```toml
rmqtt-plugins = { version = "0.21", features = ["acl"] }
```

### 注册

```rust
rmqtt_acl::register(&scx, true, false).await?;
// 或指定名称：
rmqtt_acl::register_named(&scx, "rmqtt-acl", true, false).await?;
```

参数说明：`(scx, default_startup, immutable)`。

### 配置

配置文件：`rmqtt-acl.toml`（位于插件配置目录）

```toml
# 发布被拒绝时断开客户端连接
disconnect_if_pub_rejected = true

rules = [
    # 允许 dashboard 用户订阅 $SYS/#
    ["allow", { user = "dashboard" }, "subscribe", ["$SYS/#"]],
    # 允许本地所有访问
    ["allow", { ipaddr = "127.0.0.1" }, "pubsub", ["$SYS/#", "#"]],
    # 拒绝所有人订阅 $SYS/# 和 #
    ["deny", "all", "subscribe", ["$SYS/#", { eq = "#" }]],
    # 允许其他所有
    ["allow", "all"]
]
```

### 规则格式

```
["allow" | "deny", <匹配对象>, <动作>, <主题列表>]
```

**`<匹配对象>`**：

| 格式 | 说明 |
|------|------|
| `"all"` | 匹配任意客户端 |
| `{ user = "username" }` | 按用户名匹配 |
| `{ ipaddr = "127.0.0.1" }` | 按 IP 地址匹配 |
| `{ clientid = "client123" }` | 按客户端 ID 匹配 |

**`<动作>`**：

| 值 | 说明 |
|-----|------|
| `"subscribe"` | 仅订阅 |
| `"publish"` | 仅发布 |
| `"pubsub"` | 订阅和发布 |
| `"connect"` | 仅连接（认证阶段） |
| `"all"` —— 或**省略该列** | 所有操作，**包括 CONNECT** |

**`<主题列表>`**：主题过滤器列表。支持 `{ eq = "exact/topic" }` 进行精确匹配。

### 与认证插件的交互

本插件同时挂载了 `ClientAuthenticate`。连接阶段的规则评估逻辑：

- 命中 `allow` 规则 → 连接被**显式放行**（认证 hook 链终止）。
- 命中 `deny` 规则 → 连接被拒绝。
- 无规则命中 → 连接被拒绝（`NotAuthorized`）。

默认规则末条 `["allow", "all"]` 省略了动作列，表示**包含 CONNECT 在内的所有操作**。由此带来以下行为：

- 除非更早执行的认证插件返回 `deny`，所有连接都会被本插件放行。
- 被自定义认证插件（rmqtt-auth-http、rmqtt-auth-jwt 等）判定为 **`ignore`** 的连接——例如认证服务返回非 2xx 状态码（404/500/502）——同样会被该规则放行，**即使 `allow_anonymous = false`**。

> **加固建议**：启用自定义认证插件时，请将 `rmqtt-acl.toml` 中的 `["allow", "all"]` 注释掉并启用 `["deny", "all"]`。此时本插件成为兜底的 deny-all（fail-closed）：只有被认证插件显式放行的客户端才能连接，发布/订阅必须由认证插件返回的 ACL 数据或上方的显式 allow 规则授权。

### 配置选项

| 选项 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `disconnect_if_pub_rejected` | `bool` | `true` | 发布被拒绝时断开客户端连接 |
| `rules` | `array` | — | 有序的 ACL 规则列表 |

## 配置来源

插件通过 `scx.plugins.load_config_default::<PluginConfig>("rmqtt-acl")` 加载配置，支持以下来源：

1. `{plugins.dir}/rmqtt-acl.toml`（文件，可选——文件缺失时使用默认值）
2. `rmqtt_plugin_rmqtt_acl_*` 环境变量
3. 通过 `ServerContext::plugins_config_map_add()` 内联配置

## Hook 回调

| Hook 类型 | 说明 |
|-----------|------|
| `ClientConnected` | 客户端连接后预计算主题占位符（`%c`=client_id, `%u`=username）和 ACL 规则 |
| `ClientDisconnected` | 清理客户端缓存的 topic filter |
| `ClientAuthenticate` | 对连接评估 ACL 规则：命中 `allow` 规则则放行，命中 `deny` 规则则拒绝，无规则命中则拒绝（`NotAuthorized`）。在更高优先级的认证插件之后执行（如 rmqtt-auth-http 默认 priority=100，本插件默认 10） |
| `ClientSubscribeCheckAcl` | 订阅时检查 ACL，返回 `SubscribeAclResult` |
| `MessagePublishCheckAcl` | 发布时检查 ACL，返回 `PublishAclResult` |

## 依赖

`rmqtt`（feature `plugin`）、`serde`、`tokio`、`async-trait`、`log`、`serde_json`、`ahash`、`dashmap`、`toml`、`anyhow`

## 许可证

MIT OR Apache-2.0
