# RMQTT Bridge Ingress MQTT

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-ingress-mqtt.svg)](https://crates.io/crates/rmqtt-bridge-ingress-mqtt)
[![license](https://img.shields.io/crates/l/rmqtt-bridge-ingress-mqtt.svg)](https://github.com/rmqtt/rmqtt/blob/master/LICENSE)

## 概述

`rmqtt-bridge-ingress-mqtt` 是一个 RMQTT 插件，用于连接到远程 MQTT 代理并将消息转发到本地 RMQTT 代理。它充当入站桥接器，订阅远程主题并将接收到的消息重新发布到本地，支持主题变换。

主要特性：
- 支持 MQTT v4 (3.1.1) 和 v5.0 协议版本
- TLS/SSL 加密连接到远程代理
- 可配置多个桥接实例
- 支持 `${remote.topic}` 变量替换的主题变换
- 共享订阅支持（`$share/` 前缀）
- 消息过期间隔
- 自动重连

## 使用

### 添加依赖

在 `Cargo.toml` 中添加：

```toml
[dependencies]
rmqtt-bridge-ingress-mqtt = { version = "*", features = ["plugin"] }
```

### 注册插件

在 RMQTT 节点配置中添加插件：

```toml
[[plugins]]
name = "rmqtt-bridge-ingress-mqtt"
register = true
```

可选，指定自定义配置文件路径：

```toml
[[plugins]]
name = "rmqtt-bridge-ingress-mqtt"
register = true
config_file = "/path/to/rmqtt-bridge-ingress-mqtt.toml"
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
| `expiry_interval` | Duration | `5m` | 消息过期时间。`0` 表示永不过期 |
| `mqtt_ver` | String | `v4` | MQTT 协议版本：`v4` (3.1.1) 或 `v5` (5.0) |

#### MQTT v4 选项

当 `mqtt_ver = "v4"` 时应用的选项。

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `v4.clean_session` | Boolean | `true` | 连接时清除会话状态 |

#### MQTT v5 选项

当 `mqtt_ver = "v5"` 时应用的选项。

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `v5.clean_start` | Boolean | `true` | 连接时干净启动 |
| `v5.session_expiry_interval` | Duration | `0s` | 会话过期间隔。`0` 表示断开连接后会话立即结束 |
| `v5.receive_maximum` | Integer | `16` | 客户端可同时处理的 QoS 1 和 QoS 2 消息的最大数量 |
| `v5.maximum_packet_size` | Size | `1M` | 与代理协商的最大数据包大小 |
| `v5.topic_alias_maximum` | Integer | `0` | 最大主题别名数量 |

### 桥接条目 (`[[bridges.entries]]`)

每个桥接条目定义一个主题订阅和转发规则。

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `remote.qos` | Integer | `0` | 订阅远程主题的 QoS。值：0, 1, 2 |
| `remote.topic` | String | - | 要订阅的远程主题。支持共享订阅格式 `$share/g/remote/topic` |
| `local.qos` | Integer | - | 发布到本地代理的 QoS。值：0, 1, 2。不设置则跟随原始消息 QoS |
| `local.topic` | String | - | 本地发布主题。支持 `${remote.topic}` 变量 |
| `local.retain` | Boolean | `false` | 是否在本地保留发布的消息 |

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
expiry_interval = "5m"
mqtt_ver = "v4"

v4.clean_session = true

[[bridges.entries]]
remote.qos = 1
remote.topic = "$share/g/remote/topic1/ingress/#"
local.qos = 1
local.topic = "local/topic1/ingress/${remote.topic}"
local.retain = true
```

## 依赖

- rmqtt（功能：plugin）
- ntex
- ntex-mqtt
- tokio

## 许可证

本项目基于 MIT 许可证。详情请参阅 [LICENSE](https://github.com/rmqtt/rmqtt/blob/master/LICENSE) 文件。
