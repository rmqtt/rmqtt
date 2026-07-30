# RMQTT Bridge Egress Pulsar

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-egress-pulsar.svg)](https://crates.io/crates/rmqtt-bridge-egress-pulsar)
[![license](https://img.shields.io/crates/l/rmqtt-bridge-egress-pulsar.svg)](https://github.com/rmqtt/rmqtt/blob/master/LICENSE)

## 概述

`rmqtt-bridge-egress-pulsar` 是一个 RMQTT 插件，用于将本地 MQTT 发布消息转发到一个或多个 Apache Pulsar 主题。它充当出站桥接器，订阅本地 MQTT 主题并将消息生成到 Pulsar，提供全面的消息变换能力。

主要特性：
- 将 MQTT 消息转发到 Pulsar 持久化和非持久化主题
- 支持证书链验证的 TLS/SSL 加密连接
- Token (JWT/Biscuit) 和 OAuth2 认证支持
- 通过分区键实现消息分区
- 通过排序键实现消息排序
- 可配置的压缩算法（lz4, zlib, zstd, snappy）
- 生产者访问模式配置
- 消息元数据和 Schema 版本支持
- 转发 MQTT 元数据（客户端信息、发布标志）

## 使用

### 添加依赖

在 `Cargo.toml` 中添加：

```toml
[dependencies]
rmqtt-bridge-egress-pulsar = { version = "*", features = ["plugin"] }
```

### 注册插件

在 RMQTT 节点配置中添加插件：

```toml
[[plugins]]
name = "rmqtt-bridge-egress-pulsar"
register = true
```

可选，指定自定义配置文件路径：

```toml
[[plugins]]
name = "rmqtt-bridge-egress-pulsar"
register = true
config_file = "/path/to/rmqtt-bridge-egress-pulsar.toml"
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
| `producer_name_prefix` | String | - | 生产者名称前缀 |
| `cert_chain_file` | String | - | TLS 认证的自定义证书链文件路径 |
| `allow_insecure_connection` | Boolean | `false` | 允许不安全的 TLS 连接 |
| `tls_hostname_verification_enabled` | Boolean | `true` | 是否启用 TLS 主机名验证 |
| `auth.name` | String | - | 认证类型：`token`（JWT/Biscuit）或 `oauth2` |
| `auth.data` | String | - | 认证数据。token 类型：JWT/Biscuit token 字符串。oauth2 类型：包含 `issuer_url` 和 `credentials_url` 的 JSON |

### 桥接条目 (`[[bridges.entries]]`)

每个桥接条目定义一个主题转发规则。

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `local.topic_filter` | String | - | 本地 MQTT 主题过滤器。匹配此过滤器的所有消息将被转发 |
| `remote.topic` | String | - | 要生成的 Pulsar 主题。支持 `${local.topic}` 变量替换 |
| `remote.forward_all_from` | Boolean | `false` | 转发所有来源数据：`from_type`, `from_node`, `from_ipaddress`, `from_clientid`, `from_username` |
| `remote.forward_all_publish` | Boolean | `false` | 转发所有发布数据：`dup`, `retain`, `qos`, `packet_id`, `topic`, `payload` |
| `remote.partition_key` | String | - | 确定目标分区。相同分区键的消息发送到同一分区 |
| `remote.ordering_key` | String 或 Table | - | 控制消息排序。值：`clientid`, `uuid`, `random`, 或 `{type="random", len=10}` |
| `remote.replicate_to` | Array | - | 覆盖命名空间复制目标 |
| `remote.schema_version` | String | - | 消息的 Schema 版本 |
| `remote.options.metadata` | Table | `{}` | 添加到所有消息的用户自定义属性 |
| `remote.options.compression` | String | - | 消息压缩算法：`lz4`, `zlib`, `zstd`, `snappy` |
| `remote.options.access_mode` | Integer | `0` | 生产者访问模式：`0` = shared, `1` = exclusive, `2` = wait_for_exclusive, `3` = exclusive_without_fencing |
| `remote.skip_levels` | Integer | `0` | 替换 `${local.topic}` 时要跳过的主题层级数 |

## 配置示例

```toml
[[bridges]]
enable = true
name = "bridge_pulsar_1"
servers = "pulsar://127.0.0.1:6650"
producer_name_prefix = "producer_1"

[[bridges.entries]]
local.topic_filter = "local/topic1/egress/#"
remote.topic = "non-persistent://public/default/test1"
remote.forward_all_from = false
remote.forward_all_publish = false
```

## 依赖

- rmqtt（功能：plugin）
- pulsar
- tokio

## 许可证

本项目基于 MIT 许可证。详情请参阅 [LICENSE](https://github.com/rmqtt/rmqtt/blob/master/LICENSE) 文件。
