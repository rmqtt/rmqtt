# RMQTT Bridge Egress Kafka

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-egress-kafka.svg)](https://crates.io/crates/rmqtt-bridge-egress-kafka)
[![license](https://img.shields.io/crates/l/rmqtt-bridge-egress-kafka.svg)](https://github.com/rmqtt/rmqtt/blob/master/LICENSE)

## 概述

`rmqtt-bridge-egress-kafka` 是一个 RMQTT 插件，用于将本地 MQTT 发布消息转发到一个或多个 Apache Kafka 主题。它充当出站桥接器，订阅本地 MQTT 主题并将消息生成到 Kafka，支持主题变换。

主要特性：
- 将 MQTT 消息转发到 Kafka 主题
- 支持 `${local.topic}` 变量替换的主题变换
- 可配置的 Kafka 生产者属性（librdkafka 设置）
- 多桥接和多条目配置
- 自动重连

## 使用

### 添加依赖

在 `Cargo.toml` 中添加：

```toml
[dependencies]
rmqtt-bridge-egress-kafka = { version = "*", features = ["plugin"] }
```

### 注册插件

在 RMQTT 节点配置中添加插件：

```toml
[[plugins]]
name = "rmqtt-bridge-egress-kafka"
register = true
```

可选，指定自定义配置文件路径：

```toml
[[plugins]]
name = "rmqtt-bridge-egress-kafka"
register = true
config_file = "/path/to/rmqtt-bridge-egress-kafka.toml"
```

## 配置

配置文件使用 TOML 格式。以下是完整的配置选项列表。

### 桥接实例 (`[[bridges]]`)

每个桥接实例定义到 Kafka 集群的连接。

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `enable` | Boolean | `false` | 是否启用此桥接实例 |
| `name` | String | - | 唯一的桥接名称标识符 |
| `servers` | String | - | Kafka 代理地址列表（逗号分隔）。格式：`host1:port1,host2:port2` |
| `client_id_prefix` | String | - | Kafka 客户端 ID 前缀 |
| `concurrent_client_limit` | Integer | `3` | 连接到远程 Kafka 代理的最大客户端限制 |

### Kafka 属性 (`[bridges.properties]`)

额外的 librdkafka 生产者属性。请参阅 [librdkafka 配置文档](https://github.com/confluentinc/librdkafka/blob/master/CONFIGURATION.md) 了解所有可用选项。

| 属性 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `message.timeout.ms` | String | `5000` | 消息超时时间（毫秒） |
| 其他任何 librdkafka 属性 | String | - | 参见 librdkafka 配置文档 |

### 桥接条目 (`[[bridges.entries]]`)

每个桥接条目定义一个主题转发规则。

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `local.topic_filter` | String | - | 本地 MQTT 主题过滤器。匹配此过滤器的所有消息将被转发 |
| `remote.topic` | String | - | 要生成的 Kafka 主题。支持 `${local.topic}` 变量替换 |
| `remote.queue_timeout` | Duration | `0m` | librdkafka 生产者队列满时的重试时间。`0` 表示永不阻塞 |
| `remote.partition` | Integer | - | 记录的目标分区（可选） |
| `remote.skip_levels` | Integer | `0` | 替换 `${local.topic}` 时要跳过的主题层级数 |

## 配置示例

```toml
[[bridges]]
enable = true
name = "bridge_kafka_1"
servers = "127.0.0.1:9092"
client_id_prefix = "kafka_001"
concurrent_client_limit = 3

[bridges.properties]
"message.timeout.ms" = "5000"

[[bridges.entries]]
local.topic_filter = "local/topic1/egress/#"
remote.topic = "remote-topic1-egress-${local.topic}"
remote.queue_timeout = "0m"
```

## 依赖

- rmqtt（功能：plugin）
- rdkafka
- tokio

## 许可证

本项目基于 MIT 许可证。详情请参阅 [LICENSE](https://github.com/rmqtt/rmqtt/blob/master/LICENSE) 文件。
