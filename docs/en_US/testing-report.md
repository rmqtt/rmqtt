[**English**](testing-report.md) | [简体中文](../zh_CN/testing-report.md)

# RMQTT Test Report

This document provides the detailed test results for the RMQTT MQTT broker, including interoperability testing against the paho.mqtt.testing suite, and performance benchmark data.

---

## Interoperability Testing

RMQTT is tested against the official [paho.mqtt.testing](https://github.com/eclipse/paho.mqtt.testing) interoperability test suite.

### Setup

```bash
git clone https://github.com/eclipse/paho.mqtt.testing.git
cd paho.mqtt.testing/interoperability

# Start RMQTT broker (separate terminal)
./target/release/rmqttd
```

### MQTT V3.1.1 — 11/11 Passing

| Test | Result | Notes |
|------|--------|-------|
| `test_retained_messages` | ✅ OK | — |
| `test_zero_length_clientid` | ✅ OK | — |
| `will_message_test` | ✅ OK | — |
| `test_offline_message_queueing` | ✅ OK | — |
| `test_overlapping_subscriptions` | ✅ OK | — |
| `test_keepalive` | ✅ OK | — |
| `test_redelivery_on_reconnect` | ✅ OK | — |
| `test_dollar_topics` | ✅ OK | — |
| `test_unsubscribe` | ✅ OK | — |
| `test_subscribe_failure` | ✅ OK | Requires ACL config: add `["deny", "all", "subscribe", ["test/nosubscribe"]]` at first line of `rmqtt-acl.toml` |
| `test_zero_length_clientid` | ✅ OK | — |

### MQTT V5.0 — 24/24 Passing

| Test | Result | Notes |
|------|--------|-------|
| `test_retained_message` | ✅ OK | — |
| `test_will_message` | ✅ OK | — |
| `test_offline_message_queueing` | ✅ OK | — |
| `test_dollar_topics` | ✅ OK | — |
| `test_unsubscribe` | ✅ OK | — |
| `test_session_expiry` | ✅ OK | — |
| `test_shared_subscriptions` | ✅ OK | — |
| `test_basic` | ✅ OK | — |
| `test_overlapping_subscriptions` | ✅ OK | — |
| `test_redelivery_on_reconnect` | ✅ OK | — |
| `test_payload_format` | ✅ OK | — |
| `test_publication_expiry` | ✅ OK | — |
| `test_subscribe_options` | ✅ OK | — |
| `test_assigned_clientid` | ✅ OK | — |
| `test_subscribe_identifiers` | ✅ OK | — |
| `test_request_response` | ✅ OK | — |
| `test_server_topic_alias` | ✅ OK | — |
| `test_client_topic_alias` | ✅ OK | — |
| `test_maximum_packet_size` | ✅ OK | — |
| `test_keepalive` | ✅ OK | — |
| `test_zero_length_clientid` | ✅ OK | — |
| `test_user_properties` | ✅ OK | — |
| `test_flow_control2` | ✅ OK | — |
| `test_flow_control1` | ✅ OK | — |
| `test_will_delay` | ✅ OK | — |
| `test_server_keep_alive` | ✅ OK | Requires config: set `max_keepalive` to 60 in `rmqtt.toml` |
| `test_subscribe_failure` | ✅ OK | Requires ACL config: same as v3.1.1 |

---

## Integration Test Harness

The `rmqtt-test` crate provides a custom test harness with additional test suites beyond paho:

| Suite | Cases | Description |
|-------|-------|-------------|
| `functional_v3` | 2 | MQTT 3.1 basic operations (connect/disconnect, QoS 0 pub/sub) |
| `functional_v311` | 10 | MQTT 3.1.1 protocol compliance |
| `functional_v5` | 5 | MQTT 5.0 protocol compliance |
| `stress` | 3 | Connection load (100 clients), publish QPS (1000 msgs), fan-out (1→N) |
| `chaos` | 6 | Broker restart, connection storms, reconnect, QoS 1 reliability, slow consumer |

```bash
# Run all test suites
cargo build --release
cargo build -p rmqtt-test --release
./target/release/mqtt_harness --workspace .
```

---

## Benchmark

### Environment

| Item | Content |
|------|---------|
| System | x86_64 GNU/Linux, Rocky Linux 9.2 (Blue Onyx) |
| CPU | Intel(R) Xeon(R) CPU E5-2696 v3 @ 2.30GHz, 72 threads (18 cores × 2 threads × 2 sockets) |
| Memory | DDR3/2333, 128 GB |
| Disk | 2 TB |
| Container | Podman v4.4.1 |
| MQTT Bench | `docker.io/rmqtt/rmqtt-bench:latest` (v0.1.3) |
| MQTT Broker | `docker.io/rmqtt/rmqtt:latest` (v0.3.0) |

*Note: MQTT Bench and MQTT Broker run on the same host.*

### Connection Concurrency

| Metric | Single Node | Raft Cluster (3 nodes) |
|--------|-------------|----------------------|
| Total Concurrent Clients | 1,000,000 | 1,000,000 |
| Connection Handshake Rate | 5,500-7,000/s | 5,000-7,000/s |

### Message Throughput

| Metric | Single Node | Raft Cluster (3 nodes) |
|--------|-------------|----------------------|
| Subscription Clients | 1,000,000 | 1,000,000 |
| Publishing Clients | 40 | 40 |
| Message Throughput Rate | 150,000 msg/s | 156,000 msg/s |

[Detailed benchmark documentation →](./benchmark-testing.md)

---

## License

MIT OR Apache-2.0
