# rmqtt-test

RMQTT 的工业级验证与压测核心引擎（Test Harness + Chaos + Benchmark）。

编译产物 `mqtt_harness` 作为独立可执行程序，提供功能测试、压力测试、混沌测试，并输出结构化测试报告。

## 特性

- **自研 MQTT 客户端** — 零第三方 MQTT 依赖，完整实现 MQTT 3.1 / 3.1.1 / 5.0 协议栈
- **Broker 生命周期管理** — 自动启动/停止/重启 rmqttd 进程，TCP 健康检查
- **五类测试套件** — functional_v3 / functional_v311 / functional_v5 / stress / chaos
- **QoS 全覆盖** — QoS 0 / QoS 1 / QoS 2（含完整四步握手）正确性验证
- **混沌注入** — Broker 重启、连接风暴、慢消费者、丢包模拟
- **多格式报告** — Console + JSON + HTML
- **DAG 调度** — 测试用例依赖关系拓扑排序，超时与重试机制
- **详细诊断日志** — 失败测试自动记录原因与诊断提示；MQTT 包级十六进制跟踪
- **100% Safe Rust** — `#![deny(unsafe_code)]`

## 测试套件名

可通过 `--suites` 参数按名称运行指定套件：

| `functional_v3` | `functional_v311` | `functional_v5` | `stress` | `chaos` |
|:---:|:---:|:---:|:---:|:---:|

```bash
# 运行单个套件
./target/release/mqtt_harness --workspace . --suites functional_v3
./target/release/mqtt_harness --workspace . --suites functional_v311
./target/release/mqtt_harness --workspace . --suites functional_v5
./target/release/mqtt_harness --workspace . --suites stress
./target/release/mqtt_harness --workspace . --suites chaos

# 运行多个套件（逗号分隔）
./target/release/mqtt_harness --workspace . --suites functional_v3,functional_v311
./target/release/mqtt_harness --no-broker --suites stress,chaos

# 不指定则运行全部
./target/release/mqtt_harness --workspace .
```

## 快速开始

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

## CLI 参数

```
mqtt_harness - RMQTT Industrial-Grade Test Harness

USAGE:
    mqtt_harness [FLAGS] [OPTIONS]

FLAGS:
        --no-broker    不启动/停止 Broker（假定已运行）
    -v, --verbose      详细输出（控制台显示 debug 级别日志）

OPTIONS:
    -a, --addr <ADDR>                    Broker 地址 [default: 127.0.0.1:1883]
    -b, --binary <BINARY>                rmqttd 二进制路径
    -c, --config <CONFIG>                rmqtt.toml 配置文件路径
        --workspace <WORKSPACE>          Workspace 根目录（用于定位 rmqttd）
    -s, --suites <SUITES>...             指定测试套件（逗号分隔）
    -w, --workers <WORKERS>              并行 Worker 数 [default: 4]
        --json <JSON>                    输出 JSON 报告到文件
        --html <HTML>                    输出 HTML 报告到文件
        --log-file <LOG-FILE>            测试详细日志文件 [default: test-detail.log]
        --stress-clients <N>             压测客户端数 [default: 100]
        --chaos-iterations <N>           混沌测试迭代次数 [default: 5]
```

环境变量 `RUST_LOG` 可覆盖控制台日志级别（默认 `info`）：

```bash
RUST_LOG=debug ./target/release/mqtt_harness --no-broker
```

## 测试套件

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

## 日志与诊断

每次运行会自动生成两个日志文件，便于排查失败原因。

### 测试详细日志（`--log-file`，默认 `test-detail.log`）

结构化的逐测试记录，包含：

- 总体摘要（通过/失败/超时/错误数量与耗时）
- 每个测试的状态、耗时、重试次数
- **失败原因**：完整的错误信息
- **诊断提示**：根据错误模式自动分析可能原因，例如：
  - `connection closed by broker` → 提示检查 SUBSCRIBE 包编码是否多了 V5 属性长度字节
  - `timeout` → 提示可能 Broker 未响应或客户端解码有误
- **失败汇总**：末尾汇总所有失败测试，方便快速定位

```
───────────────────────────────────────────────────────────────
TEST: pubsub_v311_qos0 [functional_v311]
───────────────────────────────────────────────────────────────
  Status:   FAILED
  Duration: 0.002s

  FAILURE REASON:
    connection closed by broker

  DIAGNOSTIC HINTS:
    - The broker actively disconnected the client.
    - Common causes:
      * Malformed MQTT packet (check packet encoding)
      * V5 property length byte sent in v3.1.1 SUBSCRIBE/UNSUBSCRIBE
      * Invalid topic filter in SUBSCRIBE
      * Protocol violation (e.g., wrong packet order)
    - To debug: run with RUST_LOG=debug to see packet hex traces
```

### MQTT 协议包跟踪（`test-trace.log`）

自动生成的 debug 级别日志，记录所有 MQTT 包的收发细节：

- 每个包的 **SEND/RECV** 方向、包类型
- 完整的**十六进制字节转储**（最多显示前 64 字节）
- 按时间顺序排列，可追溯完整的协议交互过程

```
DEBUG mqtt_harness::transport::tcp_v3: SEND packet=SUBSCRIBE bytes=82 16 00 02 00 00 10 74 65 73 74 2f 70 75 62 73 75 62 2f 71 6f 73 30 00
DEBUG mqtt_harness::transport::tcp_v3: RECV 4 bytes
DEBUG mqtt_harness::transport::tcp_v3: DECODED packet=SUBACK
```

> **提示**：控制台默认只显示 info 级别日志。使用 `-v` 或 `RUST_LOG=debug` 可在控制台也看到包跟踪。

## 测试报告

### Console 输出示例

```
============================================================
  RMQTT Test Harness - Results
============================================================

 ✔ connect_v3 [52ms]
 ✔ pubsub_v3_qos0 [48ms]
 ✔ connect_v311 [52ms]
 ✔ connect_empty_client_id [48ms]
 ✔ multiple_connections [312ms]
 ✔ pubsub_v311_qos0 [112ms]
 ✔ pubsub_v311_qos1 [135ms]
 ✔ pubsub_v311_qos2 [189ms]
 ✔ retain_v311_message [98ms]
 ✔ unsubscribe_v311 [156ms]
 ✔ wildcard_plus [104ms]
 ✔ wildcard_hash [98ms]
 ✔ connect_v5 [48ms]
 ✔ connect_v5_reason_codes [52ms]
 ✔ pubsub_v5_qos0 [112ms]
 ✔ pubsub_v5_qos1 [135ms]
 ✔ pubsub_v5_qos2 [189ms]
 ✔ connection_load [3241ms]
 ✔ publish_load [5210ms]
 ✔ fan_out [1523ms]
 ✔ chaos_broker_restart [2100ms]
 ✔ chaos_broker_restart_pubsub [3100ms]
 ✔ chaos_connection_churn [4200ms]
 ✔ chaos_reconnect_storm [1800ms]
 ✔ chaos_qos1_reliability [2100ms]
 ✔ chaos_slow_consumer [1500ms]

------------------------------------------------------------
  Total: 26 | Passed: 26 | Failed: 0 | Errors: 0 | Timeouts: 0 | Skipped: 0
  Duration: 12.34s
============================================================
```

### JSON 报告结构

```json
{
  "suite": "rmqtt-test",
  "summary": {
    "total": 26,
    "passed": 26,
    "failed": 0,
    "skipped": 0,
    "errors": 0,
    "timeouts": 0,
    "duration_ms": 12340
  },
  "cases": [
    {
      "name": "connect_v3",
      "suite": "functional_v3",
      "status": "passed",
      "reason": null,
      "duration_ms": 52,
      "retries": 0
    }
  ]
}
```

### HTML 报告

深色主题，包含结果表格和汇总统计，可直接在浏览器中打开。

## 自研 MQTT 客户端

rmqtt-test 内置完整 MQTT 协议栈，不依赖任何外部 MQTT crate：

- **传输层**：基于 `tokio::net::TcpStream` 的异步 TCP 读写，读写半部独立拆分避免锁竞争
- **编解码**：基于 `rmqtt-codec` crate 的完整 v3.1 / v3.1.1 / v5.0 协议编解码
- **数据包**：CONNECT / CONNACK / PUBLISH / PUBACK / PUBREC / PUBREL / PUBCOMP / SUBSCRIBE / SUBACK / UNSUBSCRIBE / UNSUBACK / PINGREQ / PINGRESP / DISCONNECT / AUTH (v5)
- **QoS 状态机**：QoS 0（至多一次）、QoS 1（至少一次，PUBACK 自动确认）、QoS 2（恰好一次，PUBREC → PUBREL → PUBCOMP 自动应答）
- **MQTT 5.0 扩展**：Session Expiry Interval、Message Expiry Interval、User Properties、Reason Codes
- **协议合规**：Reader loop 自动发送 PUBACK/PUBREC/PUBCOMP，确保 broker inflight 窗口不会阻塞

## 扩展测试

实现 `TestCase` trait 即可添加自定义测试：

```rust
use rmqtt_test::framework::testcase::{TestCase, TestResult};
use rmqtt_test::framework::context::TestContext;

struct MyCustomTest;

impl TestCase for MyCustomTest {
    fn name(&self) -> &str { "my_custom_test" }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = std::time::Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let mut client = ctx.create_client("custom-test");
            client.connect().await?;
            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });
        match result {
            Ok(()) => TestResult::passed(self.name(), "custom", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "custom", start.elapsed(), e.to_string()),
        }
    }
}
```

> **注意**：每个测试用例需自行创建 `tokio::runtime::Runtime`（而非使用 `Handle::current()`），因为调度器运行在同步上下文中。

## 项目结构

```
rmqtt-test/
  Cargo.toml
  src/
    main.rs                      # mqtt_harness 入口，套件注册

    broker/                      # Broker 生命周期管理
      mod.rs
      lifecycle.rs              # start / stop / restart / kill
      healthcheck.rs            # TCP 健康检查

    mqtt/                        # 自研 MQTT 客户端（零第三方 MQTT 依赖）
      mod.rs
      common/
        mod.rs                   # QoS 类型、协议常量
        session.rs               # Packet ID 计数器
      v3/
        mod.rs                   # MQTT 3.1 客户端（QoS 0）
        client.rs
      v311/
        mod.rs                   # MQTT 3.1.1 客户端（QoS 0/1/2）
        client.rs
      v5/
        mod.rs                   # MQTT 5.0 客户端（QoS 0/1/2）
        client.rs

    transport/                    # 网络传输层（读写半部独立拆分）
      mod.rs
      tcp_v3.rs                  # v3/v3.1.1 TCP transport
      tcp_v5.rs                  # v5.0 TCP transport

    framework/                   # 测试框架
      mod.rs
      testcase.rs                # TestCase trait / TestResult / TestVerdict
      scheduler.rs               # DAG 调度、串行/并行、超时、重试
      context.rs                  # TestContext（Broker 句柄 / 客户端工厂 / 指标）
      suite.rs                    # TestSuite 分组

    tests/                        # 测试用例
      mod.rs
      functional/
        connect_v3.rs             # functional_v3
        pubsub_v3.rs
        connect_v311.rs           # functional_v311
        pubsub_v311.rs
        wildcard.rs
        connect_v5.rs             # functional_v5
        pubsub_v5.rs
      stress/
        load_v311.rs
        fanout.rs
      chaos/
        restart.rs
        disconnect.rs
        packet_loss.rs

    report/                       # 报告系统
      mod.rs
      json.rs                     # JSON 报告
      console.rs                  # 控制台输出
      html.rs                     # HTML 报告
      detail_log.rs               # 测试详细诊断日志
```

## 依赖

| Crate | 用途 |
|-------|------|
| tokio | 异步运行时 |
| bytes / bytestring | 网络字节处理 |
| rmqtt-codec | MQTT 协议编解码 |
| serde / serde_json | 序列化与 JSON 报告 |
| clap | CLI 参数解析 |
| tracing / tracing-subscriber | 日志（含文件输出与分层过滤） |
| anyhow / thiserror | 错误处理 |
| rand | 混沌测试随机数 |
| uuid | 客户端 ID 生成 |

## 许可证

MIT OR Apache-2.0
