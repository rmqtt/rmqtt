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

未指定 `--binary` 时，程序按 `target/release/rmqttd` → `target/debug/rmqttd`
的顺序在 workspace 根下自动查找并启动 Broker；需要固定某个构建产物时用
`--binary <path>` 显式指定。注意这是 **release 优先**的顺序——只要 release
产物存在就会优先选用，即使你启动的是 debug 版 harness，因此一个陈旧的
release 构建可能在你不知情的情况下成为实际被测对象。

**默认使用自包含配置 `rmqtt-test/configs/default/rmqtt.toml`**（不依赖仓库根的
`rmqtt.toml` / `rmqtt-plugins/*.toml`；保留 TCP/TLS/WS/WSS/QUIC 全部监听，
便于后续添加 TLS/WS/QUIC 专项测试）。

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

### 运行指定用例

```bash
# 单个用例（-t/--test 按用例名子串匹配，可重复）
./target/release/mqtt_harness --workspace . --suites functional_v5 -t will_delay_v5

# 多个用例
./target/release/mqtt_harness --workspace . --suites functional_v5 \
  -t will_published_on_disconnect_rc_0x04_v5 \
  -t will_not_published_on_disconnect_rc_0x00_v5

# 不指定 --suites：在所有套件里按名字搜，只跑命中的用例
./target/release/mqtt_harness --workspace . -t mqtt_keepalive

# 时序敏感用例：钉住 --workers 1，避免其他用例共用同一个 broker 造成扰动
# （issue #513 消息生命周期复现用例）
./target/release/mqtt_harness --workspace . --suites functional_v5 --workers 1 \
  -t retained_message_expiry_not_decremented_v5 \
  -t message_expiry_deletes_qos2_inflight_v5 \
  -t oversized_queued_message_stalls_queue_v5
```

> `-t/--test` 在套件/配置拆分之后生效，命中的用例仍在它声明的 broker 配置下运行，
> 过滤本身不会引入额外的配置切换；没有任何命中的套件会被整体丢弃。
>
> `--workers N`（默认 4）决定同时有多少个用例并行跑在**同一个** harness 托管的
> broker 上。依赖固定等待、连接断开检测、严格投递顺序的用例应显式钉
> `--workers 1`，否则被并发调度的其他用例可能扰动它们的时序。
> 上面三个用例即下方 `functional_v5` 表中登记的 issue #513 复现用例。

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
  auth-http-acl-fallthrough/ # issue #501：auth-http 404-ignore × rmqtt-acl 末条规则
                             #（自管 broker 1896 allow-all / 1900 deny-all，mock 用临时端口）
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

### functional_v311（110 个用例）— MQTT 3.1.1

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
| issue #501 认证 × ACL 穿透（自管 broker） | `auth_http_ignore_allow_all_acl_v311`（auth 服务返回 404 → 判定 'ignore'；acl `["allow", "all"]` 将其放行 → CONNACK 0x00 fail-open 复现，端口 1896）/ `auth_http_ignore_deny_all_acl_v311`（acl `["deny", "all"]` 兜底 → CONNACK 0x05 fail-closed，端口 1900） |

### functional_v5（108 个用例）— MQTT 5.0

| 类别 | 用例 |
|------|------|
| 连接 / CONNACK | `connect_v5` / `connect_v5_reason_codes` / `connect_v5_session_present_fresh` / `connect_v5_wrong_protocol_name` / `connect_v5_unsupported_level` / `connect_v5_reserved_flag` / `connect_v5_second_connect` / `connect_v5_client_id_too_long` / `connect_v5_auth_method_rejected` (0x8C) / `connack_capabilities_v5` / `connack_receive_max_echo_v5` / `connack_assigned_client_id_v5` / `assigned_clientid_v5` / `v5_empty_clientid_cleanstart0_rejected` |
| 连接反向（🐞 expected-fail） | `connect_v5_will_flag_zero_but_qos_set` / `connect_v5_will_flag_zero_but_retain_set` [MQTT-3.1.2-11/12] —— 已登记 broker 缺陷 |
| 发布/订阅 | `pubsub_v5_qos0` / `pubsub_v5_qos1` / `pubsub_v5_qos2` / `pubsub_v5_qos1_ordering` / `qos_downgrade_v5_matrix` / `publish_properties_passthrough_v5` |
| 会话 | `session_expiry_v5` / `session_takeover_v5` / `session_clean_start_v5` / `session_v5_disconnect_expiry_zero` [MQTT-3.14.2-2] / `session_v5_expiry_cleanup` / `session_v5_expiry_update_on_reconnect` |
| 拆除时的服务端 DISCONNECT | `takeover_sends_disconnect_0x8e_v5` [MQTT-3.1.4-3] —— **已修复并转为 PASS**（见下方说明）：v5 DISCONNECT 在 sink 写半端仍然打开时发出，被接管的客户端先收到 Reason Code 0x8E，之后才关闭连接 |
| 流控 | `flow_control_v5` / `flow_control_v5_inflight_cap_strict` |
| 流控反向（🐞 expected-fail） | `flow_control_v5_receive_max_violation` [MQTT-4.9.0-1/2] —— 已登记 broker 缺陷（无 DISCONNECT 0x93） |
| 主题别名 | `client_topic_alias_v5` / `server_topic_alias_v5` / `topic_alias_v5_unknown_alias`（→0x94）/ `topic_alias_v5_zero` / `topic_alias_v5_over_max` —— 后两条负向用例改用 QoS 1 发布，以 PUBACK 作为「被接受」的证据；`over_max` 当前 **FAIL**（见下方说明） |
| 共享订阅 | `shared_sub_v5` / `shared_sub_v5_malformed_filter` |
| 保留处理 | `retain_handling_new_v5` / `retain_handling_no_at_subscribe_v5` / `retain_as_published_v5` |
| 消息过期 | `publication_expiry_v5` / `message_expiry_v5_forwarded` / `message_expiry_v5_queued_drop` |
| 遗嘱 | `last_will_v5_fires` / `will_delay_v5` / `will_properties_v5_delivery` / `will_published_on_abrupt_close_v5` / `will_published_on_disconnect_rc_0x04_v5` / `will_published_on_disconnect_rc_not_0x00_v5` / `will_not_published_on_disconnect_rc_0x00_v5` |
| 报文大小 / Server Keep Alive | `max_packet_size_v5` / `max_packet_size_enforcement_v5` / `server_keepalive_v5` |
| 请求/响应与属性 | `request_response_v5` / `request_problem_info_v5` / `user_properties_v5` / `payload_format_v5` |
| 订阅选项 | `subscribe_identifiers_v5` / `subscribe_identifiers_v5_update` / `subscribe_multi_filter_mixed_v5` / `no_local_v5` / `wildcard_available_v5` |
| 保留消息 | `retain_v5_store_and_deliver` / `retain_v5_empty_payload_deletes` / `retain_v5_overwrite` / `retain_v5_live_message_not_retained` / `retain_v5_will` |
| QoS 2 | `qos2_replayed_publish_dedup` [MQTT-4.3.3-10] / `qos2_pubrel_resend_on_resume` [MQTT-4.4.0-1] / `qos2_pubrel_resume_collision` |
| 通配符 | `wildcard_v5_case_sensitive` / `wildcard_v5_leading_slash` |
| 原因码（MAY 级） | `reason_code_v5_puback_no_matching_subscribers` / `reason_code_v5_unsuback_no_subscription` —— 断言合法原因码（0x00 或 0x10 / 0x11） |
| 响应/问题信息（info） | `connack_response_info_v5` / `publish_v5_response_topic_wildcard` —— 记录型观察，不计成败 |
| 协议错误 | `protocol_error_v5_bad_remaining_length` / `protocol_error_v5_disconnect_bad_flags` / `protocol_error_v5_invalid_utf8_topic` / `protocol_error_v5_publish_dup_on_qos0` / `protocol_error_v5_publish_empty_topic` / `protocol_error_v5_publish_packet_id_zero` / `protocol_error_v5_publish_qos3` / `protocol_error_v5_reserved_packet_type` / `protocol_error_v5_retain_handling_3` / `protocol_error_v5_sub_id_zero` / `protocol_error_v5_sub_options_reserved_bits` / `protocol_error_v5_subscribe_empty_payload` / `protocol_error_v5_subscribe_packet_id_zero` / `protocol_error_v5_subscribe_qos0_fixed_header` / `protocol_error_v5_subscribe_qos3` / `protocol_error_v5_unsolicited_auth` / `protocol_error_v5_unsubscribe_empty_payload` / `protocol_error_v5_unsubscribe_packet_id_zero` / `protocol_error_v5_unsubscribe_qos0_fixed_header` / `protocol_error_v5_unsubscribe_with_sub_id` / `protocol_error_v5_user_property_bad_utf8` |
| 断开原因码 | `disconnect_reason_v5` |
| Keep Alive / TCP | `ping_v5` / `mqtt_keepalive_timeout_reclaims_tcp` / `tcp_keepalive_socket_option`（仅 Linux，其他平台跳过） |
| 消息生命周期（issue #513） | `retained_message_expiry_not_decremented_v5` [MQTT-3.3.2-6] —— **已修复并转为 PASS**（发出的 `Message Expiry Interval` 会扣除其在保留存储中的停留时间）；`message_expiry_deletes_qos2_inflight_v5` [MQTT-4.3.3-7] / [MQTT-4.4.0-1] —— **已修复并转为 PASS**（未完成的 QoS 2 交换不再被消息过期删除）；`oversized_queued_message_stalls_queue_v5` + `retained_oversized_message_keeps_session_v5` [MQTT-3.1.2-24/-25] —— 同一缺陷在「排队投递」与「保留消息」两条路径上的表现，**已修复并转为 PASS**；见下方说明 |
| Will Retain vs Retain Available | `v5_will_retain_rejected_when_retain_unavailable`（在 `functional_v5@retain-disabled` 子套件中真正执行） |

> **expected-fail 用例（🐞）**：完整执行，但断言 broker 尚未实现的行为（已登记的
> 合规缺口）。失败记为 `EXPECTED-FAIL`，不计入套件失败；broker 合规后会浮出为
> `UNEXPECTED-PASS`，届时应转正为普通断言。详见
> `designs/mqtt-5.0-standalone-test-gap-analysis.md`。

> functional_v5 共 108 个用例：默认配置组运行其中 106 个；
> `v5_will_retain_rejected_when_retain_unavailable` 与 `qos2_pubrel_resume_collision`
> 因需要不同的 broker 配置，构建时自动拆分为 `functional_v5@retain-disabled` 与
> `functional_v5@pubrel-collision` 两个子套件执行（见上方「Broker 配置」章节）。
> 全量 `--suites functional_v5 --workers 1` 运行的汇总为
> `Total: 108 | Passed: 101 | Failed: 1 | Skipped: 1 | ExpectedFail: 3 | Info: 2`
> —— 那一处失败就是下方登记的 `topic_alias_v5_over_max`。

> **issue #513 相关的四个用例刻意不标 expected-fail（🐞）。** 它们各自断言规范要求
> 的行为：保留消息的 `Message Expiry Interval` 必须扣除其在保留存储中的停留时间
> （[MQTT-3.3.2-6]）；收到 PUBREC 后欠下的 PUBREL 必须在会话恢复时用**原 Packet
> Identifier** 重发（[MQTT-4.4.0-1]）；超过客户端 Maximum Packet Size 的报文必须被丢弃，
> 并「视同已完成投递该应用消息」继续服务——排队路径与保留消息路径都如此
> （[MQTT-3.1.2-24] / [MQTT-3.1.2-25]）。因此缺陷修复前它们会作为普通失败显示，
> 修复后无需改动测试即转为 PASS——与上面那些不合规时保持静默的 🐞 用例不同。
> 四个现已全部 PASS：`message_expiry_deletes_qos2_inflight_v5`，因为
> `Session::reforward` 不再对 `UnComplete` 交换套用消息过期——PUBREC 已证明 PUBLISH
> 发出过，而 [MQTT-4.3.3-7] 从此禁止套用过期；两条超限报文用例，因为
> `EncodeError::OverMaxPacketSize` 已在 `Session::deliver`（`rmqtt/src/session.rs`）
> 中降级为「已完成投递」，不再逃出会话事件循环；以及
> `retained_message_expiry_not_decremented_v5`，因为 `_send_retain_messages` 不再把
> `publish.create_time` 改写成投递时刻，`message_expiry_check` 得以拿到原始接收
> 时间戳来扣除消息的等待时长。
> 请用 `--workers 1` 串行运行（见「运行指定用例」）。

> **`takeover_sends_disconnect_0x8e_v5` 同样刻意不标 expected-fail（🐞）—— 且现已 PASS。**
> 它复现的是排查 #513 时发现的**第四个独立缺陷**——属**连接拆除**环节，不属于消息生命周期。
> `Session::run` 原先先调用 `sink.close()`（关闭写半端），**之后**才构造并发送 v5
> DISCONNECT，而失败被 `let _ =` 丢弃。于是服务端能发出的所有 Reason Code——0x8D
> Keep Alive 超时、0x8E 会话被接管、0x93 超出接收上限、0x95 报文过大——都成了死码，
> 客户端无法区分「被服务端断开（附原因）」与「网络断开」。[MQTT-3.1.4-3] 规定会话被
> 接管时发送 0x8E DISCONNECT 是 MUST。修复内容：把 `sink.close()` 移回 DISCONNECT
> 之后；对「客户端自己已结束交互」的路径（客户端发来的 DISCONNECT、传输层关闭）不再回响
> DISCONNECT；发送失败改为记录日志而非静默丢弃；并把连接期接管（`Reason::ConnectKicked(false)`）
> 由原先错误返回的 0x87 Not Authorized 改为 0x8E Session taken over。用例把两层要求分开
> 断言，部分修复也能精确定位：对「发送 0x8E 后再关闭」的桩 broker 它 PASS，对「只修顺序、发
> 0x87」的桩 broker 它仅在原因码这一层失败——实际排查中正是这样抓出错误码值的。

> **`topic_alias_v5_over_max` 同样是裸 Failed，而非 🐞。** 它断言：Topic Alias 超过服务端
> 自己在 CONNACK 里广播的上限的 PUBLISH 不得被接受；而当前 broker **接受了它**——
> `ClientTopicAliases::set_and_get`（`rmqtt/src/types.rs`）只限制一条连接能登记**多少个**
> 别名，从不校验单个别名是否在上限之内，于是「上限 + 1」被直接登记、消息照常投递，用例读到的
> 正是证明这一点的 PUBACK。缺的是校验，不是原因码：[MQTT-3.3.2-9] 只写了「发送方 MUST NOT」，
> 但广播出去的上限本身就是服务端「我会认哪些别名」的承诺，而 0x94 Topic Alias invalid
> 正是为「主题别名非法」定义的原因码（MQTT 5.0 第 4.13.1 节覆盖了「先告知再关闭」）。
> 这条用例与 `topic_alias_v5_zero` 原先都用 QoS 0 发布，而 QoS 0 本就不期待任何应答；再加上
> 判定把「读超时」当成「连接已关闭」，于是无论 broker 怎么处理都会 PASS——实测 `over_max`
> 耗时 5.0s，正好是整个读超时。现改为 QoS 1 发布，让 PUBACK 成为「被接受」的唯一证据，
> 连接仍开着却读超时也算失败。`topic_alias_v5_zero` 仍 PASS（broker 确实会及时拒绝别名 0），
> 留下的那一处红就是 `topic_alias_v5_over_max`。

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
    auth-http-acl-fallthrough/   #   issue #501 复现：auth-http（临时端口 mock，恒回 404）+
                                 #   rmqtt-acl；自管 broker 1896/5376（allow-all）与
                                 #   1900/5377（deny-all）
```

> **测试隔离说明**：所有发布保留消息的测试结束后会自行删除（空 payload + RETAIN=1）；
> `#` 通配符测试会先排空残留保留消息并以轮询方式过滤自己的 payload，因此各套件可
> 通过 `--workers N` 并发执行而不互相干扰。

## 📄 许可证

MIT OR Apache-2.0
