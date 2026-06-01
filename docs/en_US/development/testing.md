[**English**](testing.md) | [简体中文](../zh_CN/development/testing.md)

# RMQTT Testing Guide

This document describes the RMQTT testing strategy, test layers, and how to run and extend the test suite.

---

## Test Layers

RMQTT has three testing layers:

```mermaid
graph TD
    subgraph "Layer 1: Unit Tests"
        UT1[rmqtt-codec tests<br/>v3/v5 encode/decode]
        UT2[rmqtt-net tests<br/>builder, stream]
        UT3[rmqtt-utils tests<br/>Bytesize, NodeAddr, parse]
        UT4[Other crate tests<br/>#\[cfg(test)\] modules]
    end

    subgraph "Layer 2: Integration Tests"
        IT1[mqtt_harness<br/>5 suite types]
        IT2[functional_v3/311/v5<br/>Protocol compliance]
        IT3[stress<br/>Load & performance]
        IT4[chaos<br/>Fault injection]
    end

    subgraph "Layer 3: Interoperability"
        IP1[paho.mqtt.testing<br/>V3.1.1: 11 tests]
        IP2[paho.mqtt.testing<br/>V5.0: 24 tests]
    end

    UT1 --> IT1
    UT2 --> IT1
    UT3 --> IT1
    IT1 --> IP1
    IT1 --> IP2
```

---

## Layer 1: Unit Tests

Each crate contains standard `#[cfg(test)]` modules alongside the production code.

### Running Unit Tests

```bash
# All unit tests
cargo test

# Tests for a specific crate
cargo test -p rmqtt-codec

# Tests matching a name pattern
cargo test -p rmqtt-codec -- qos

# Run without parallel execution (helpful for debugging)
cargo test -- --test-threads=1

# Show output of passing tests
cargo test -- --nocapture
```

### Where to Find Unit Tests

| Crate | Test Focus | Key Test Files |
|-------|-----------|----------------|
| `rmqtt-codec` | MQTT encode/decode round-trips, version detection, QoS flows | `src/v3/packet.rs`, `src/v5/packet.rs`, `tests/` |
| `rmqtt-net` | Builder defaults, listener binding, protocol upgrade guards | `src/builder.rs` |
| `rmqtt-conf` | CLI argument parsing, default values | `src/options.rs` |
| `rmqtt-utils` | Bytesize parsing, duration parsing, NodeAddr, Counter | `src/lib.rs`, `src/counter.rs` |
| `rmqtt` | Session management, inflight tracking, subscription trie | Various modules |
| `rmqtt-macros` | Macro expansion correctness | `src/metrics.rs`, `src/plugin.rs` |

---

## Layer 2: Integration Test Harness

The `rmqtt-test` crate provides a comprehensive test harness named `mqtt_harness`. It is a standalone binary that starts an RMQTT broker, runs test suites against it, and outputs structured reports.

### Building the Test Harness

```bash
# Build release binary (required — the harness runs rmqttd)
cargo build --release

# Build the test harness
cargo build -p rmqtt-test --release
```

### Running Test Suites

```bash
# Run all test suites (auto-starts broker)
./target/release/mqtt_harness --workspace .

# Run specific suites
./target/release/mqtt_harness --workspace . --suites functional_v5
./target/release/mqtt_harness --workspace . --suites stress --suites chaos

# Connect to an already-running broker
./target/release/mqtt_harness --no-broker

# Generate reports
./target/release/mqtt_harness --workspace . --json report.json --html report.html
```

### Test Suite Reference

| Suite | Cases | What It Tests |
|-------|-------|---------------|
| `functional_v3` | 2 | MQTT 3.1 connect/disconnect, QoS 0 pub/sub |
| `functional_v311` | 10 | MQTT 3.1.1 protocol compliance (connect, QoS 0/1/2, retain, wildcards, unsubscribe, empty client ID) |
| `functional_v5` | 5 | MQTT 5.0 protocol compliance (connect, reason codes, QoS 0/1/2) |
| `stress` | 3 | Connection load (100 clients), publish QPS (1000 msgs), fan-out (1→N) |
| `chaos` | 6 | Broker restart, connection churn, reconnect storm, QoS 1 reliability, slow consumer |

### Test Case Architecture

Each test case implements the `TestCase` trait:

```rust
pub trait TestCase: Send + Sync {
    fn name(&self) -> &str;
    fn suite(&self) -> &str;  // functional_v3, stress, etc.
    fn execute(&self, ctx: &mut TestContext) -> TestResult;
}
```

The test scheduler uses DAG-based dependency ordering with timeout and retry support.

### Custom MQTT Client

The test harness includes a custom MQTT client implementation with zero third-party MQTT dependencies:

- **Transport**: Async TCP via `tokio::net::TcpStream`
- **Codec**: Based on `rmqtt-codec` for v3/v5 protocol
- **QoS State Machine**: Automatic PUBACK/PUBREC/PUBCOMP handling
- **Packet Types**: All standard MQTT packet types supported

---

## Layer 3: Interoperability Testing

RMQTT is tested against the [paho.mqtt.testing](https://github.com/eclipse/paho.mqtt.testing) suite.

### Setup

```bash
git clone https://github.com/eclipse/paho.mqtt.testing.git
cd paho.mqtt.testing
```

### MQTT v3.1.1 Tests

```bash
# Start broker in one terminal
./target/release/rmqttd

# In another terminal
cd paho.mqtt.testing/interoperability
python client_test.py
```

All 11 tests pass.

### MQTT v5.0 Tests

```bash
cd paho.mqtt.testing/interoperability
python client_test5.py
```

All 24 tests pass.

**Note**: For the `test_subscribe_failure` and `test_server_keep_alive` tests, minor configuration adjustments are needed — see the test output for details.

---

## Performance Benchmarking

### Environment

The test harness supports stress testing out of the box:

```bash
# Connection load test (100 clients)
./target/release/mqtt_harness --no-broker --suites stress \
  --stress-clients 100

# Custom client count
./target/release/mqtt_harness --no-broker --suites stress \
  --stress-clients 10000
```

For comprehensive benchmarking, see the [Benchmark Testing Guide](../benchmark-testing.md).

---

## Writing New Tests

### Adding a Unit Test

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_my_feature() {
        let result = my_function();
        assert_eq!(result, expected_value);
    }

    #[tokio::test]
    async fn test_async_feature() {
        let result = my_async_function().await;
        assert!(result.is_ok());
    }
}
```

### Adding an Integration Test Case

```rust
use rmqtt_test::framework::testcase::{TestCase, TestResult};
use rmqtt_test::framework::context::TestContext;

struct MyCustomTest;

impl TestCase for MyCustomTest {
    fn name(&self) -> &str { "my_custom_test" }

    fn suite(&self) -> &str { "functional_v5" }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = std::time::Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            let mut client = ctx.create_client("test-client");
            client.connect().await?;
            client.subscribe("test/topic").await?;
            client.publish("test/topic", b"hello", 1, false).await?;
            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });
        match result {
            Ok(()) => TestResult::passed(self.name(), self.suite(), start.elapsed()),
            Err(e) => TestResult::failed(self.name(), self.suite(), start.elapsed(), e.to_string()),
        }
    }
}
```

Then register it in `rmqtt-test/src/tests/mod.rs` or the main entry point.

---

## CI/CD

The project's GitHub CI workflow:

```yaml
# .github/workflows/ci.yml
# Runs on: push to master, pull requests
# Steps:
#   1. Install system dependencies (protoc, etc.)
#   2. cargo check
#   3. cargo clippy --all-targets
#   4. cargo test
#   5. cargo build --release
```

Before pushing, always run:

```bash
cargo fmt --all && cargo clippy --all-targets && cargo test
```

---

## Test Output

### Console Output

```
============================================================
  RMQTT Test Harness - Results
============================================================

 ✔ connect_v3 [52ms]
 ✔ pubsub_v3_qos0 [48ms]
 ✔ connect_v311 [52ms]
 ...
------------------------------------------------------------
  Total: 26 | Passed: 26 | Failed: 0 | Duration: 12.34s
============================================================
```

### JSON Report

```json
{
  "suite": "rmqtt-test",
  "summary": { "total": 26, "passed": 26, "failed": 0, "duration_ms": 12340 },
  "cases": [
    { "name": "connect_v3", "suite": "functional_v3", "status": "passed", "duration_ms": 52 }
  ]
}
```

---

## Related Resources

- [CONTRIBUTING.md](../../CONTRIBUTING.md) — Contribution guidelines
- [benchmark-testing.md](../benchmark-testing.md) — Performance benchmark details
- Plugin READMEs — Individual plugin test notes
