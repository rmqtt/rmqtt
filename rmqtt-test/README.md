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

Without `--binary` the harness auto-locates the broker binary under the
workspace root in the order `target/release/rmqttd` → `target/debug/rmqttd`
and starts it; pass `--binary <path>` to pin a specific build. Note the
release-first order — whenever a release `rmqttd` exists it wins, even when you
launched the debug harness — so a stale release build can silently be the
binary under test.

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

### Running Specific Test Cases

```bash
# Single case (-t/--test matches a substring of the case name, repeatable)
./target/release/mqtt_harness --workspace . --suites functional_v5 -t will_delay_v5

# Several cases
./target/release/mqtt_harness --workspace . --suites functional_v5 \
  -t will_published_on_disconnect_rc_0x04_v5 \
  -t will_not_published_on_disconnect_rc_0x00_v5

# No --suites: search every suite by name and run only the matches
./target/release/mqtt_harness --workspace . -t mqtt_keepalive

# Timing-sensitive cases: pin --workers 1 so nothing else shares the broker
# (issue #513 message-lifecycle reproductions)
./target/release/mqtt_harness --workspace . --suites functional_v5 --workers 1 \
  -t retained_message_expiry_not_decremented_v5 \
  -t message_expiry_deletes_qos2_inflight_v5 \
  -t oversized_queued_message_stalls_queue_v5
```

> `-t/--test` is applied after the suite/config split, so a selected case still
> runs under its declared broker config and no extra config switches are
> introduced by the filtering; suites left without a match are dropped.
>
> `--workers N` (default 4) sets how many cases run in parallel against the
> single harness-managed broker. Cases that depend on fixed waits,
> connection-close detection, or strict delivery order should be pinned to
> `--workers 1` so a concurrently scheduled case cannot perturb their timing.
> The three cases above are the issue #513 reproductions listed in the
> `functional_v5` table below.

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
  session-sled/             # single node, sled session storage (issue #475 repro, harness-switched)
  session-sled-stress/      # same, isolated sled path (stress test only)
  cluster-broadcast-sled/   # two-node cluster (1886/1887 MQTT, self-managed by tests)
  cluster-broadcast-sled-stress/  # same, isolated sled path (stress test only)
  cluster-raft-sled/        # three-node raft cluster (1888/1889/1890 MQTT, 6008-6010 raft)
  cluster-raft-sled-stress/ # same, isolated sled path (stress test only)
  auth-http-acl-fallthrough/ # issue #501: auth-http 404-ignore × rmqtt-acl final rule
                             # (self-managed 1896 allow-all / 1900 deny-all, ephemeral mock)
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

### `functional_v3` (51 cases) — MQTT 3.1

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
| Protocol errors | `protocol_error_v3_subscribe_qos3` / `publish_packet_id_zero` / `publish_empty_topic` / `bad_remaining_length` / `empty_topic_filter` (subscribe/unsubscribe) / `reserved_packet_type` / `subscribe_qos0_fixed_header` |

> The v3.1 client hand-builds the MQIsdp CONNECT bytes (`build_connect_bytes`)
> because the codec hard-codes protocol level 4 (correct for 3.1.1/5.0).

### `functional_v311` (110 cases) — MQTT 3.1.1

| Category | Cases |
|----------|-------|
| Connect | `connect_v311` / `empty_client_id` / `multiple_connections` / `session_present_fresh` / `wrong_protocol_name` / `unsupported_level` / `reserved_flag` / `second_connect` [MQTT-3.1.0-2] / `long_client_id` / `client_id_65535` / `assigned_client_id` / `invalid_utf8_client_id` / `invalid_utf8_username` / `invalid_utf8_will_topic` / `username_flag_mismatch` / `password_flag_mismatch` / `will_flag_zero_but_qos_set` / `will_qos3` / `will_not_fire_on_rejected_connect` |
| Pub/Sub | `pubsub_v311_qos0/1/2` / `publish_wildcard_reject` / `qos_downgrade_v311` / `ordering_qos2_v311` |
| QoS 2 / resume | `qos2_replayed_publish_dedup_v311` [MQTT-4.3.3-10] / `qos2_pubrel_resend_on_resume_v311` [MQTT-4.4.0-1] / `qos2_duplicate_detection` / `qos1_publish_resend_on_resume_v311` / `qos2_broker_to_client_no_pubrec_v311` |
| Retained | `retain_v311_store_and_deliver` / `empty_payload_deletes` [MQTT-3.3.1-9] / `overwrite` / `live_message_not_retained` / `live_publish_keeps_retained` / `will` / `restart_recovery` |
| Last Will | `last_will_v311` / `qos0` / `qos1` / `qos2` / `clean` / `unclean` / `invalid_utf8_payload` / `keepalive_timeout` |
| Keep alive | `keepalive_v311_ping_keeps_alive` / `timeout` / `zero` / `max_value` / `pingresp_explicit` / `window_boundary` |
| Session | `clean_session_false` / `offline_queue_v311` / `present_on_resume` [MQTT-3.2.2.1] / `clean_discard` [MQTT-3.1.2-6] / `takeover` / `tcp_fin_rst` |
| Wildcard | `wildcard_plus` / `hash` / `case_sensitive` / `leading_slash` / `hash_not_last` / `overlap` / `empty_levels` |
| Auth / dollar / shared | `auth_empty_client_id_fail` / `auth_connect_disconnect_sequence` / `dollar_topics` / `shared_sub_v311` |
| Boundary | `max_client_id` / `long_topic` / `empty_payload` / `large_payload` / `special_chars_topic` / `rapid_subscribe` / `remaining_length_max` |
| Multi-topic | `multi_topic_subscribe_v311` / `overlapping_subscriptions` / `message_ordering` |
| Protocol errors | `invalid_protocol_version` / `protocol_error_v311_*` (subscribe/unsubscribe: qos3, qos0 fixed header, empty payload/filter, packet id 0; publish: qos3, pid0, empty topic, packet id on QoS0; bad remaining length, declared length mismatch, truncated packet, reserved packet type, packet type 15, pubrel/pubrec/pubcomp wrong flags, unsolicited pubrel, connect payload order, invalid UTF-8 topic) / `remaining_length_transition_v311` |
| CONNACK return codes (self-managed brokers) | `connack_return_codes_auth_http_v311` (auth-http + in-test mock, port 1892) / `connack_not_authorized_v311` (auth-jwt, port 1893) — these cases spawn their own brokers and don't use the harness broker |
| Issue #501 auth × ACL fallthrough (self-managed brokers) | `auth_http_ignore_allow_all_acl_v311` (auth service replies 404 → auth 'ignore'; acl `["allow", "all"]` promotes it → CONNACK 0x00 fail-open reproduction, port 1896) / `auth_http_ignore_deny_all_acl_v311` (acl `["deny", "all"]` backstop → CONNACK 0x05 fail-closed, port 1900) |

### `functional_v5` (108 cases) — MQTT 5.0

| Category | Cases |
|----------|-------|
| Connect / CONNACK | `connect_v5` / `connect_v5_reason_codes` / `connect_v5_session_present_fresh` / `connect_v5_wrong_protocol_name` / `connect_v5_unsupported_level` / `connect_v5_reserved_flag` / `connect_v5_second_connect` / `connect_v5_client_id_too_long` / `connect_v5_auth_method_rejected` (0x8C) / `connack_capabilities_v5` / `connack_receive_max_echo_v5` / `connack_assigned_client_id_v5` / `assigned_clientid_v5` / `v5_empty_clientid_cleanstart0_rejected` |
| Connect negative (🐞 expected-fail) | `connect_v5_will_flag_zero_but_qos_set` / `connect_v5_will_flag_zero_but_retain_set` [MQTT-3.1.2-11/12] — registered broker defects |
| Pub/Sub | `pubsub_v5_qos0` / `pubsub_v5_qos1` / `pubsub_v5_qos2` / `pubsub_v5_qos1_ordering` / `qos_downgrade_v5_matrix` / `publish_properties_passthrough_v5` |
| Session | `session_expiry_v5` / `session_takeover_v5` / `session_clean_start_v5` / `session_v5_disconnect_expiry_zero` [MQTT-3.14.2-2] / `session_v5_expiry_cleanup` / `session_v5_expiry_update_on_reconnect` |
| Server DISCONNECT on teardown | `takeover_sends_disconnect_0x8e_v5` [MQTT-3.1.4-3] — reproduction that currently FAILS: `Session::run` closes the sink before the v5 DISCONNECT is sent, so the taken-over Client sees a bare FIN instead of Reason Code 0x8E |
| Flow control | `flow_control_v5` / `flow_control_v5_inflight_cap_strict` |
| Flow-control negative (🐞 expected-fail) | `flow_control_v5_receive_max_violation` [MQTT-4.9.0-1/2] — registered broker defect (no DISCONNECT 0x93) |
| Topic alias | `client_topic_alias_v5` / `server_topic_alias_v5` / `topic_alias_v5_unknown_alias` (→ 0x94) / `topic_alias_v5_zero` (→ 0x94) / `topic_alias_v5_over_max` (→ 0x94) |
| Shared subscription | `shared_sub_v5` / `shared_sub_v5_malformed_filter` |
| Retain handling | `retain_handling_new_v5` / `retain_handling_no_at_subscribe_v5` / `retain_as_published_v5` |
| Message expiry | `publication_expiry_v5` / `message_expiry_v5_forwarded` / `message_expiry_v5_queued_drop` |
| Will | `last_will_v5_fires` / `will_delay_v5` / `will_properties_v5_delivery` / `will_published_on_abrupt_close_v5` / `will_published_on_disconnect_rc_0x04_v5` / `will_published_on_disconnect_rc_not_0x00_v5` / `will_not_published_on_disconnect_rc_0x00_v5` |
| Packet size / Server Keep Alive | `max_packet_size_v5` / `max_packet_size_enforcement_v5` / `server_keepalive_v5` |
| Request/Response & properties | `request_response_v5` / `request_problem_info_v5` / `user_properties_v5` / `payload_format_v5` |
| Subscription options | `subscribe_identifiers_v5` / `subscribe_identifiers_v5_update` / `subscribe_multi_filter_mixed_v5` / `no_local_v5` / `wildcard_available_v5` |
| Retained | `retain_v5_store_and_deliver` / `retain_v5_empty_payload_deletes` / `retain_v5_overwrite` / `retain_v5_live_message_not_retained` / `retain_v5_will` |
| QoS 2 | `qos2_replayed_publish_dedup` [MQTT-4.3.3-10] / `qos2_pubrel_resend_on_resume` [MQTT-4.4.0-1] / `qos2_pubrel_resume_collision` |
| Wildcard | `wildcard_v5_case_sensitive` / `wildcard_v5_leading_slash` |
| Reason codes (MAY-level) | `reason_code_v5_puback_no_matching_subscribers` / `reason_code_v5_unsuback_no_subscription` — assert a legal reason code (0x00 or 0x10 / 0x11) |
| Response/Problem Information (info) | `connack_response_info_v5` / `publish_v5_response_topic_wildcard` — record-type observations, never failures |
| Protocol errors | `protocol_error_v5_bad_remaining_length` / `protocol_error_v5_disconnect_bad_flags` / `protocol_error_v5_invalid_utf8_topic` / `protocol_error_v5_publish_dup_on_qos0` / `protocol_error_v5_publish_empty_topic` / `protocol_error_v5_publish_packet_id_zero` / `protocol_error_v5_publish_qos3` / `protocol_error_v5_reserved_packet_type` / `protocol_error_v5_retain_handling_3` / `protocol_error_v5_sub_id_zero` / `protocol_error_v5_sub_options_reserved_bits` / `protocol_error_v5_subscribe_empty_payload` / `protocol_error_v5_subscribe_packet_id_zero` / `protocol_error_v5_subscribe_qos0_fixed_header` / `protocol_error_v5_subscribe_qos3` / `protocol_error_v5_unsolicited_auth` / `protocol_error_v5_unsubscribe_empty_payload` / `protocol_error_v5_unsubscribe_packet_id_zero` / `protocol_error_v5_unsubscribe_qos0_fixed_header` / `protocol_error_v5_unsubscribe_with_sub_id` / `protocol_error_v5_user_property_bad_utf8` |
| Disconnect | `disconnect_reason_v5` |
| Keep alive / TCP | `ping_v5` / `mqtt_keepalive_timeout_reclaims_tcp` / `tcp_keepalive_socket_option` (Linux-gated, skipped elsewhere) |
| Message lifecycle (issue #513) | `retained_message_expiry_not_decremented_v5` [MQTT-3.3.2-6] — FIXED, now PASSing (the outgoing Message Expiry Interval is decremented by the time spent in the retain store); `message_expiry_deletes_qos2_inflight_v5` [MQTT-4.3.3-7] / [MQTT-4.4.0-1] — FIXED, now PASSing (an `UnComplete` exchange is no longer deleted by Message Expiry); `oversized_queued_message_stalls_queue_v5` + `retained_oversized_message_keeps_session_v5` [MQTT-3.1.2-24/-25] — FIXED, now PASSing; see the note below |
| Will Retain vs Retain Available | `v5_will_retain_rejected_when_retain_unavailable` (executed in the `functional_v5@retain-disabled` sub-suite) |

> **Expected-fail cases (🐞)**: they execute fully but assert behaviors the
> broker does not implement yet (registered conformance gaps). A failure is
> recorded as `EXPECTED-FAIL` and does not count against the suite; when the
> broker becomes compliant the case surfaces as `UNEXPECTED-PASS` and should
> be promoted to a normal assertion. See
> `designs/mqtt-5.0-standalone-test-gap-analysis.md`.

> `functional_v5` totals 108 cases: the default-config group runs 106 of them;
> `v5_will_retain_rejected_when_retain_unavailable` and
> `qos2_pubrel_resume_collision` require different broker configs and are
> automatically split into the `functional_v5@retain-disabled` and
> `functional_v5@pubrel-collision` sub-suites at build time (see the
> "Broker Configs" section above). A full `--suites functional_v5 --workers 1`
> run reports `Total: 108 | Passed: 101 | Failed: 1 | Skipped: 1 |
> ExpectedFail: 3 | Info: 2`.

> **Four issue #513-related cases are deliberately NOT marked expected-fail
> (🐞).** Each asserts spec-required behaviour: a retained message's `Message
> Expiry Interval` must be decremented by the time spent in the retain store
> ([MQTT-3.3.2-6]); a PUBREL owed after PUBREC must be re-sent with its original
> Packet Identifier on the resumed session ([MQTT-4.4.0-1]); a packet above the
> client's Maximum Packet Size must be discarded and the server must then behave
> as if it had completed sending that Application Message, on the queued path
> and on the retained path alike ([MQTT-3.1.2-24] / [MQTT-3.1.2-25]). They
> therefore surface as ordinary failures until the defect is fixed, then flip to
> PASS with no test change — unlike the 🐞 cases above, which stay silent while
> non-conformant. All four now PASS: `message_expiry_deletes_qos2_inflight_v5`,
> because `Session::reforward` no longer applies Message Expiry to an
> `UnComplete` exchange — PUBREC proves the PUBLISH was sent, and [MQTT-4.3.3-7]
> forbids applying expiry from that point on; the two oversized-message cases,
> because `EncodeError::OverMaxPacketSize` is now downgraded to a completed
> delivery in `Session::deliver` (`rmqtt/src/session.rs`) instead of escaping the
> session event loop; and `retained_message_expiry_not_decremented_v5`, because
> `_send_retain_messages` no longer stamps `publish.create_time` with the
> delivery time, leaving `message_expiry_check` the original receipt timestamp
> to subtract the message's wait from. Run them serially (see "Running Specific
> Test Cases").

> **`takeover_sends_disconnect_0x8e_v5` is likewise NOT marked expected-fail
> (🐞).** It reproduces a fourth, independent defect found while investigating
> #513 — connection *teardown*, not message lifetime. `Session::run` calls
> `sink.close()` (which shuts the write half down) **before** building and
> sending the v5 DISCONNECT, and the resulting error is discarded by `let _ =`.
> Every server-initiated Reason Code — 0x8D Keep Alive timeout, 0x8E Session
> taken over, 0x93 Receive Maximum exceeded, 0x95 Packet too large — is
> therefore dead code, and a Client cannot tell "the Server dropped me, and
> here is why" from "the network died". [MQTT-3.1.4-3] makes the 0x8E
> DISCONNECT a MUST for a session takeover, so the case is the only red left in
> `functional_v5` until the ordering is fixed. It asserts two layers
> separately, so a partial fix is still reported precisely: against a stub
> broker that sends 0x8E before closing it PASSes, and against one that sends
> 0x87 it fails on the reason code alone.

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

### `stress` (6 cases)

| Case | Description |
|------|-------------|
| `connection_load` | N concurrent client connect/disconnect (default 100) |
| `publish_load` | Continuous publish 1000 QoS 1 messages, QPS statistics |
| `fan_out` | 1 publisher → N subscribers fan-out test |
| `stress_mixed_qos_v311` | Mixed QoS 0/1/2 traffic (v3.1.1 client) |
| `stress_subscription_mass` | Mass subscription setup and delivery verification |
| `stress_retain_flood` | Publish many retained messages to flood broker memory |

### `chaos` (18 cases)

| Case | Description |
|------|-------------|
| `chaos_broker_restart` | Client reconnection after broker restart |
| `chaos_broker_restart_pubsub` | Pub/Sub recovery after broker restart |
| `chaos_connection_churn` | Rapid connect/disconnect cycling |
| `chaos_reconnect_storm` | 50 concurrent connection storms |
| `chaos_qos1_reliability` | QoS 1 reliability verification |
| `chaos_slow_consumer` | Slow consumer scenario |
| `session_storage_expired_cleanup` / `_edge` | Session-storage startup-load optimization: expired offline sessions are skipped (and removed) during load, live sessions survive (edge variant included) |
| `chaos_broker_restart_session_routing` | Issue #475 single-node reproduction: a persistent session restored from sled must stay routable across a broker restart (sub-suite `chaos@session-sled`) |
| `cluster_restart_session_routing_broadcast` / `_raft` | Same defect through a cluster (broadcast 2 nodes / raft 3 nodes), node 1 restarted only |
| `cluster_whole_restart_session_routing_broadcast` / `_raft` | Same defect through a cluster, whole-cluster restart |
| `stress_single_node_restart_session_routing` | Issue #475 stress: 1000 persistent sessions × 100 QoS 1 messages across a standalone-broker restart (sub-suite `chaos@session-sled-stress`) |
| `stress_cluster_restart_session_routing_broadcast` / `_raft` | Same stress via cluster-broadcast / cluster-raft, node 1 restarted only |
| `stress_cluster_whole_restart_session_routing_broadcast` / `_raft` | Same stress via cluster-broadcast / cluster-raft, whole-cluster restart |

#### Issue #475 stress tests — how to run

The 5 stress tests scale the issue #475 reproduction to **1000 persistent
sessions × 100 QoS 1 messages** (100k publishes) and require the broker +
harness to be built with rustc ≥ 1.94:

```bash
RUSTUP_TOOLCHAIN=1.97 cargo build -p rmqttd
RUSTUP_TOOLCHAIN=1.97 cargo build -p rmqtt-test

# Clean sled data from previous runs (a large sled makes broker startup
# slow; the harness health-check timeout is now 60s as a fallback, but
# heavy accumulation still slows startup — clean before each run):
rm -rf rmqtt-test/configs/{session-sled,session-sled-stress,cluster-broadcast-sled,cluster-broadcast-sled-stress,cluster-raft-sled,cluster-raft-sled-stress}/.sled

# Full chaos suite (functional restart tests + all 5 stress tests, ~6.5 min):
./target/debug/mqtt_harness --binary target/debug/rmqttd \
  --config rmqtt-test/configs/default/rmqtt.toml \
  --workspace . --suites chaos --workers 1

# Only the standalone-broker stress test (~25 s):
./target/debug/mqtt_harness --binary target/debug/rmqttd \
  --config rmqtt-test/configs/default/rmqtt.toml \
  --workspace . --suites chaos@session-sled-stress --workers 1
```

Cluster stress tests are self-managed processes registered in the main
`chaos` suite (no dedicated sub-suite); per-node logs are written to
`target/cluster-stress-{broadcast,raft,...}-node{1,2,3}.log`. Scale is
controlled by `STRESS_SESSIONS` / `STRESS_MSGS_PER_SESSION` in
`src/tests/functional/session_restart_stress.rs`. Design & defect analysis:
[`designs/issue-475-restored-session-routing-fix.md`](../designs/issue-475-restored-session-routing-fix.md).

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
      functional/cluster_session_restart.rs  # issue #475 cluster reproduction (broadcast/raft)
      functional/session_restart_stress.rs   # issue #475 stress (1000×100, 5 scenarios)
    report/                      # Report system (console, JSON, HTML, detail log)
  configs/                       # Test broker configs (all self-contained)
    default/                     #   default config: rmqtt.toml + plugins/ (retainer/shared-subscription/http-api)
    retain-disabled/             #   retainer plugin NOT loaded (Retain Available = 0)
    pubrel-collision/            #   single node: message-storage enabled broker config
    pubrel-collision-cluster/    #   cluster: node1/node2 configs (1884/1885 MQTT, 5364/5365 gRPC)
    session-sled/                #   single node, sled session storage (issue #475 reproduction)
    session-sled-stress/         #   same, isolated sled path for the stress test
    cluster-broadcast-sled/      #   2-node cluster (1886/1887 MQTT, 5366/5367 gRPC)
    cluster-broadcast-sled-stress/ #  same, isolated sled path for the stress test
    cluster-raft-sled/           #   3-node raft cluster (1888/1889/1890 MQTT, 5368-5370 gRPC, 6008-6010 raft)
    cluster-raft-sled-stress/    #   same, isolated sled path for the stress test
    auth-http-acl-fallthrough/   #   issue #501 repro: auth-http (ephemeral mock, always 404) +
                                 #   rmqtt-acl; self-managed brokers on 1896/5376 (allow-all)
                                 #   and 1900/5377 (deny-all)
```

> **Test isolation note**: all tests that publish retained messages delete them
> afterwards (empty payload + RETAIN=1); `#` wildcard tests drain stale retained
> messages and poll-filter their own payloads, so suites can run concurrently
> (with `--workers N`) without cross-test interference.

## 📄 License

MIT OR Apache-2.0
