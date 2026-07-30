# RMQTT Bridge Ingress Kafka

[![crates.io](https://img.shields.io/crates/v/rmqtt-bridge-ingress-kafka.svg)](https://crates.io/crates/rmqtt-bridge-ingress-kafka)
[![license](https://img.shields.io/crates/l/rmqtt-bridge-ingress-kafka.svg)](https://github.com/rmqtt/rmqtt/blob/master/LICENSE)

## 概述

`rmqtt-bridge-ingress-kafka` 是一个 RMQTT 插件，用于从一个或多个 Apache Kafka 主题消费消息并发布到本地 RMQTT 代理。它充当入站桥接器，将 Kafka 消息桥接到 MQTT，支持灵活的主题映射和消费者组管理。

主要特性：
- 使用消费者组从 Kafka 主题消费消息
- 支持 `${kafka.key}` 变量替换的灵活主题变换
- 可配置的消费者偏移起始位置
- 可配置的 Kafka 消费者属性（librdkafka 设置）
- 分区级别消费控制
- 消息过期间隔

## 使用

### 添加依赖

在 `Cargo.toml` 中添加：

```toml
[dependencies]
rmqtt-bridge-ingress-kafka = { version = "*", features = ["plugin"] }
```

### 注册插件

在 RMQTT 节点配置中添加插件：

```toml
[[plugins]]
name = "rmqtt-bridge-ingress-kafka"
register = true
```

可选，指定自定义配置文件路径：

```toml
[[plugins]]
name = "rmqtt-bridge-ingress-kafka"
register = true
config_file = "/path/to/rmqtt-bridge-ingress-kafka.toml"
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
| `expiry_interval` | Duration | `5m` | 消息过期时间。`0` 表示永不过期 |

### Kafka 属性 (`[bridges.properties]`)

额外的 librdkafka 消费者属性。请参阅 [librdkafka 配置文档](https://github.com/confluentinc/librdkafka/blob/master/CONFIGURATION.md) 了解所有可用选项。

| 属性 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `message.timeout.ms` | String | `5000` | 消息超时时间（毫秒） |
| `enable.auto.commit` | String | `true` | 是否启用消费者偏移自动提交 |
| 其他任何 librdkafka 属性 | String | - | 参见 librdkafka 配置文档 |

### 桥接条目 (`[[bridges.entries]]`)

每个桥接条目定义一个主题消费和转发规则。

| 选项 | 类型 | 默认值 | 描述 |
|------|------|--------|------|
| `remote.topic` | String | - | 要消费的 Kafka 主题 |
| `remote.group_id` | String | - | Kafka 消费者组 ID |
| `remote.start_partition` | Integer | `-1` | 起始分区索引（包含）。`-1` 表示所有分区 |
| `remote.stop_partition` | Integer | `-1` | 结束分区索引（包含）。`-1` 表示所有分区 |
| `remote.offset` | String | `end` | 消费者偏移起始位置。值：`beginning`, `end`, `stored`, 或具体的偏移数值 |
| `local.qos` | Integer | - | 发布到本地代理的 QoS。值：0, 1, 2。不设置则跟随 Kafka 消息元数据 |
| `local.topic` | String | - | 本地 MQTT 发布主题。支持 `${kafka.key}` 变量用于 Kafka 消息键 |
| `local.retain` | Boolean | `false` | 是否在本地保留发布的消息 |

## 配置示例

```toml
[[bridges]]
enable = true
name = "bridge_kafka_1"
servers = "127.0.0.1:9092"
client_id_prefix = "kafka_001"
expiry_interval = "5m"

[bridges.properties]
"message.timeout.ms" = "5000"

[[bridges.entries]]
remote.topic = "remote-topic1-ingress"
remote.group_id = "group_id_001"
local.topic = "local/topic1/ingress/${kafka.key}"
```

## 依赖

- rmqtt（功能：plugin）
- rdkafka
- tokio

## 许可证

本项目基于 MIT 许可证。详情请参阅 [LICENSE](https://github.com/rmqtt/rmqtt/blob/master/LICENSE) 文件。
