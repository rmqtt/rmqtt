[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-test

[![crates.io page](https://img.shields.io/crates/v/rmqtt.svg)](https://crates.io/crates/rmqtt)
![Rust](https://img.shields.io/badge/rust-1.89%2B-blue)

RMQTT 的工业级验证与压测核心引擎（Test Harness + Chaos + Benchmark）。编译产物 `mqtt_harness` 作为独立可执行程序，提供功能测试、压力测试、混沌测试，并输出结构化测试报告。

## ✨ 特性

- **自研 MQTT 客户端** — 零第三方 MQTT 依赖，完整实现 MQTT 3.1 / 3.1.1 / 5.0 协议栈
- **Broker 生命周期管理** — 自动启动/停止/重启 rmqttd 进程，TCP 健康检查
- **六类测试套件** — functional_v3 / functional_v311 / functional_v5 / functional_v5_cluster / stress / chaos
- **QoS 全覆盖** — QoS 0 / QoS 1 / QoS 2（含完整四步握手）正确性验证
- **并发缺陷复现** — QoS 2 会话恢复时 PUBREL 重发与存储消息的 packet-id 冲突（单元级 + 集群端到端）
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

### functional_v5（34 个用例）— MQTT 5.0

| 用例 | 说明 |
|------|------|
| `connect_v5` / `connect_v5_reason_codes` | MQTT 5.0 连接与 Reason Code 验证 |
| `pubsub_v5_qos0/1/2` | MQTT 5.0 QoS 0/1/2 发布/订阅 |
| `session_expiry_v5` / `session_takeover_v5` / `session_clean_start_v5` | 会话过期 / 接管 / Clean Start |
| `qos2_replayed_publish_dedup` | [MQTT-4.3.3-10] 重放 QoS 2 PUBLISH 去重（issue #456） |
| `qos2_pubrel_resend_on_resume` | [MQTT-4.4.0-1] 会话恢复时重发欠的 PUBREL（issue #456） |
| `qos2_pubrel_resume_collision` | **PUBREL 重发与并发 deliver 的 packet-id 冲突（单机版回归测试）** |
| `flow_control_v5` / `no_local_v5` / `will_delay_v5` | 流控 / 本地不转发 / Will 延迟 |
| `retain_handling_*_v5` / `shared_sub_v5` / `topic_alias_v5` 等 | V5 特性覆盖（详见 `src/tests/functional/`） |

### functional_v5_cluster（1 个用例）— 双节点集群端到端复现

| 用例 | 说明 |
|------|------|
| `qos2_pubrel_resume_collision_cluster` | 集群路径端到端复现 packet-id 冲突：远端投递不标记存储 → 会话跨节点恢复时存储消息与 PUBREL 重发抢 id |

该套件**需要手动启动双节点**（默认全量运行不会包含它，避免污染单机测试）：

```bash
# 终端 1 / 终端 2：启动两个节点
./target/release/rmqttd -f rmqtt-test/configs/pubrel-collision-cluster/node1/rmqtt.toml
./target/release/rmqttd -f rmqtt-test/configs/pubrel-collision-cluster/node2/rmqtt.toml

# 终端 3：运行集群复现套件
./target/release/mqtt_harness --no-broker --addr 127.0.0.1:1884 --suites functional_v5_cluster --workers 1
```

> 该测试修复前 3/3 轮复现 BUG（重复 PUBREL）；修复后 3/3 轮 PASS。修复方案详见
> [`designs/pubrel-resume-inflight-id-collision.md`](../designs/pubrel-resume-inflight-id-collision.md)。

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
      functional/                #   functional_v3/v311/v5 用例
      functional/qos2_pubrel_resume_collision_cluster.rs  # 集群复现用例
    report/                      # 报告系统（控制台、JSON、HTML、详细日志）
  configs/                       # 测试用 broker 配置
    pubrel-collision/            #   单机：启用 message-storage 的 broker 配置
    pubrel-collision-cluster/    #   集群：node1/node2 双节点配置（1884/1885 MQTT、5364/5365 gRPC）
```

## 📄 许可证

MIT OR Apache-2.0
