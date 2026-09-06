[English](README.md) | [**简体中文**](README-CN.md)

# rmqtt-test

[![crates.io page](https://img.shields.io/crates/v/rmqtt.svg)](https://crates.io/crates/rmqtt)
![Rust](https://img.shields.io/badge/rust-1.94%2B-blue)

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
  session-sled/             # 单机 sled 会话存储（issue #475 复现，harness 自动切换）
  session-sled-stress/      # 同上，独立 sled 路径（压测专用）
  cluster-broadcast-sled/   # 双节点集群（1886/1887 MQTT，测试自管理进程）
  cluster-broadcast-sled-stress/  # 同上，独立 sled 路径（压测专用）
  cluster-raft-sled/        # 三节点 raft 集群（1888/1889/1890 MQTT、6008-6010 raft）
  cluster-raft-sled-stress/ # 同上，独立 sled 路径（压测专用）
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

### functional_v3（51 个用例）— MQTT 3.1

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
| 协议错误 | `protocol_error_v3_subscribe_qos3` / `publish_packet_id_zero` / `publish_empty_topic` / `bad_remaining_length` / `empty_topic_filter`（订阅/取消订阅）/ `reserved_packet_type` / `subscribe_qos0_fixed_header` |

> v3.1 客户端通过 `build_connect_bytes` 手工构造 MQIsdp CONNECT 报文（codec 将协议级别硬编码为 4，对 3.1.1/5.0 正确）。

### functional_v311（108 个用例）— MQTT 3.1.1

| 类别 | 用例 |
|------|------|
| 连接 | `connect_v311` / `empty_client_id` / `multiple_connections` / `session_present_fresh` / `wrong_protocol_name` / `unsupported_level` / `reserved_flag` / `second_connect` [MQTT-3.1.0-2] / `long_client_id` / `client_id_65535` / `assigned_client_id` / `invalid_utf8_client_id` / `invalid_utf8_username` / `invalid_utf8_will_topic` / `username_flag_mismatch` / `password_flag_mismatch` / `will_flag_zero_but_qos_set` / `will_qos3` / `will_not_fire_on_rejected_connect` |
| 发布/订阅 | `pubsub_v311_qos0/1/2` / `publish_wildcard_reject` / `qos_downgrade_v311` / `ordering_qos2_v311` |
| QoS 2 / 恢复 | `qos2_replayed_publish_dedup_v311` [MQTT-4.3.3-10] / `qos2_pubrel_resend_on_resume_v311` [MQTT-4.4.0-1] / `qos2_duplicate_detection` / `qos1_publish_resend_on_resume_v311` / `qos2_broker_to_client_no_pubrec_v311` |
| 保留消息 | `retain_v311_store_and_deliver` / `empty_payload_deletes` [MQTT-3.3.1-9] / `overwrite` / `live_message_not_retained` / `live_publish_keeps_retained` / `will` / `restart_recovery` |
| 遗嘱消息 | `last_will_v311` / `qos0` / `qos1` / `qos2` / `clean` / `unclean` / `invalid_utf8_payload` / `keepalive_timeout` |
| Keep Alive | `keepalive_v311_ping_keeps_alive` / `timeout` / `zero` / `max_value` / `pingresp_explicit` / `window_boundary` |
| 会话 | `clean_session_false` / `offline_queue_v311` / `present_on_resume` [MQTT-3.2.2.1] / `clean_discard` [MQTT-3.1.2-6] / `takeover` / `tcp_fin_rst` |
| 通配符 | `wildcard_plus` / `hash` / `case_sensitive` / `leading_slash` / `hash_not_last` / `overlap` / `empty_levels` |
| 认证 / $SYS / 共享订阅 | `auth_empty_client_id_fail` / `auth_connect_disconnect_sequence` / `dollar_topics` / `shared_sub_v311` |
| 边界 | `max_client_id` / `long_topic` / `empty_payload` / `large_payload` / `special_chars_topic` / `rapid_subscribe` / `remaining_length_max` |
| 多主题 | `multi_topic_subscribe_v311` / `overlapping_subscriptions` / `message_ordering` |
| 协议错误 | `invalid_protocol_version` / `protocol_error_v311_*`（订阅/取消订阅：QoS3、QoS0 固定头、空 payload/filter、packet id 0；发布：QoS3、pid0、空主题、QoS0 携带 packet id；剩余长度非法、声明长度不匹配、报文截断、保留 packet type、packet type 15、PUBREL/PUBREC/PUBCOMP 错误 flags、未请求的 PUBREL、CONNECT payload 顺序、非法 UTF-8 主题）/ `remaining_length_transition_v311` |
| CONNACK 返回码（自管 broker） | `connack_return_codes_auth_http_v311`（auth-http + 用例内 mock，端口 1892）/ `connack_not_authorized_v311`（auth-jwt，端口 1893）——这两个用例自行拉起 broker，不使用 harness broker |

### functional_v5（99 个用例）— MQTT 5.0

| 类别 | 用例 |
|------|------|
| 连接 / CONNACK | `connect_v5` / `reason_codes` / `session_present_fresh` / `wrong_protocol_name` / `unsupported_level` / `reserved_flag` / `second_connect` / `client_id_too_long` / `auth_method_rejected` (0x8C) / `connack_capabilities_v5` / `connack_receive_max_echo_v5` / `connack_assigned_client_id_v5` / `assigned_clientid_v5` / `empty_clientid_cleanstart0_rejected` |
| 连接反向（🐞 expected-fail） | `connect_v5_will_flag_zero_but_qos_set` / `connect_v5_will_flag_zero_but_retain_set` [MQTT-3.1.2-11/12] — 已登记 broker 缺陷 |
| 发布/订阅 | `pubsub_v5_qos0/1/2` / `qos1_ordering` / `qos_downgrade_v5_matrix` / `publish_properties_passthrough_v5` |
| 会话 | `session_expiry_v5` / `takeover_v5` / `clean_start_v5` / `disconnect_expiry_zero` [MQTT-3.14.2-2] / `expiry_cleanup` / `expiry_update_on_reconnect` |
| V5 特性 | `flow_control_v5` / `flow_control_v5_inflight_cap_strict` / `no_local_v5` / `will_delay_v5` / `will_properties_v5_delivery` / `shared_sub_v5` / `shared_sub_v5_malformed_filter` / `topic_alias_v5`（服务端/客户端/未知别名→0x94、零→0x94、超上限→0x94）/ `retain_handling_*_v5` / `retain_as_published_v5` / `server_keepalive_v5` / `max_packet_size_v5`（+ 强制）/ `subscribe_identifiers_v5`（+ 更新）/ `subscribe_multi_filter_mixed_v5` / `payload_format_v5` / `publication_expiry_v5` / `message_expiry_v5_forwarded` / `message_expiry_v5_queued_drop` / `request_response_v5` / `request_problem_info_v5` / `user_properties_v5` / `wildcard_available_v5` |
| 流控反向（🐞 expected-fail） | `flow_control_v5_receive_max_violation` [MQTT-4.9.0-1/2] — 已登记 broker 缺陷（无 DISCONNECT 0x93） |
| 保留消息 | `retain_v5_store_and_deliver` / `empty_payload_deletes` / `overwrite` / `live_message_not_retained` / `will` |
| QoS 2 | `qos2_replayed_publish_dedup` [MQTT-4.3.3-10] / `qos2_pubrel_resend_on_resume` [MQTT-4.4.0-1] / `qos2_pubrel_resume_collision` |
| 通配符 | `wildcard_v5_case_sensitive` / `leading_slash` |
| 原因码（MAY 级） | `reason_code_v5_puback_no_matching_subscribers` / `reason_code_v5_unsuback_no_subscription` —— 断言合法原因码（0x00 或 0x10 / 0x11） |
| 响应/问题信息（info） | `connack_response_info_v5` / `publish_v5_response_topic_wildcard` —— 记录型观察，不计成败 |
| 协议错误 | `protocol_error_v5_*`（订阅/取消订阅：QoS3、QoS0 固定头、空 payload、packet id 0、sub-id 0、保留位、retain handling 3、取消订阅携带 sub-id；发布：QoS3、pid0、空主题、QoS0 携带 DUP；剩余长度非法、保留类型、DISCONNECT 错误 flags、非法 UTF-8 主题、未请求的 AUTH、User Property 非法 UTF-8） |
| 断开原因码 | `disconnect_reason_v5` |
| Keep Alive / TCP | `ping_v5` / `mqtt_keepalive_timeout_reclaims_tcp` / `tcp_keepalive_socket_option`（仅 Linux，其他平台跳过） |
| Will Retain vs Retain Available | `v5_will_retain_rejected_when_retain_unavailable`（在 `functional_v5@retain-disabled` 子套件中真正执行） |

> **expected-fail 用例（🐞）**：完整执行，但断言 broker 尚未实现的行为（已登记的
> 合规缺口）。失败记为 `EXPECTED-FAIL`，不计入套件失败；broker 合规后会浮出为
> `UNEXPECTED-PASS`，届时应转正为普通断言。详见
> `designs/mqtt-5.0-standalone-test-gap-analysis.md`。

> functional_v5 共 99 个用例：默认配置组运行其中 97 个；
> `v5_will_retain_rejected_when_retain_unavailable` 与 `qos2_pubrel_resume_collision`
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

### stress（6 个用例）

| 用例 | 说明 |
|------|------|
| `connection_load` | N 客户端并发连接/断开（默认 100） |
| `publish_load` | 持续发布 1000 条 QoS 1 消息，统计 QPS |
| `fan_out` | 1 发布者 → N 订阅者扇出测试 |
| `stress_mixed_qos_v311` | QoS 0/1/2 混合流量（v3.1.1 客户端） |
| `stress_subscription_mass` | 大量订阅建立与投递验证 |
| `stress_retain_flood` | 发布大量保留消息冲击 broker 内存 |

### chaos（18 个用例）

| 用例 | 说明 |
|------|------|
| `chaos_broker_restart` | Broker 重启后客户端可重连 |
| `chaos_broker_restart_pubsub` | Broker 重启后 Pub/Sub 恢复 |
| `chaos_connection_churn` | 快速连接/断开循环 |
| `chaos_reconnect_storm` | 50 客户端同时连接风暴 |
| `chaos_qos1_reliability` | QoS 1 可靠性验证 |
| `chaos_slow_consumer` | 慢消费者场景 |
| `session_storage_expired_cleanup` / `_edge` | 会话存储启动加载优化：过期离线会话在加载时被跳过（并删除），存活会话不受影响（含边界变体） |
| `chaos_broker_restart_session_routing` | issue #475 单机复现：持久会话从 sled 恢复后跨重启仍可路由（子套件 `chaos@session-sled`） |
| `cluster_restart_session_routing_broadcast` / `_raft` | 同缺陷经集群复现（broadcast 双节点 / raft 三节点），仅重启 node1 |
| `cluster_whole_restart_session_routing_broadcast` / `_raft` | 同缺陷经集群复现，全集群重启 |
| `stress_single_node_restart_session_routing` | issue #475 压测：1000 持久会话 × 100 条 QoS 1，单机重启（子套件 `chaos@session-sled-stress`） |
| `stress_cluster_restart_session_routing_broadcast` / `_raft` | 同压测经 cluster-broadcast / cluster-raft，仅重启 node1 |
| `stress_cluster_whole_restart_session_routing_broadcast` / `_raft` | 同压测经 cluster-broadcast / cluster-raft，全集群重启 |

#### issue #475 压测 — 执行方式

5 个压测将 issue #475 复现放大到 **1000 持久会话 × 100 条 QoS 1 消息（10 万条发布）**，
构建需 rustc ≥ 1.94：

```bash
RUSTUP_TOOLCHAIN=1.97 cargo build -p rmqttd
RUSTUP_TOOLCHAIN=1.97 cargo build -p rmqtt-test

# 清理上次运行的 sled 数据（sled 过大时 broker 启动很慢；harness 健康检查
# 超时已放宽到 60s 作兜底，但大量累积仍会显著拖慢启动，建议每次运行前清理）：
rm -rf rmqtt-test/configs/{session-sled,session-sled-stress,cluster-broadcast-sled,cluster-broadcast-sled-stress,cluster-raft-sled,cluster-raft-sled-stress}/.sled

# 全量 chaos（功能重启测试 + 全部 5 个压测，约 6.5 分钟）：
./target/debug/mqtt_harness --binary target/debug/rmqttd \
  --config rmqtt-test/configs/default/rmqtt.toml \
  --workspace . --suites chaos --workers 1

# 仅运行单机压测（约 25 秒）：
./target/debug/mqtt_harness --binary target/debug/rmqttd \
  --config rmqtt-test/configs/default/rmqtt.toml \
  --workspace . --suites chaos@session-sled-stress --workers 1
```

集群压测为测试自管理进程，注册在 `chaos` 主套件（无独立子套件）；节点日志在
`target/cluster-stress-{broadcast,raft,...}-node{1,2,3}.log`。规模常量
`STRESS_SESSIONS` / `STRESS_MSGS_PER_SESSION` 位于
`src/tests/functional/session_restart_stress.rs`。设计与缺陷分析详见
[`designs/issue-475-restored-session-routing-fix.md`](../designs/issue-475-restored-session-routing-fix.md)。

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
      functional/cluster_session_restart.rs  # issue #475 集群复现（broadcast/raft）
      functional/session_restart_stress.rs   # issue #475 压测（1000×100，5 场景）
    report/                      # 报告系统（控制台、JSON、HTML、详细日志）
  configs/                       # 测试用 broker 配置（全部自包含）
    default/                     #   默认配置：rmqtt.toml + plugins/（retainer/shared-subscription/http-api）
    retain-disabled/             #   不加载 retainer 插件（Retain Available = 0）
    pubrel-collision/            #   单机：启用 message-storage 的 broker 配置
    pubrel-collision-cluster/    #   集群：node1/node2 双节点配置（1884/1885 MQTT、5364/5365 gRPC）
    session-sled/                #   单机 sled 会话存储（issue #475 复现）
    session-sled-stress/         #   同上，独立 sled 路径（压测专用，避免污染复现测试）
    cluster-broadcast-sled/      #   双节点集群（1886/1887 MQTT、5366/5367 gRPC）
    cluster-broadcast-sled-stress/ # 同上，独立 sled 路径（压测专用）
    cluster-raft-sled/           #   三节点 raft 集群（1888/1889/1890 MQTT、5368-5370 gRPC、6008-6010 raft）
    cluster-raft-sled-stress/    #   同上，独立 sled 路径（压测专用）
```

> **测试隔离说明**：所有发布保留消息的测试结束后会自行删除（空 payload + RETAIN=1）；
> `#` 通配符测试会先排空残留保留消息并以轮询方式过滤自己的 payload，因此各套件可
> 通过 `--workers N` 并发执行而不互相干扰。

## 📄 许可证

MIT OR Apache-2.0
