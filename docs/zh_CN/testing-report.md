[English](../en_US/testing-report.md) | [**简体中文**](testing-report.md)

# RMQTT 测试报告

本文档提供 RMQTT MQTT Broker 的详细测试结果，包括针对 paho.mqtt.testing 套件的互操作性测试和性能基准数据。

---

## 互操作性测试

RMQTT 通过了官方 [paho.mqtt.testing](https://github.com/eclipse/paho.mqtt.testing) 互操作性测试套件。

### 环境准备

```bash
git clone https://github.com/eclipse/paho.mqtt.testing.git
cd paho.mqtt.testing/interoperability

# 另开终端启动 RMQTT Broker
./target/release/rmqttd
```

### MQTT V3.1.1 — 11/11 通过

| 测试 | 结果 | 备注 |
|------|------|------|
| `test_retained_messages` | ✅ 通过 | — |
| `test_zero_length_clientid` | ✅ 通过 | — |
| `will_message_test` | ✅ 通过 | — |
| `test_offline_message_queueing` | ✅ 通过 | — |
| `test_overlapping_subscriptions` | ✅ 通过 | — |
| `test_keepalive` | ✅ 通过 | — |
| `test_redelivery_on_reconnect` | ✅ 通过 | — |
| `test_dollar_topics` | ✅ 通过 | — |
| `test_unsubscribe` | ✅ 通过 | — |
| `test_subscribe_failure` | ✅ 通过 | 需在 `rmqtt-acl.toml` 首行添加：`["deny", "all", "subscribe", ["test/nosubscribe"]]` |
| `test_zero_length_clientid` | ✅ 通过 | — |

### MQTT V5.0 — 24/24 通过

| 测试 | 结果 |
|------|------|
| `test_retained_message` | ✅ 通过 |
| `test_will_message` | ✅ 通过 |
| `test_offline_message_queueing` | ✅ 通过 |
| `test_dollar_topics` | ✅ 通过 |
| `test_unsubscribe` | ✅ 通过 |
| `test_session_expiry` | ✅ 通过 |
| `test_shared_subscriptions` | ✅ 通过 |
| `test_basic` | ✅ 通过 |
| `test_overlapping_subscriptions` | ✅ 通过 |
| `test_redelivery_on_reconnect` | ✅ 通过 |
| `test_payload_format` | ✅ 通过 |
| `test_publication_expiry` | ✅ 通过 |
| `test_subscribe_options` | ✅ 通过 |
| `test_assigned_clientid` | ✅ 通过 |
| `test_subscribe_identifiers` | ✅ 通过 |
| `test_request_response` | ✅ 通过 |
| `test_server_topic_alias` | ✅ 通过 |
| `test_client_topic_alias` | ✅ 通过 |
| `test_maximum_packet_size` | ✅ 通过 |
| `test_keepalive` | ✅ 通过 |
| `test_zero_length_clientid` | ✅ 通过 |
| `test_user_properties` | ✅ 通过 |
| `test_flow_control2` | ✅ 通过 |
| `test_flow_control1` | ✅ 通过 |
| `test_will_delay` | ✅ 通过 |
| `test_server_keep_alive` | ✅ 通过（需将 `rmqtt.toml` 中 `max_keepalive` 改为 60） |
| `test_subscribe_failure` | ✅ 通过（ACL 配置同 v3.1.1） |

---

## 集成测试框架

`rmqtt-test` crate 提供了自定义测试框架，在 paho 之外还包含以下套件：

| 套件 | 用例数 | 说明 |
|-------|--------|------|
| `functional_v3` | 2 | MQTT 3.1 基本操作 |
| `functional_v311` | 10 | MQTT 3.1.1 协议合规 |
| `functional_v5` | 5 | MQTT 5.0 协议合规 |
| `stress` | 3 | 连接负载、发布 QPS、扇出测试 |
| `chaos` | 6 | Broker 重启、连接风暴、重连、QoS 1 可靠性、慢消费者 |

```bash
# 运行所有测试套件
cargo build --release
cargo build -p rmqtt-test --release
./target/release/mqtt_harness --workspace .
```

---

## 性能基准

### 环境

| 项目 | 内容 |
|------|------|
| 操作系统 | x86_64 GNU/Linux, Rocky Linux 9.2 (Blue Onyx) |
| CPU | Intel(R) Xeon(R) CPU E5-2696 v3 @ 2.30GHz, 72 线程 |
| 内存 | DDR3/2333, 128 GB |
| 磁盘 | 2 TB |
| 容器 | Podman v4.4.1 |
| 测试工具 | `rmqtt/rmqtt-bench:latest` (v0.1.3) |
| MQTT Broker | `rmqtt/rmqtt:latest` (v0.3.0) |

*测试客户端和 Broker 同机部署。*

### 连接并发性能

| 指标 | 单节点 | Raft 集群（3 节点） |
|------|--------|-------------------|
| 并发客户端总数 | 1,000,000 | 1,000,000 |
| 连接握手速率 | 5,500-7,000/秒 | 5,000-7,000/秒 |

### 消息吞吐性能

| 指标 | 单节点 | Raft 集群（3 节点） |
|------|--------|-------------------|
| 订阅客户端数 | 1,000,000 | 1,000,000 |
| 发布客户端数 | 40 | 40 |
| 消息吞吐速率 | 150,000 条/秒 | 156,000 条/秒 |

[详细基准测试文档 →](./benchmark-testing.md)

---

## 许可证

MIT OR Apache-2.0
