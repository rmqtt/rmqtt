[**English**](README.md) | [简体中文](README-CN.md)

# rmqtt-test

[![crates.io page](https://img.shields.io/crates/v/rmqtt.svg)](https://crates.io/crates/rmqtt)
![Rust](https://img.shields.io/badge/rust-1.94%2B-blue)

**rmqtt-test** is the industrial-grade test harness, chaos engineering, and benchmarking engine for the [RMQTT](https://github.com/rmqtt/rmqtt) MQTT broker.

The build artifact `mqtt_harness` is a standalone executable that provides functional testing, stress testing, chaos testing, and outputs structured test reports.

## ✨ Features

- **Custom MQTT Client** — Zero third-party MQTT dependency, complete MQTT 3.1 / 3.1.1 / 5.0 protocol stack
- **Broker Lifecycle Management** — Auto start/stop/restart `rmqttd` process with TCP health checks
- **Six Test Suites** — `functional_v3` / `functional_v311` / `functional_v5` / `functional_v5_cluster` / `stress` / `chaos`
- **Full QoS Coverage** — QoS 0 / QoS 1 / QoS 2 (including full 4-step handshake) correctness verification
- **Concurrency Bug Reproduction** — QoS 2 PUBREL-resume packet-id collision (unit-level + cluster end-to-end)
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
By default it uses the **self-contained config**
`rmqtt-test/configs/default/rmqtt.toml` (independent from the repository-root
`rmqtt.toml` / `rmqtt-plugins/*.toml`; all TCP/TLS/WS/WSS/QUIC listeners are
kept enabled for the upcoming TLS/WS/QUIC test suites).

### Use a Different Broker Config

```bash
# Explicit config for the whole run
./target/release/mqtt_harness --workspace . --config rmqtt-test/configs/retain-disabled/rmqtt.toml

# Run only one config-split sub-suite
./target/release/mqtt_harness --workspace . --suites functional_v5@retain-disabled
```

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

> `--suites` supports prefix matching: `functional_v5` also selects every
> config-split sub-suite (e.g. `functional_v5@retain-disabled`). The
> `functional_v5_cluster` two-node suite only runs when explicitly requested
> and is never part of the default full run.

## ⚙️ Broker Configs (self-contained `configs/`)

All test broker configs live under `rmqtt-test/configs/<name>/` and are
**self-contained** (main config + own `plugins/` sub-dir), independent from
the repository-root `rmqtt.toml` / `rmqtt-plugins/*.toml`:

```
configs/
  default/                  # default config (used when --config is omitted)
    rmqtt.toml              #   based on the repo-root config, all listeners kept
    plugins/                #   retainer / shared-subscription / http-api
  retain-disabled/          # retainer plugin NOT loaded (Retain Available = 0)
  pubrel-collision/         # message-storage loaded (PUBREL collision repro)
  pubrel-collision-cluster/ # two-node cluster (manual start, 1884/1885 MQTT)
```

**Per-test config switching**: a test case can declare its required config via
`TestCase::broker_config()` (e.g. `WillRetainRejectedWhenRetainUnavailableV5Test`
→ `retain-disabled`, `Qos2PubrelResumeCollisionTest` → `pubrel-collision`).
At suite build time, cases declaring the same non-default config are split
into a dedicated `{suite}@{config}` sub-suite (e.g.
`functional_v5@retain-disabled`); the scheduler switches the broker config
(restart) only at **suite boundaries**, and the default-config group keeps
its original name with zero extra restarts.

Port constraint: configs participating in auto-switching must listen on the
harness `--addr` (default `127.0.0.1:1883`), otherwise the health check fails.

## 📋 Test Suites

### `functional_v3` (47 cases) — MQTT 3.1

Spec-conformance suite for MQTT v3.1 (IBM MQIsdp), covering positive, negative
and boundary scenarios:

| Category | Cases |
|----------|-------|
| Connect | `connect_v3` / `with_options` / `wrong_protocol_name` / `unsupported_level` / `reserved_flag` / `empty_clientid_cleansession0/1` / `long_client_id` / `client_id_max_length` |
| Pub/Sub | `pubsub_v3_qos0/1/2` / `publish_v3_wildcard_reject` |
| QoS 2 conformance | `qos2_replayed_publish_dedup_v3` [MQTT-4.3.3-10] / `qos2_pubrel_resend_on_resume_v3` [MQTT-4.4.0-1] |
| Retained | `retain_v3_store_and_deliver` / `empty_payload_deletes` / `overwrite` / `live_message_not_retained` / `will` |
| Last Will | `last_will_v3` / `clean` / `qos2` |
| Keep alive | `keepalive_v3_ping` / `zero` / `timeout` |
| Session | `session_v3_persistent` / `clean` / `offline_queue` |
| Wildcard | `wildcard_v3_plus` / `hash` / `overlap` / `dollar_topics` / `case_sensitive` / `leading_slash` |
| Boundary | `boundary_v3_empty_payload` / `large_payload` / `long_topic` / `special_chars_topic` / `max_keepalive` / `rapid_subscribe` |
| Protocol errors | `protocol_error_v3_subscribe_qos3` / `publish_packet_id_zero` / `bad_remaining_length` / `empty_topic_filter` / `reserved_packet_type` / `subscribe_qos0_fixed_header` |

> The v3.1 client hand-builds the MQIsdp CONNECT bytes (`build_connect_bytes`)
> because the codec hard-codes protocol level 4 (correct for 3.1.1/5.0).

### `functional_v311` (64 cases) — MQTT 3.1.1

| Category | Cases |
|----------|-------|
| Connect | `connect_v311` / `empty_client_id` / `multiple_connections` / `session_present_fresh` / `wrong_protocol_name` / `unsupported_level` / `reserved_flag` / `second_connect` [MQTT-3.1.0-2] / `long_client_id` |
| Pub/Sub | `pubsub_v311_qos0/1/2` / `retain_v311_message` / `unsubscribe_v311` |
| QoS 2 conformance | `qos2_replayed_publish_dedup_v311` [MQTT-4.3.3-10] / `qos2_pubrel_resend_on_resume_v311` [MQTT-4.4.0-1] / `qos2_duplicate_detection` |
| Retained | `retain_v311_store_and_deliver` / `empty_payload_deletes` [MQTT-3.3.1-9] / `overwrite` / `live_message_not_retained` / `will` |
| Last Will | `last_will_v311` / `clean` / `unclean` / `qos2` / `keepalive_timeout` |
| Keep alive | `keepalive_v311_ping_keeps_alive` / `timeout` / `zero` / `max_value` |
| Session | `clean_session_false` / `offline_queue_v311` / `present_on_resume` [MQTT-3.2.2.1] / `clean_discard` [MQTT-3.1.2-6] |
| Wildcard | `wildcard_plus` / `hash` / `case_sensitive` / `leading_slash` / `hash_not_last` |
| Auth / dollar / shared | `auth_empty_client_id_fail` / `auth_connect_disconnect_sequence` / `dollar_topics` / `shared_sub_v311` |
| Boundary | `max_client_id` / `long_topic` / `empty_payload` / `large_payload` / `special_chars_topic` / `rapid_subscribe` |
| Multi-topic | `multi_topic_subscribe_v311` / `overlapping_subscriptions` / `message_ordering` |
| Protocol errors | `invalid_protocol_version` / `empty_topic_filter` / `protocol_error_v311_*` (subscribe qos3, fixed-header QoS, publish qos3/pid0, bad remaining length, reserved type) |

### `functional_v5` (63 cases) — MQTT 5.0

| Category | Cases |
|----------|-------|
| Connect / CONNACK | `connect_v5` / `reason_codes` / `session_present_fresh` / `wrong_protocol_name` / `unsupported_level` / `reserved_flag` / `second_connect` / `client_id_too_long` / `auth_method_rejected` (0x8C) / `connack_capabilities_v5` / `connack_receive_max_echo_v5` / `connack_assigned_client_id_v5` / `empty_clientid_cleanstart0_rejected` |
| Pub/Sub | `pubsub_v5_qos0/1/2` |
| Session | `session_expiry_v5` / `takeover_v5` / `clean_start_v5` / `disconnect_expiry_zero` [MQTT-3.14.2-2] / `expiry_cleanup` |
| V5 features | `flow_control_v5` / `no_local_v5` / `will_delay_v5` / `shared_sub_v5` / `topic_alias_v5` (server/client/unknown-alias → 0x94) / `retain_handling_*_v5` / `retain_as_published_v5` / `server_keepalive_v5` / `max_packet_size_v5` (+ enforcement) / `subscribe_identifiers_v5` / `payload_format_v5` / `publication_expiry_v5` / `request_response_v5` / `user_properties_v5` / `wildcard_available_v5` |
| Retained | `retain_v5_store_and_deliver` / `empty_payload_deletes` / `overwrite` / `live_message_not_retained` / `will` |
| QoS 2 | `qos2_replayed_publish_dedup` [MQTT-4.3.3-10] / `qos2_pubrel_resend_on_resume` [MQTT-4.4.0-1] / `qos2_pubrel_resume_collision` |
| Wildcard | `wildcard_v5_case_sensitive` / `leading_slash` |
| Protocol errors | `protocol_error_v5_*` (subscribe qos3, fixed-header QoS, publish qos3/pid0/empty-topic, bad remaining length, reserved type) |
| Disconnect | `disconnect_reason_v5` |
| Will Retain vs Retain Available | `will_retain_rejected_when_retain_unavailable_v5` (executed in the `functional_v5@retain-disabled` sub-suite) |

> `functional_v5` totals 63 cases: the default-config group runs 61 of them;
> `will_retain_rejected_when_retain_unavailable_v5` and
> `qos2_pubrel_resume_collision` require different broker configs and are
> automatically split into the `functional_v5@retain-disabled` and
> `functional_v5@pubrel-collision` sub-suites at build time (see the
> "Broker Configs" section above).

### `functional_v5_cluster` (1 case) — two-node cluster end-to-end reproduction

| Case | Description |
|------|-------------|
| `qos2_pubrel_resume_collision_cluster` | Cluster-path end-to-end reproduction of the packet-id collision: remote delivery is not `mark_forwarded` on the receiving node, so a stored message loaded during cross-node session resume races with owed PUBREL re-sends |

This suite **requires two manually started nodes** and is never included in the default full run (so it cannot break the single-node suites):

```bash
# terminal 1 / terminal 2: start both nodes
./target/release/rmqttd -f rmqtt-test/configs/pubrel-collision-cluster/node1/rmqtt.toml
./target/release/rmqttd -f rmqtt-test/configs/pubrel-collision-cluster/node2/rmqtt.toml

# terminal 3: run the cluster reproduction suite
./target/release/mqtt_harness --no-broker --addr 127.0.0.1:1884 --suites functional_v5_cluster --workers 1
```

> Before the fix this test reproduced the BUG in 3/3 rounds (duplicate PUBREL);
> after the fix it PASSES in 3/3 rounds. Fix design: see
> [`designs/pubrel-resume-inflight-id-collision.md`](../designs/pubrel-resume-inflight-id-collision.md).

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
      v3/                        # MQTT 3.1 client (QoS 0/1/2, hand-built MQIsdp CONNECT)
      v311/                      # MQTT 3.1.1 client (QoS 0/1/2)
      v5/                        # MQTT 5.0 client (QoS 0/1/2)
    transport/                   # Network transport layer (incl. raw-byte send for negative tests)
    framework/                   # Test framework (TestCase, DAG scheduler, context)
    tests/                       # Test cases (functional, stress, chaos)
      functional/                #   functional_v3/v311/v5 cases
      functional/qos2_pubrel_resume_collision_cluster.rs  # cluster reproduction case
    report/                      # Report system (console, JSON, HTML, detail log)
  configs/                       # Test broker configs (all self-contained)
    default/                     #   default config: rmqtt.toml + plugins/ (retainer/shared-subscription/http-api)
    retain-disabled/             #   retainer plugin NOT loaded (Retain Available = 0)
    pubrel-collision/            #   single node: message-storage enabled broker config
    pubrel-collision-cluster/    #   cluster: node1/node2 configs (1884/1885 MQTT, 5364/5365 gRPC)
```

> **Test isolation note**: all tests that publish retained messages delete them
> afterwards (empty payload + RETAIN=1); `#` wildcard tests drain stale retained
> messages and poll-filter their own payloads, so suites can run concurrently
> (with `--workers N`) without cross-test interference.

## 📄 License

MIT OR Apache-2.0
