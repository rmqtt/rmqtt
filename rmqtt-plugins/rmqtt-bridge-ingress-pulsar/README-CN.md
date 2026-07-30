# RMQTT Bridge Ingress Pulsar

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-ingress-pulsar.svg)](https://crates.io/crates/rmqtt-bridge-ingress-pulsar)
[![license](https://img.shields.io/crates/l/rmqtt-bridge-ingress-pulsar.svg)](https://github.com/rmqtt/rmqtt/blob/master/LICENSE)

## 概述

`rmqtt-bridge-ingress-pulsar` 是一个 RMQTT 插件，用于从 Apache Pulsar 主题消费消息并发布到本地 RMQTT 代理。它充当入站桥接器，将 Pulsar 消息桥接到 MQTT，提供全面的订阅和消息变换能力。

主要特性：
- 使用多种订阅类型（exclusive, shared, failover, key_shared）消费 Pulsar 主题
- 支持单主题、多主题和正则表达式主题模式
- 支持证书链验证的 TLS/SSL 加密连接
- Token (JWT/Biscuit) 和 OAuth2 认证支持
- 灵活的占位符主题变换（`${remote.topic}`, `${remote.properties.*}`, `${remote.payload.*}`）
- 消费者配置（批量大小、死信策略、优先级级别）
- 消息格式支持（bytes, JSON）及负载路径提取
- 消费者组（订阅）管理
- 死信策略支持
- 可配置的消息过期间隔

## 使用

### 添加依赖

在 `Cargo.toml` 中添加：

```toml
[dependencies]
rmqtt-bridge-ingress-pulsar = { version = "*", features = ["plugin"] }
```

### 注册插件

在 RMQTT 节点配置中添加插件：

```toml
[[plugins]]
name = "rmqtt-bridge-ingress-pulsar"
register = true
```

可选，指定自定义配置文件路径：

```toml
[[plugins]]
name = "rmqtt-bridge-ingress-pulsar"
register = true
config_file = "/path/to/rmqtt-bridge-ingress-pulsar.toml"
```

## 配置

配置文件使用 TOML 格式。以下是完整的配置选项列表。

### 桥接实例 (`[[bridges]]`)

每个桥接实例定义到 Pulsar 集群的连接。

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `enable` | Boolean | `false` | 是否启用此桥接实例 |
| `name` | String | - | 唯一的桥接名称标识符 |
| `servers` | String | - | Pulsar 代理地址。格式：`pulsar://host:port`（普通 TCP）或 `pulsar+ssl://host:port`（TLS） |
| `consumer_name_prefix` | String | - | 消费者名称前缀 |
| `cert_chain_file` | String | - | TLS 认证的自定义证书链文件路径 |
| `allow_insecure_connection` | Boolean | `false` | 允许不安全的 TLS 连接 |
| `tls_hostname_verification_enabled` | Boolean | `true` | 是否启用 TLS 主机名验证 |
| `auth.name` | String | - | 认证类型：`token`（JWT/Biscuit）或 `oauth2` |
| `auth.data` | String | - | 认证数据。token 类型：JWT/Biscuit token 字符串。oauth2 类型：包含 `issuer_url` 和 `credentials_url` 的 JSON |
| `expiry_interval` | Duration | `5m` | 消息过期时间。`0` 表示永不过期 |

### 桥接条目 (`[[bridges.entries]]`)

每个桥接条目定义一个主题消费和转发规则。

#### 消费者配置

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `remote.topic` | String | - | 要消费的 Pulsar 主题 |
| `remote.topics` | Array of String | - | 消费者主题列表 |
| `remote.topic_regex` | String | - | 匹配主题的正则表达式模式。消费者监听所有匹配的主题 |
| `remote.subscription_type` | String | `shared` | 订阅类型：`exclusive`, `shared`, `failover`, `key_shared` |
| `remote.subscription` | String | - | 订阅名称 |
| `lookup_namespace` | String | `public/default` | 正则匹配的租户/命名空间 |
| `topic_refresh_interval` | Duration | `10s` | 使用正则模式时刷新主题的间隔 |
| `consumer_id` | Integer | - | 消费者 ID |
| `batch_size` | Integer | `1000` | Pulsar 消息的最大批量大小 |
| `remote.dead_letter_policy` | Table | - | 死信策略：`{max_redeliver_count, dead_letter_topic}` |
| `remote.unacked_message_resend_delay` | Duration | - | 未确认消息重新发送前的延迟时间 |

#### 消费者选项 (`remote.options`)

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `remote.options.priority_level` | Integer | - | 消费者优先级级别 |
| `remote.options.durable` | Boolean | - | 订阅是否由持久化游标支持 |
| `remote.options.read_compacted` | Boolean | - | 读取压缩的主题 |
| `remote.options.initial_position` | String | `latest` | 初始位置：`earliest`（最早）或 `latest`（最新） |
| `remote.options.metadata` | Table | `{}` | 添加到所有消息的用户自定义属性 |

#### 负载和主题映射

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `remote.payload_format` | String | `bytes` | 负载数据格式：`bytes` 或 `json` |
| `remote.payload_path` | String | - | 从 Pulsar JSON 消息负载中提取发布负载。例如：`/msg` 或 `/data/msg` |
| `local.topic` | String | - | MQTT 发布主题。占位符：`${remote.topic}`, `${remote.properties.xxxx}`, `${remote.payload.xxxx}` |
| `local.topics` | Array of String | - | 多个 MQTT 发布主题（含占位符） |
| `local.qos` | Integer | - | 发布 QoS。值：0, 1, 2。不设置则跟随 Pulsar 消息属性 |
| `local.retain` | Boolean | `false` | 是否在本地保留发布的消息 |
| `local.allow_empty_forward` | Boolean | `true` | 允许转发空消息 |

## 配置示例

```toml
[[bridges]]
enable = true
name = "bridge_pulsar_1"
servers = "pulsar://127.0.0.1:6650"
consumer_name_prefix = "consumer_1"
expiry_interval = "5m"

[[bridges.entries]]
remote.topic = "non-persistent://public/default/test"
local.topic = "local/topic1/ingress/${remote.topic}"
```

## 依赖

- rmqtt（功能：plugin）
- pulsar
- tokio

## 许可证

本项目基于 MIT 许可证。详情请参阅 [LICENSE](https://github.com/rmqtt/rmqtt/blob/master/LICENSE) 文件。
