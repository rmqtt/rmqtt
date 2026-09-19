//! Pulsar bridge suite (`--suites pulsar`).
//!
//! Verifies the two Pulsar bridge plugins end to end with an **external**
//! Apache Pulsar service at `pulsar://127.0.0.1:6650`:
//!
//! * `egress`  - MQTT publish   -> Pulsar topic (see [`egress`])
//! * `ingress` - Pulsar message -> MQTT publish (see [`ingress`])
//!
//! The suite is bound to its own broker config
//! (`rmqtt-test/configs/pulsar/rmqtt.toml`, which starts
//! `rmqtt-bridge-egress-pulsar`, `rmqtt-bridge-ingress-pulsar` and
//! `rmqtt-retainer`), it is never part of the default full run because it
//! depends on that external service, and every test reports `Skipped` when the
//! service is unreachable.
//!
//! ```bash
//! cargo build -p rmqttd && cargo build -p rmqtt-test
//! ./target/debug/mqtt_harness --workspace . --suites pulsar --workers 1
//! ```

pub mod common;
pub mod egress;
pub mod ingress;
