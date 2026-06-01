[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-test

[![crates.io page](https://img.shields.io/crates/v/rmqtt.svg)](https://crates.io/crates/rmqtt)
![Rust](https://img.shields.io/badge/rust-1.89%2B-blue)

RMQTT 的工业级验证与压测核心引擎（Test Harness + Chaos + Benchmark）。编译产物 `mqtt_harness` 作为独立可执行程序，提供功能测试、压力测试、混沌测试，并输出结构化测试报告。

## ✨ 特性

- **自研 MQTT 客户端** — 零第三方 MQTT 依赖，完整实现 MQTT 3.1 / 3.1.1 / 5.0 协议栈
- **Broker 生命周期管理** — 自动启动/停止/重启 rmqttd 进程，TCP 健康检查
- **五类测试套件** — functional_v3 / functional_v311 / functional_v5 / stress / chaos
- **QoS 全覆盖** — QoS 0 / QoS 1 / QoS 2（含完整四步握手）正确性验证
- **混沌注入** — Broker 重启、连接风暴、慢消费者、丢包模拟
- **多格式报告** — Console + JSON + HTML
- **DAG 调度** — 测试用例依赖关系拓扑排序，超时与重试机制
- **详细诊断日志** — 失败测试自动记录原因与诊断提示；MQTT 包级十六进制跟踪
- **100% Safe Rust** — `#![deny(unsafe_code)]`

## 🚀 快速开始

### 构建

```bash
cargo build -p rmqtt-test --release
```

产物位于 `target/release/mqtt_harness`（Windows 下为 `mqtt_harness.exe`）。

### 运行全部测试（自动启动 Broker）

```bash
./target/release/mqtt_harness --workspace .
```

程序会自动查找 `target/release/rmqttd` 并启动 Broker。

### 连接已运行的 Broker

```bash
./target/release/mqtt_harness --no-broker
```

### 输出报告

```bash
# JSON 报告
./target/release/mqtt_harness --no-broker --json report.json

# HTML 报告
./target/release/mqtt_harness --no-broker --html report.html

# 同时输出两种格式
./target/release/mqtt_harness --no-broker --json report.json --html report.html
```

### 运行指定套件

```bash
# 单个套件
./target/release/mqtt_harness --workspace . --suites functional_v5
./target/release/mqtt_harness --workspace . --suites stress

# 多个套件（可多次使用 --suites 参数）
./target/release/mqtt_harness --workspace . --suites functional_v3 --suites functional_v311
```

## 📋 测试套件

### functional_v3（2 个用例）— MQTT 3.1

| 用例 | 说明 |
|------|------|
| `connect_v3` | MQTT 3.1 连接与断开 |
| `pubsub_v3_qos0` | QoS 0 发布/订阅 |

### functional_v311（10 个用例）— MQTT 3.1.1

| 用例 | 说明 |
|------|------|
| `connect_v311` | MQTT 3.1.1 连接与断开 |
| `connect_empty_client_id` | 空 Client ID 连接（需 Clean Session） |
| `multiple_connections` | 10 个并发连接 |
| `pubsub_v311_qos0` | QoS 0 发布/订阅 |
| `pubsub_v311_qos1` | QoS 1 发布/订阅 |
| `pubsub_v311_qos2` | QoS 2 发布/订阅（完整四步握手） |
| `retain_v311_message` | 保留消息存储与获取 |
| `unsubscribe_v311` | 取消订阅后不再收到消息 |
| `wildcard_plus` | 单层通配符 `+` 匹配 |
| `wildcard_hash` | 多层通配符 `#` 匹配 |

### functional_v5（5 个用例）— MQTT 5.0

| 用例 | 说明 |
|------|------|
| `connect_v5` | MQTT 5.0 连接与断开 |
| `connect_v5_reason_codes` | V5 连接 Reason Code 验证 |
| `pubsub_v5_qos0` | MQTT 5.0 QoS 0 发布/订阅 |
| `pubsub_v5_qos1` | MQTT 5.0 QoS 1 发布/订阅 |
| `pubsub_v5_qos2` | MQTT 5.0 QoS 2 发布/订阅 |

### stress（3 个用例）

| 用例 | 说明 |
|------|------|
| `connection_load` | N 客户端并发连接/断开（默认 100） |
| `publish_load` | 持续发布 1000 条 QoS 1 消息，统计 QPS |
| `fan_out` | 1 发布者 → N 订阅者扇出测试 |

### chaos（6 个用例）

| 用例 | 说明 |
|------|------|
| `chaos_broker_restart` | Broker 重启后客户端可重连 |
| `chaos_broker_restart_pubsub` | Broker 重启后 Pub/Sub 恢复 |
| `chaos_connection_churn` | 快速连接/断开循环 |
| `chaos_reconnect_storm` | 50 客户端同时连接风暴 |
| `chaos_qos1_reliability` | QoS 1 可靠性验证 |
| `chaos_slow_consumer` | 慢消费者场景 |

## 🏗 项目结构

```
rmqtt-test/
  src/
    main.rs                      # mqtt_harness 入口，套件注册
    broker/                      # Broker 生命周期管理
    mqtt/                        # 自研 MQTT 客户端（零第三方 MQTT 依赖）
      v3/                        # MQTT 3.1 客户端（QoS 0）
      v311/                      # MQTT 3.1.1 客户端（QoS 0/1/2）
      v5/                        # MQTT 5.0 客户端（QoS 0/1/2）
    transport/                   # 网络传输层
    framework/                   # 测试框架（TestCase, DAG 调度器, 上下文）
    tests/                       # 测试用例（功能测试、压测、混沌测试）
    report/                      # 报告系统（控制台、JSON、HTML、详细日志）
```

## 📄 许可证

MIT OR Apache-2.0
