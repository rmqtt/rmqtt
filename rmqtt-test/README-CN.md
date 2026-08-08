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

程序会自动查找 `target/release/rmqttd` 并启动 Broker，**默认使用自包含配置
`rmqtt-test/configs/default/rmqtt.toml`**（不依赖仓库根的 `rmqtt.toml` /
`rmqtt-plugins/*.toml`；保留 TCP/TLS/WS/WSS/QUIC 全部监听，便于后续添加
TLS/WS/QUIC 专项测试）。

### 使用其他 Broker 配置

```bash
# 显式指定配置文件（整批测试共用）
./target/release/mqtt_harness --workspace . --config rmqtt-test/configs/retain-disabled/rmqtt.toml

# 仅运行某个按配置拆分的子套件
./target/release/mqtt_harness --workspace . --suites functional_v5@retain-disabled
```

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

> `--suites` 支持前缀匹配：`functional_v5` 会同时命中其按配置拆出的所有子套件
> （如 `functional_v5@retain-disabled`）；`functional_v5_cluster` 双节点集群套件
> 仅当显式指定时才运行，不参与默认全量。

## ⚙️ Broker 配置（configs/ 自包含约定）

所有测试用 broker 配置均位于 `rmqtt-test/configs/<name>/`，**自包含**（主配置 +
自身 `plugins/` 子目录），不依赖仓库根的 `rmqtt.toml` / `rmqtt-plugins/*.toml`：

```
configs/
  default/                  # 默认配置（未指定 --config 时使用）
    rmqtt.toml              #   以仓库根 rmqtt.toml 为蓝本，保留全部 listener
    plugins/                #   retainer / shared-subscription / http-api
  retain-disabled/          # 不加载 retainer 插件（Retain Available = 0）
  pubrel-collision/         # 加载 message-storage（PUBREL 冲突复现）
  pubrel-collision-cluster/ # 双节点集群（手动启动，1884/1885 MQTT）
```

**按用例自动切换配置**：用例可通过 `TestCase::broker_config()` 声明所需配置
（如 `WillRetainRejectedWhenRetainUnavailableV5Test` 声明 `retain-disabled`、
`Qos2PubrelResumeCollisionTest` 声明 `pubrel-collision`）。构建套件时，
声明了同一配置的用例会被自动拆分为独立的 `{suite}@{config}` 子套件
（如 `functional_v5@retain-disabled`），调度器仅在 **suite 边界**切换配置
（重启 broker），默认配置组保持原名不变、零额外重启开销。

端口约束：参与自动切换的配置，`listener.tcp.external.addr` 必须与 harness 的
`--addr`（默认 `127.0.0.1:1883`）一致，否则健康检查无法通过。

## 📋 测试套件

### functional_v3（47 个用例）— MQTT 3.1

针对 MQTT v3.1（IBM MQIsdp）的规范符合性套件，覆盖正向、反向与边界场景：

| 类别 | 用例 |
|------|------|
| 连接 | `connect_v3` / `with_options` / `wrong_protocol_name` / `unsupported_level` / `reserved_flag` / `empty_clientid_cleansession0/1` / `long_client_id` / `client_id_max_length` |
| 发布/订阅 | `pubsub_v3_qos0/1/2` / `publish_v3_wildcard_reject` |
| QoS 2 一致性 | `qos2_replayed_publish_dedup_v3` [MQTT-4.3.3-10] / `qos2_pubrel_resend_on_resume_v3` [MQTT-4.4.0-1] |
| 保留消息 | `retain_v3_store_and_deliver` / `empty_payload_deletes` / `overwrite` / `live_message_not_retained` / `will` |
| 遗嘱消息 | `last_will_v3` / `clean` / `qos2` |
| Keep Alive | `keepalive_v3_ping` / `zero` / `timeout` |
| 会话 | `session_v3_persistent` / `clean` / `offline_queue` |
| 通配符 | `wildcard_v3_plus` / `hash` / `overlap` / `dollar_topics` / `case_sensitive` / `leading_slash` |
| 边界 | `boundary_v3_empty_payload` / `large_payload` / `long_topic` / `special_chars_topic` / `max_keepalive` / `rapid_subscribe` |
| 协议错误 | `protocol_error_v3_subscribe_qos3` / `publish_packet_id_zero` / `bad_remaining_length` / `empty_topic_filter` / `reserved_packet_type` / `subscribe_qos0_fixed_header` |

> v3.1 客户端通过 `build_connect_bytes` 手工构造 MQIsdp CONNECT 报文（codec 将协议级别硬编码为 4，对 3.1.1/5.0 正确）。

### functional_v311（64 个用例）— MQTT 3.1.1

| 类别 | 用例 |
|------|------|
| 连接 | `connect_v311` / `empty_client_id` / `multiple_connections` / `session_present_fresh` / `wrong_protocol_name` / `unsupported_level` / `reserved_flag` / `second_connect` [MQTT-3.1.0-2] / `long_client_id` |
| 发布/订阅 | `pubsub_v311_qos0/1/2` / `retain_v311_message` / `unsubscribe_v311` |
| QoS 2 一致性 | `qos2_replayed_publish_dedup_v311` [MQTT-4.3.3-10] / `qos2_pubrel_resend_on_resume_v311` [MQTT-4.4.0-1] / `qos2_duplicate_detection` |
| 保留消息 | `retain_v311_store_and_deliver` / `empty_payload_deletes` [MQTT-3.3.1-9] / `overwrite` / `live_message_not_retained` / `will` |
| 遗嘱消息 | `last_will_v311` / `clean` / `unclean` / `qos2` / `keepalive_timeout` |
| Keep Alive | `keepalive_v311_ping_keeps_alive` / `timeout` / `zero` / `max_value` |
| 会话 | `clean_session_false` / `offline_queue_v311` / `present_on_resume` [MQTT-3.2.2.1] / `clean_discard` [MQTT-3.1.2-6] |
| 通配符 | `wildcard_plus` / `hash` / `case_sensitive` / `leading_slash` / `hash_not_last` |
| 认证 / $SYS / 共享订阅 | `auth_empty_client_id_fail` / `auth_connect_disconnect_sequence` / `dollar_topics` / `shared_sub_v311` |
| 边界 | `max_client_id` / `long_topic` / `empty_payload` / `large_payload` / `special_chars_topic` / `rapid_subscribe` |
| 多主题 | `multi_topic_subscribe_v311` / `overlapping_subscriptions` / `message_ordering` |
| 协议错误 | `invalid_protocol_version` / `empty_topic_filter` / `protocol_error_v311_*`（订阅 QoS3、固定头 QoS、发布 QoS3/pid0、剩余长度、保留类型） |

### functional_v5（63 个用例）— MQTT 5.0

| 类别 | 用例 |
|------|------|
| 连接 / CONNACK | `connect_v5` / `reason_codes` / `session_present_fresh` / `wrong_protocol_name` / `unsupported_level` / `reserved_flag` / `second_connect` / `client_id_too_long` / `auth_method_rejected` (0x8C) / `connack_capabilities_v5` / `connack_receive_max_echo_v5` / `connack_assigned_client_id_v5` / `empty_clientid_cleanstart0_rejected` |
| 发布/订阅 | `pubsub_v5_qos0/1/2` |
| 会话 | `session_expiry_v5` / `takeover_v5` / `clean_start_v5` / `disconnect_expiry_zero` [MQTT-3.14.2-2] / `expiry_cleanup` |
| V5 特性 | `flow_control_v5` / `no_local_v5` / `will_delay_v5` / `shared_sub_v5` / `topic_alias_v5`（服务端/客户端/未知别名→0x94）/ `retain_handling_*_v5` / `retain_as_published_v5` / `server_keepalive_v5` / `max_packet_size_v5`（+ 强制）/ `subscribe_identifiers_v5` / `payload_format_v5` / `publication_expiry_v5` / `request_response_v5` / `user_properties_v5` / `wildcard_available_v5` |
| 保留消息 | `retain_v5_store_and_deliver` / `empty_payload_deletes` / `overwrite` / `live_message_not_retained` / `will` |
| QoS 2 | `qos2_replayed_publish_dedup` [MQTT-4.3.3-10] / `qos2_pubrel_resend_on_resume` [MQTT-4.4.0-1] / `qos2_pubrel_resume_collision` |
| 通配符 | `wildcard_v5_case_sensitive` / `leading_slash` |
| 协议错误 | `protocol_error_v5_*`（订阅 QoS3、固定头 QoS、发布 QoS3/pid0/空主题、剩余长度、保留类型） |
| 断开原因码 | `disconnect_reason_v5` |
| Will Retain vs Retain Available | `will_retain_rejected_when_retain_unavailable_v5`（在 `functional_v5@retain-disabled` 子套件中真正执行） |

> functional_v5 共 63 个用例：默认配置组运行其中 61 个；
> `will_retain_rejected_when_retain_unavailable_v5` 与 `qos2_pubrel_resume_collision`
> 因需要不同的 broker 配置，构建时自动拆分为 `functional_v5@retain-disabled` 与
> `functional_v5@pubrel-collision` 两个子套件执行（见上方「Broker 配置」章节）。

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
      v3/                        # MQTT 3.1 客户端（QoS 0/1/2，手工构造 MQIsdp CONNECT）
      v311/                      # MQTT 3.1.1 客户端（QoS 0/1/2）
      v5/                        # MQTT 5.0 客户端（QoS 0/1/2）
    transport/                   # 网络传输层（含 raw 字节发送，供负面测试使用）
    framework/                   # 测试框架（TestCase, DAG 调度器, 上下文）
    tests/                       # 测试用例（功能测试、压测、混沌测试）
      functional/                #   functional_v3/v311/v5 用例
      functional/qos2_pubrel_resume_collision_cluster.rs  # 集群复现用例
    report/                      # 报告系统（控制台、JSON、HTML、详细日志）
  configs/                       # 测试用 broker 配置（全部自包含）
    default/                     #   默认配置：rmqtt.toml + plugins/（retainer/shared-subscription/http-api）
    retain-disabled/             #   不加载 retainer 插件（Retain Available = 0）
    pubrel-collision/            #   单机：启用 message-storage 的 broker 配置
    pubrel-collision-cluster/    #   集群：node1/node2 双节点配置（1884/1885 MQTT、5364/5365 gRPC）
```

> **测试隔离说明**：所有发布保留消息的测试结束后会自行删除（空 payload + RETAIN=1）；
> `#` 通配符测试会先排空残留保留消息并以轮询方式过滤自己的 payload，因此各套件可
> 通过 `--workers N` 并发执行而不互相干扰。

## 📄 许可证

MIT OR Apache-2.0
