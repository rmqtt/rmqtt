[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-flapping

[![crates.io](https://img.shields.io/crates/v/rmqtt-flapping.svg)](https://crates.io/crates/rmqtt-flapping)

连接抖动防护插件。按 ClientId、用户名、来源地址三个维度在滑动窗口内统计 CONNECT 尝试，超过阈值者被封禁一段时间，期间连接一律被拒绝。

## 概述

一台不断重连的故障设备、一个失控的客户端、一次撞库攻击，从外部看是同一件事：短时间内同一来源发起大量 CONNECT。插件把这些尝试挂在 `ClientConnect` 钩子上筛检——这是**每一条**连接都会经过的钩子，包括不带用户名、因而永远不进入认证链的连接。

- **按键滑动窗口**：每个键有一个宽度为 `window_time` 的窗口和窗口内尝试计数。窗口内第 `max_count` 次尝试本身被拒绝，并封禁该键。
- **封禁后自动解封**：封禁持续 `ban_time`，生效期间不会被延长。创建封禁时会丢弃该键的窗口，使解封后的额度从零开始（否则当 `ban_time < window_time` 时，解封后的第一次尝试会立即再次触发封禁）。
- **表有上限**：每个维度最多记录 `max_track` 个键。表满之后新键会被放行但**不记账**，而不是被拒绝——选择拒绝会把「随机 ClientId 洪水」变成对所有其它客户端的故障。
- **清理很便宜**：每个截止时间同时记入时间序索引，因此周期清理只遍历真正过期的条目；空闲 broker 每个周期只付一次读时钟的代价。
- **可被外部读取**：封禁状态通过核心的 `rmqtt::flapping::Flapping` 扩展槽对外暴露，HTTP API 端点与 `flapping_banned` 统计都由它支撑。

### 拒绝码

| MQTT 版本 | 码值 | 名称 |
|-----------|------|------|
| 5.0 | `0x8A` | Banned |
| 3.1.1 / 3.1 | `0x05` | Not authorized |

v3 没有「封禁」码，只能取最接近的一个。刻意不用 `0x03`（*Server unavailable*）：它会诱导客户端立即重连。

## 使用方法

### 启用

在 `rmqtt.toml` 的 `plugins.default_startups` 列表中取消注释 `rmqtt-flapping`：

```toml
plugins.default_startups = [
    #"rmqtt-acl",
    #"rmqtt-auth-http",
    #"rmqtt-cluster-broadcast",
    #"rmqtt-cluster-raft",
    "rmqtt-flapping",       # <- 启用
    "rmqtt-web-hook",
    "rmqtt-http-api"
]
```

或在 `rmqttd/Cargo.toml` 中添加依赖，或通过 `rmqtt-plugins` 元 crate 启用：

```toml
# 直接依赖
rmqtt-flapping = { version = "0.25" }

# 或通过元 crate
rmqtt-plugins = { version = "0.25", features = ["flapping"] }
```

### 注册

```rust
rmqtt_flapping::register(&scx, true, false).await?;
```

参数说明：`(scx, default_startup, immutable)`。经 `rmqttd` 集成时，是否随启动注册由 `rmqtt-bin/Cargo.toml` 的 `package.metadata.plugins` 条目以及 `rmqtt.toml` 的 `plugins.default_startups` / `plugins.disabled_default_startups` 控制（`rmqtt-flapping` **默认不随启动注册**）。

## 配置

配置文件：`rmqtt-flapping.toml`（位于插件配置目录）。通过 `scx.plugins.read_config_default::<PluginConfig>("rmqtt-flapping")` 加载；reload 会重新读取策略表与白名单，且**不会丢掉已生效的封禁**。

| 选项 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `enable` | `bool` | `true` | 是否启用门禁。为 `false` 时插件照常加载并报告状态，但不拒绝任何连接。 |
| `max_track` | `usize` | `100_000` | **每个维度**记录的键数量上限。满表后，本应新建键的尝试会被放行但不记账，而不是被拒绝。 |
| `gc_interval` | `duration` | `"10s"` | 过期窗口与到期封禁的清理周期。不得小于 `1s`。 |
| `by_clientid` | table | （缺省） | `clientid` 维度的策略表。呈现该表即启用该维度。 |
| `by_username` | table | （缺省） | `username` 维度的策略表。不带用户名的连接不记账。 |
| `by_peerhost` | table | （缺省） | 来源 IP 维度的策略表。唯一能抓住「不断更换随机 ClientId」的维度。 |
| `notify_sys_topic` | `bool` | `true` | `$SYS` 通告的总开关。 |
| `sys_topic` | `string` | `"$SYS/brokers/{node}/flapping/banned"` | 封禁通告主题。`{node}` 会被替换为节点 ID。 |
| `sys_topic_qos` | `u8` | `0` | 通告的 QoS。 |
| `message_expiry_interval` | `duration` | `"5m"` | 通告的生命周期，在 `rmqtt-retainer` 加载时由它消费。 |
| `notify_on_every_refusal` | `bool` | `false` | 是否连封禁生效期间每一次被拒的尝试也通告。 |
| `notify_on_unban` | `bool` | `true` | 是否通告通过管理接口手工解除的封禁。自然到期的封禁永不通告。 |
| `sys_topic_unban` | `string` | `"$SYS/brokers/{node}/flapping/unbanned"` | 手工解封的通告主题。 |
| `allow_clientids` | `array` | `[]` | 永不记账、永不被封禁的 ClientId（精确匹配）。 |
| `allow_usernames` | `array` | `[]` | 永不记账、永不被封禁的用户名（精确匹配）。 |
| `allow_peerhosts` | `array` | `[]` | 永不记账、永不被封禁的来源 IP（精确匹配，不带端口）。 |

每张策略表有三个键：

| 选项 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `window_time` | `duration` | `"1m"` | 统计窗口宽度。 |
| `max_count` | `usize` | `15` | 窗口内触发封禁的次数；达到该次数的那一次尝试本身也被拒绝。`0` 会被替换为 `1`。 |
| `ban_time` | `duration` | `"5m"` | 封禁时长。 |

### 示例

```toml
enable = true
max_track = 100_000
gc_interval = "10s"

[by_clientid]
window_time = "1m"
max_count = 15
ban_time = "5m"

#[by_username]
#window_time = "1m"
#max_count = 15
#ban_time = "5m"

#[by_peerhost]
#window_time = "30s"
#max_count = 100
#ban_time = "10m"

notify_sys_topic = true
sys_topic = "$SYS/brokers/{node}/flapping/banned"
notify_on_unban = true
sys_topic_unban = "$SYS/brokers/{node}/flapping/unbanned"

allow_clientids = []
allow_usernames = []
allow_peerhosts = []
```

## HTTP API

需要启用 `rmqtt-http-api` 插件。两个端点都只回答收到请求的那个节点；封禁表是节点本地的，因此没有集群级聚合视图。

| 端点 | 说明 |
|------|------|
| `GET /api/v1/flapping/banned` | 列出生效中的封禁。查询参数 `dimension`（`clientid`/`username`/`peerhost`，留空为全部）、`key`（精确匹配）、`offset`、`limit`。响应 `{ "available": bool, "items": [...], "has_more": bool, "banned_count": n }`；未装门禁或门禁未在筛检时 `available` 为 `false`，这正是把「该功能未投入使用」与「没有任何封禁」区分开的依据。`/api/v1/features` 在 `flapping` 下报告同一状态。 |
| `DELETE /api/v1/flapping/banned` | 手工解除一条封禁。`dimension` 与 `key` **都作为查询参数**传递（键按原样精确匹配，且可能包含 `/`）。响应 `{ "dimension": ..., "key": ..., "unbanned": true }`；无封禁可解时返回 `404`。 |

## 指标

| 指标 | 位置 | 含义 |
|------|------|------|
| `conn.flapping.banned` | metrics | **创建**的封禁累计数。 |
| `conn.flapping.refused` | metrics | 因封禁生效而被拒绝的连接累计数。 |
| `flapping_banned.count` | stats | 当前**生效中**的封禁数；封禁到期会下降。 |
| `flapping_banned.max` | stats | 上者的历史峰值。 |

## 限制

- **状态是节点本地的。** 经负载均衡重连的客户端会在每个节点上分别计数，因此负载均衡后面有 `N` 个节点时，攻击者约能发起 `N × max_count` 次尝试才会被任一节点拒绝。请据此设置 `max_count`。
- **仅存于内存。** broker 重启会清空所有封禁。
- **不踢线。** 插件只筛检新连接；封禁创建时已在线的客户端保持连接。需要时请用 `DELETE /api/v1/clients/{clientid}`。
- **满表 fail-open**，见上文。
- **可能被更早的 handler 静默绕过。** 本插件在 `ClientConnect` 上注册于 `Priority::MAX / 2`；优先级高于该值且会短路的 handler 会在门禁执行前中断整条链，且不会有任何提示。请让这类 handler 取低于 `MAX / 2` 的优先级 —— 两个观察者（`rmqtt-counter`、`rmqtt-web-hook`）取 `MAX`，从不短路。

## 依赖

`rmqtt`（features: `plugin`、`msgstore`、`metrics`、`flapping`）、`tokio`、`async-trait`、`log`、`serde`、`serde_json`、`bytes`、`bytestring`、`chrono`、`parking_lot`

## 许可证

MIT OR Apache-2.0
