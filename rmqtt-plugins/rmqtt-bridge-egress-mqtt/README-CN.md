# RMQTT Bridge Egress MQTT

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-egress-mqtt.svg)](https://crates.io/crates/rmqtt-bridge-egress-mqtt)
[![license](https://img.shields.io/crates/l/rmqtt-bridge-egress-mqtt.svg)](https://github.com/rmqtt/rmqtt/blob/master/LICENSE)

## 概述

`rmqtt-bridge-egress-mqtt` 是一个 RMQTT 插件，用于将本地 MQTT 发布消息转发到一个或多个远程 MQTT 代理。它充当出站桥接器，订阅本地主题并将匹配的消息重新发布到配置的远程代理，支持主题变换。

主要特性：
- 支持 MQTT v4 (3.1.1) 和 v5.0 协议版本
- TLS/SSL 加密连接到远程代理
- 可配置多个桥接实例
- 支持 `${local.topic}` 变量替换的主题变换
- 消息 QoS 和保留标志覆盖
- 遗嘱消息支持
- 自动重连

## 使用

### 添加依赖

在 `Cargo.toml` 中添加：

```toml
[dependencies]
rmqtt-bridge-egress-mqtt = { version = "*", features = ["plugin"] }
```

### 注册插件

在 RMQTT 节点配置中添加插件：

```toml
[[plugins]]
name = "rmqtt-bridge-egress-mqtt"
register = true
```

可选，指定自定义配置文件路径：

```toml
[[plugins]]
name = "rmqtt-bridge-egress-mqtt"
register = true
config_file = "/path/to/rmqtt-bridge-egress-mqtt.toml"
```

## 配置

配置文件使用 TOML 格式。以下是完整的配置选项列表。

### 桥接实例 (`[[bridges]]`)

每个桥接实例定义到远程 MQTT 代理的连接。

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `enable` | Boolean | `false` | 是否启用此桥接实例 |
| `name` | String | - | 唯一的桥接名称标识符 |
| `client_id_prefix` | String | - | MQTT 连接的客户端 ID 前缀 |
| `server` | String | - | 远程 MQTT 代理地址。格式：`tcp://host:port` 或 `tls://host:port` |
| `root_cert` | String | - | CA 签名服务器证书 PEM 文件路径（TLS） |
| `client_cert` | String | - | 客户端证书 PEM 文件路径（TLS） |
| `client_key` | String | - | 客户端私钥 PEM 文件路径（TLS） |
| `username` | String | - | 远程代理认证用户名。支持 `${ENV:VAR}` 环境变量替换 |
| `password` | String | - | 远程代理认证密码。支持 `${ENV:VAR}` 环境变量替换 |
| `concurrent_client_limit` | Integer | `5` | 连接到远程代理的最大并发客户端数 |
| `connect_timeout` | Duration | `20s` | 连接超时时间 |
| `keepalive` | Duration | `60s` | MQTT 保活间隔 |
| `reconnect_interval` | Duration | `5s` | 自动重连间隔 |
| `message_channel_capacity` | Integer | `100_000` | 通道可同时容纳的最大消息数 |
| `mqtt_ver` | String | `v5` | MQTT 协议版本：`v4` (3.1.1) 或 `v5` (5.0) |

#### MQTT v4 选项

当 `mqtt_ver = "v4"` 时应用的选项。

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `v4.clean_session` | Boolean | `true` | 连接时清除会话状态 |
| `v4.last_will` | Table | - | 遗嘱消息（可选）。见下方遗嘱消息表 |

#### MQTT v5 选项

当 `mqtt_ver = "v5"` 时应用的选项。

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `v5.clean_start` | Boolean | `true` | 连接时干净启动 |
| `v5.session_expiry_interval` | Duration | `0s` | 会话过期间隔。`0` 表示断开连接后会话立即结束 |
| `v5.receive_maximum` | Integer | `16` | 客户端可同时处理的 QoS 1 和 QoS 2 消息的最大数量 |
| `v5.maximum_packet_size` | Size | `1M` | 与代理协商的最大数据包大小 |
| `v5.topic_alias_maximum` | Integer | `0` | 最大主题别名数量 |
| `v5.last_will` | Table | - | 遗嘱消息（可选）。见下方遗嘱消息表 |

#### 遗嘱消息配置 (`v4.last_will` / `v5.last_will`)

| 字段 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `qos` | Integer | `0` | QoS 级别 (0, 1, 或 2) |
| `retain` | Boolean | `false` | 是否保留遗嘱消息 |
| `topic` | String | - | 遗嘱消息主题 |
| `message` | String | - | 遗嘱消息内容 |
| `encoding` | String | `plain` | 消息编码方式：`plain` 或 `base64` |

### 桥接条目 (`[[bridges.entries]]`)

每个桥接条目定义一个主题转发规则。

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `local.topic_filter` | String | - | 本地主题过滤器。匹配此主题过滤器的所有消息将被转发 |
| `remote.qos` | Integer | - | 转发消息的 QoS。值：0, 1, 2。不设置则跟随原始消息 QoS |
| `remote.retain` | Boolean | `false` | 是否保留转发消息 |
| `remote.topic` | String | - | 转发消息的远程主题。支持 `${local.topic}` 变量代表原始本地主题 |
| `remote.skip_levels` | Integer | `0` | 替换 `${local.topic}` 时要跳过的主题层级数。例如，本地主题为 `a/b/c/d` 且 `skip_levels = 2` 时，`${local.topic}` 解析为 `c/d` |

## 配置示例

```toml
[[bridges]]
enable = true
name = "bridge_name_1"
client_id_prefix = "prefix"
server = "tcp://127.0.0.1:2883"
username = "rmqtt_u"
password = "public"
concurrent_client_limit = 5
connect_timeout = "20s"
keepalive = "60s"
reconnect_interval = "5s"
message_channel_capacity = 100_000
mqtt_ver = "v5"

v5.clean_start = true
v5.session_expiry_interval = "0s"
v5.receive_maximum = 16
v5.maximum_packet_size = "1M"
v5.topic_alias_maximum = 0

[[bridges.entries]]
local.topic_filter = "local/topic/egress/#"
remote.qos = 1
remote.retain = false
remote.topic = "remote/topic/egress/${local.topic}"
```

## 依赖

- rmqtt（功能：plugin）
- ntex
- ntex-mqtt
- ntex-tls
- tokio
- rustls

## 许可证

本项目基于 MIT 许可证。详情请参阅 [LICENSE](https://github.com/rmqtt/rmqtt/blob/master/LICENSE) 文件。
