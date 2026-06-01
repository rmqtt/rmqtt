# RMQTT Bridge Egress NATS

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-egress-nats.svg)](https://crates.io/crates/rmqtt-bridge-egress-nats)
[![license](https://img.shields.io/crates/l/rmqtt-bridge-egress-nats.svg)](https://github.com/rmqtt/rmqtt/blob/master/LICENSE)

## 概述

`rmqtt-bridge-egress-nats` 是一个 RMQTT 插件，用于将本地 MQTT 发布消息转发到一个或多个 NATS 主题。它充当出站桥接器，订阅本地 MQTT 主题并将消息发布到 NATS，支持主题变换。

主要特性：
- 将 MQTT 消息转发到 NATS 主题
- TLS/SSL 加密连接
- 多种认证方式（JWT, NKey, 用户名/密码, 令牌）
- 转发 MQTT 元数据（客户端信息、发布标志）
- 可配置的 NATS 连接选项（no echo、ping 间隔、连接超时）
- 支持 `${local.topic}` 变量替换的主题变换

## 使用

### 添加依赖

在 `Cargo.toml` 中添加：

```toml
[dependencies]
rmqtt-bridge-egress-nats = { version = "*", features = ["plugin"] }
```

### 注册插件

在 RMQTT 节点配置中添加插件：

```toml
[[plugins]]
name = "rmqtt-bridge-egress-nats"
register = true
```

可选，指定自定义配置文件路径：

```toml
[[plugins]]
name = "rmqtt-bridge-egress-nats"
register = true
config_file = "/path/to/rmqtt-bridge-egress-nats.toml"
```

## 配置

配置文件使用 TOML 格式。以下是完整的配置选项列表。

### 桥接实例 (`[[bridges]]`)

每个桥接实例定义到 NATS 服务器的连接。

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `enable` | Boolean | `false` | 是否启用此桥接实例 |
| `name` | String | - | 唯一的桥接名称标识符 |
| `servers` | String | - | NATS 服务器地址。格式：`nats://host:port`（普通 TCP）或 `tls://host:port`（TLS） |
| `producer_name_prefix` | String | - | 生产者名称前缀 |
| `no_echo` | Boolean | - | 不接收此连接发布的消息 |
| `ping_interval` | Duration | `60s` | 保活 ping 间隔 |
| `connection_timeout` | Duration | `10s` | 连接超时时间 |
| `tls_required` | Boolean | - | 需要 TLS 连接 |
| `tls_first` | Boolean | - | 在 NATS 协议之前进行 TLS 握手 |
| `root_certificates` | String | - | 根证书文件路径 |
| `client_cert` | String | - | 客户端证书文件路径 |
| `client_key` | String | - | 客户端密钥文件路径 |
| `sender_capacity` | Integer | `256` | 发送者通道容量 |
| `auth.jwt` | String | - | JWT 认证令牌 |
| `auth.jwt_seed` | String | - | JWT 签名种子 |
| `auth.nkey` | String | - | NKey 认证 |
| `auth.username` | String | - | 用户名认证 |
| `auth.password` | String | - | 密码认证 |
| `auth.token` | String | - | 令牌认证 |

### 桥接条目 (`[[bridges.entries]]`)

每个桥接条目定义一个主题转发规则。

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `local.topic_filter` | String | - | 本地 MQTT 主题过滤器。匹配此过滤器的所有消息将被转发 |
| `remote.topic` | String | - | 要发布到的 NATS 主题。支持 `${local.topic}` 变量替换 |
| `remote.forward_all_from` | Boolean | `false` | 转发所有来源数据：`from_type`, `from_node`, `from_ipaddress`, `from_clientid`, `from_username` |
| `remote.forward_all_publish` | Boolean | `false` | 转发所有发布数据：`dup`, `retain`, `qos`, `packet_id`, `topic`, `payload` |
| `remote.skip_levels` | Integer | `0` | 替换 `${local.topic}` 时要跳过的主题层级数 |

## 配置示例

```toml
[[bridges]]
enable = true
name = "bridge_nats_1"
servers = "nats://127.0.0.1:4222"
producer_name_prefix = "producer_1"

[[bridges.entries]]
local.topic_filter = "local/topic1/egress/#"
remote.topic = "test1"
```

## 依赖

- rmqtt（功能：plugin）
- async-nats
- tokio

## 许可证

本项目基于 MIT 许可证。详情请参阅 [LICENSE](https://github.com/rmqtt/rmqtt/blob/master/LICENSE) 文件。
