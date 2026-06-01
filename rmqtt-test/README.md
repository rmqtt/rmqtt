[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-test

[![crates.io page](https://img.shields.io/crates/v/rmqtt.svg)](https://crates.io/crates/rmqtt)
![Rust](https://img.shields.io/badge/rust-1.89%2B-blue)

**rmqtt-test** is the industrial-grade test harness, chaos engineering, and benchmarking engine for the [RMQTT](https://github.com/rmqtt/rmqtt) MQTT broker.

The build artifact `mqtt_harness` is a standalone executable that provides functional testing, stress testing, chaos testing, and outputs structured test reports.

## ✨ Features

- **Custom MQTT Client** — Zero third-party MQTT dependency, complete MQTT 3.1 / 3.1.1 / 5.0 protocol stack
- **Broker Lifecycle Management** — Auto start/stop/restart `rmqttd` process with TCP health checks
- **Five Test Suites** — `functional_v3` / `functional_v311` / `functional_v5` / `stress` / `chaos`
- **Full QoS Coverage** — QoS 0 / QoS 1 / QoS 2 (including full 4-step handshake) correctness verification
- **Chaos Injection** — Broker restart, connection storms, slow consumers, packet loss simulation
- **Multi-Format Reports** — Console + JSON + HTML
- **DAG Scheduling** — Topological sort of test case dependencies with timeout and retry
- **Detailed Diagnostic Logs** — Automatic failure reason logging with diagnostic hints; MQTT packet-level hex tracing
- **100% Safe Rust** — `#![deny(unsafe_code)]`

## 🚀 Quick Start

### Build

```bash
cargo build -p rmqtt-test --release
```

Artifact located at `target/release/mqtt_harness` (`mqtt_harness.exe` on Windows).

### Run All Tests (Auto-Start Broker)

```bash
./target/release/mqtt_harness --workspace .
```

The program will auto-locate `target/release/rmqttd` and start the broker.

### Connect to a Running Broker

```bash
./target/release/mqtt_harness --no-broker
```

### Generate Reports

```bash
# JSON report
./target/release/mqtt_harness --no-broker --json report.json

# HTML report
./target/release/mqtt_harness --no-broker --html report.html

# Both formats
./target/release/mqtt_harness --no-broker --json report.json --html report.html
```

### Running Specific Suites

```bash
# Single suite
./target/release/mqtt_harness --workspace . --suites functional_v5
./target/release/mqtt_harness --workspace . --suites stress

# Multiple suites
./target/release/mqtt_harness --workspace . --suites functional_v3 --suites functional_v311
```

## 📋 Test Suites

### `functional_v3` (2 cases) — MQTT 3.1

| Case | Description |
|------|-------------|
| `connect_v3` | MQTT 3.1 connect/disconnect |
| `pubsub_v3_qos0` | QoS 0 publish/subscribe |

### `functional_v311` (10 cases) — MQTT 3.1.1

| Case | Description |
|------|-------------|
| `connect_v311` | MQTT 3.1.1 connect/disconnect |
| `connect_empty_client_id` | Empty Client ID connection (Clean Session) |
| `multiple_connections` | 10 concurrent connections |
| `pubsub_v311_qos0` | QoS 0 publish/subscribe |
| `pubsub_v311_qos1` | QoS 1 publish/subscribe |
| `pubsub_v311_qos2` | QoS 2 publish/subscribe (full 4-step handshake) |
| `retain_v311_message` | Retained message storage and retrieval |
| `unsubscribe_v311` | No message received after unsubscription |
| `wildcard_plus` | Single-level wildcard `+` matching |
| `wildcard_hash` | Multi-level wildcard `#` matching |

### `functional_v5` (5 cases) — MQTT 5.0

| Case | Description |
|------|-------------|
| `connect_v5` | MQTT 5.0 connect/disconnect |
| `connect_v5_reason_codes` | V5 Reason Code verification |
| `pubsub_v5_qos0` | Qos 0 publish/subscribe |
| `pubsub_v5_qos1` | QoS 1 publish/subscribe |
| `pubsub_v5_qos2` | QoS 2 publish/subscribe |

### `stress` (3 cases)

| Case | Description |
|------|-------------|
| `connection_load` | N concurrent client connect/disconnect (default 100) |
| `publish_load` | Continuous publish 1000 QoS 1 messages, QPS statistics |
| `fan_out` | 1 publisher → N subscribers fan-out test |

### `chaos` (6 cases)

| Case | Description |
|------|-------------|
| `chaos_broker_restart` | Client reconnection after broker restart |
| `chaos_broker_restart_pubsub` | Pub/Sub recovery after broker restart |
| `chaos_connection_churn` | Rapid connect/disconnect cycling |
| `chaos_reconnect_storm` | 50 concurrent connection storms |
| `chaos_qos1_reliability` | QoS 1 reliability verification |
| `chaos_slow_consumer` | Slow consumer scenario |

## 🏗 Project Structure

```
rmqtt-test/
  src/
    main.rs                      # mqtt_harness entry point, suite registration
    broker/                      # Broker lifecycle management
    mqtt/                        # Custom MQTT client (zero external MQTT deps)
      v3/                        # MQTT 3.1 client (QoS 0)
      v311/                      # MQTT 3.1.1 client (QoS 0/1/2)
      v5/                        # MQTT 5.0 client (QoS 0/1/2)
    transport/                   # Network transport layer
    framework/                   # Test framework (TestCase, DAG scheduler, context)
    tests/                       # Test cases (functional, stress, chaos)
    report/                      # Report system (console, JSON, HTML, detail log)
```

## 📄 License

MIT OR Apache-2.0
