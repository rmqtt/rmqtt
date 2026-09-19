//! Egress direction tests: MQTT publish -> Pulsar topic.
//!
//! Every case builds its own Pulsar consumer (the independent observer) on the
//! remote topic configured in
//! `rmqtt-test/configs/pulsar/plugins/rmqtt-bridge-egress-pulsar.toml`, then
//! publishes over MQTT and asserts what Pulsar received.
//!
//! Ordering rule inside each case: the observer subscription is created
//! *before* the MQTT publish, so `initial_position = latest` guarantees the
//! message cannot be missed.

use std::time::Duration;

use super::common::{
    ack_pulsar_message, connect_pulsar, create_observer, expect_no_pulsar_message, mqtt_v311, nonce,
    properties, property, recv_pulsar_payload, run_async, tagged_payload, EGRESS_SKIP_TOPIC, EGRESS_TOPIC,
    NEGATIVE_TIMEOUT, RECV_TIMEOUT,
};
use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// MQTT topics routed to `EGRESS_TOPIC` (`pulsar/egress/#` in the plugin config).
const TOPIC_BASIC: &str = "pulsar/egress/basic";
const TOPIC_QOS1: &str = "pulsar/egress/qos1";
const TOPIC_BINARY: &str = "pulsar/egress/binary";
const TOPIC_LARGE: &str = "pulsar/egress/large";
const TOPIC_RETAIN: &str = "pulsar/egress/retain";
const TOPIC_FROM: &str = "pulsar/egress/from";
/// Deliberately *outside* `pulsar/egress/#`: must not be forwarded.
const TOPIC_UNMATCHED: &str = "pulsar/other/unmatched";
/// In-scope control topic (multi-level, so no `#`-matches-parent ambiguity).
const TOPIC_CONTROL: &str = "pulsar/egress/control";
/// Routed to `EGRESS_SKIP_TOPIC` by the second bridge with `skip_levels = 2`.
const TOPIC_SKIP_LEVELS: &str = "pulsar/skip/a/data";

/// Declares an egress test case (struct + `TestCase` impl) over an async body.
macro_rules! egress_case {
    ($ty:ident, $case:literal, $body:ident) => {
        pub struct $ty;

        impl TestCase for $ty {
            fn name(&self) -> &str {
                $case
            }

            fn execute(&self, ctx: &mut TestContext) -> TestResult {
                let mqtt_addr = ctx.config.broker_addr.clone();
                run_async(self.name(), mqtt_addr, |addr| $body(addr))
            }

            fn timeout(&self) -> Duration {
                Duration::from_secs(120)
            }
        }
    };
}

/// Assert one Pulsar message property and return a readable error otherwise.
fn expect_property(
    msg: &pulsar::consumer::Message<Vec<u8>>,
    key: &str,
    expected: &str,
) -> Result<(), anyhow::Error> {
    match property(msg, key) {
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => Err(anyhow::anyhow!(
            "Pulsar property {key:?} = {actual:?}, expected {expected:?} (all properties: {:?})",
            properties(msg)
        )),
        None => Err(anyhow::anyhow!(
            "Pulsar property {key:?} missing, expected {expected:?} (all properties: {:?})",
            properties(msg)
        )),
    }
}

// ---------------------------------------------------------------------------
// EG-01 basic QoS 0 forwarding
// ---------------------------------------------------------------------------

async fn run_basic_qos0(mqtt_addr: String) -> Result<(), anyhow::Error> {
    let pulsar = connect_pulsar().await?;
    let mut observer = create_observer(&pulsar, EGRESS_TOPIC, &nonce()).await?;
    let mqtt = mqtt_v311(&mqtt_addr, &format!("egress-basic-{}", nonce())).await?;

    let payload = tagged_payload("egress-basic");
    mqtt.publish(TOPIC_BASIC, &payload, QoS::AtMostOnce, false).await?;

    let msg = recv_pulsar_payload(&mut observer, &payload, RECV_TIMEOUT).await?;
    // `topic` is the one property the egress plugin always forwards.
    expect_property(&msg, "topic", TOPIC_BASIC)?;
    ack_pulsar_message(&mut observer, &msg).await?;

    let _ = mqtt.disconnect().await;
    Ok(())
}

egress_case!(PulsarEgressBasicQos0Test, "pulsar_egress_basic_qos0", run_basic_qos0);

// ---------------------------------------------------------------------------
// EG-02 QoS 1 forwarding
// ---------------------------------------------------------------------------

async fn run_qos1(mqtt_addr: String) -> Result<(), anyhow::Error> {
    let pulsar = connect_pulsar().await?;
    let mut observer = create_observer(&pulsar, EGRESS_TOPIC, &nonce()).await?;
    let mqtt = mqtt_v311(&mqtt_addr, &format!("egress-qos1-{}", nonce())).await?;

    let payload = tagged_payload("egress-qos1");
    // QoS 1: `publish` waits for the PUBACK before returning.
    mqtt.publish(TOPIC_QOS1, &payload, QoS::AtLeastOnce, false).await?;

    let msg = recv_pulsar_payload(&mut observer, &payload, RECV_TIMEOUT).await?;
    expect_property(&msg, "topic", TOPIC_QOS1)?;
    expect_property(&msg, "qos", "1")?;
    ack_pulsar_message(&mut observer, &msg).await?;

    let _ = mqtt.disconnect().await;
    Ok(())
}

egress_case!(PulsarEgressQos1Test, "pulsar_egress_qos1", run_qos1);

// ---------------------------------------------------------------------------
// EG-03 binary payload integrity
// ---------------------------------------------------------------------------

async fn run_binary_payload(mqtt_addr: String) -> Result<(), anyhow::Error> {
    let pulsar = connect_pulsar().await?;
    let mut observer = create_observer(&pulsar, EGRESS_TOPIC, &nonce()).await?;
    let mqtt = mqtt_v311(&mqtt_addr, &format!("egress-binary-{}", nonce())).await?;

    // NUL byte, invalid UTF-8 (0xff/0xfe), and the full byte range.
    let mut payload = vec![0x00, 0xff, 0xfe, 0x01];
    payload.extend((0u8..=255).collect::<Vec<u8>>());
    payload.extend_from_slice(nonce().as_bytes());

    mqtt.publish(TOPIC_BINARY, &payload, QoS::AtLeastOnce, false).await?;

    let msg = recv_pulsar_payload(&mut observer, &payload, RECV_TIMEOUT).await?;
    if msg.payload.data != payload {
        return Err(anyhow::anyhow!(
            "binary payload corrupted: sent {} bytes, received {} bytes",
            payload.len(),
            msg.payload.data.len()
        ));
    }
    ack_pulsar_message(&mut observer, &msg).await?;

    let _ = mqtt.disconnect().await;
    Ok(())
}

egress_case!(PulsarEgressBinaryPayloadTest, "pulsar_egress_payload_binary", run_binary_payload);

// ---------------------------------------------------------------------------
// EG-04 large payload integrity (64 KiB)
// ---------------------------------------------------------------------------

async fn run_large_payload(mqtt_addr: String) -> Result<(), anyhow::Error> {
    const SIZE: usize = 64 * 1024;

    let pulsar = connect_pulsar().await?;
    let mut observer = create_observer(&pulsar, EGRESS_TOPIC, &nonce()).await?;
    let mqtt = mqtt_v311(&mqtt_addr, &format!("egress-large-{}", nonce())).await?;

    let mut payload = Vec::with_capacity(SIZE + 64);
    payload.extend_from_slice(b"egress-large-");
    payload.extend_from_slice(nonce().as_bytes());
    while payload.len() < SIZE {
        payload.push((payload.len() % 251) as u8);
    }

    mqtt.publish(TOPIC_LARGE, &payload, QoS::AtLeastOnce, false).await?;

    let msg = recv_pulsar_payload(&mut observer, &payload, RECV_TIMEOUT).await?;
    if msg.payload.data != payload {
        return Err(anyhow::anyhow!(
            "large payload mismatch: sent {} bytes, received {} bytes",
            payload.len(),
            msg.payload.data.len()
        ));
    }
    ack_pulsar_message(&mut observer, &msg).await?;

    let _ = mqtt.disconnect().await;
    Ok(())
}

egress_case!(PulsarEgressLargePayloadTest, "pulsar_egress_payload_large", run_large_payload);

// ---------------------------------------------------------------------------
// EG-05 topic_filter scope: unmatched MQTT topics must not be forwarded
// ---------------------------------------------------------------------------

async fn run_topic_filter_scope(mqtt_addr: String) -> Result<(), anyhow::Error> {
    let pulsar = connect_pulsar().await?;
    let mut observer = create_observer(&pulsar, EGRESS_TOPIC, &nonce()).await?;
    let mqtt = mqtt_v311(&mqtt_addr, &format!("egress-scope-{}", nonce())).await?;

    // `pulsar/other/unmatched` does not match `pulsar/egress/#`.
    let payload = tagged_payload("egress-out-of-scope");
    mqtt.publish(TOPIC_UNMATCHED, &payload, QoS::AtLeastOnce, false).await?;

    expect_no_pulsar_message(&mut observer, NEGATIVE_TIMEOUT).await?;

    // Control: an in-scope topic right after the negative check *is* forwarded,
    // proving the observer subscription itself is healthy.
    let control = tagged_payload("egress-in-scope");
    mqtt.publish(TOPIC_CONTROL, &control, QoS::AtLeastOnce, false).await?;
    let msg = recv_pulsar_payload(&mut observer, &control, RECV_TIMEOUT).await?;
    ack_pulsar_message(&mut observer, &msg).await?;

    let _ = mqtt.disconnect().await;
    Ok(())
}

egress_case!(PulsarEgressTopicFilterScopeTest, "pulsar_egress_topic_filter_scope", run_topic_filter_scope);

// ---------------------------------------------------------------------------
// EG-06 forward_all_publish: dup / retain / qos properties
// ---------------------------------------------------------------------------

async fn run_forward_all_publish(mqtt_addr: String) -> Result<(), anyhow::Error> {
    let pulsar = connect_pulsar().await?;
    let mut observer = create_observer(&pulsar, EGRESS_TOPIC, &nonce()).await?;
    let mqtt = mqtt_v311(&mqtt_addr, &format!("egress-publish-{}", nonce())).await?;

    // Retained publish.
    let retained = tagged_payload("egress-retained");
    mqtt.publish(TOPIC_RETAIN, &retained, QoS::AtLeastOnce, true).await?;
    let msg = recv_pulsar_payload(&mut observer, &retained, RECV_TIMEOUT).await?;
    expect_property(&msg, "retain", "true")?;
    expect_property(&msg, "dup", "false")?;
    expect_property(&msg, "qos", "1")?;
    ack_pulsar_message(&mut observer, &msg).await?;

    // Non-retained publish on the same MQTT topic.
    let live = tagged_payload("egress-live");
    mqtt.publish(TOPIC_RETAIN, &live, QoS::AtLeastOnce, false).await?;
    let msg = recv_pulsar_payload(&mut observer, &live, RECV_TIMEOUT).await?;
    expect_property(&msg, "retain", "false")?;
    ack_pulsar_message(&mut observer, &msg).await?;

    let _ = mqtt.disconnect().await;
    Ok(())
}

egress_case!(PulsarEgressForwardAllPublishTest, "pulsar_egress_forward_all_publish", run_forward_all_publish);

// ---------------------------------------------------------------------------
// EG-07 forward_all_from: origin metadata properties
// ---------------------------------------------------------------------------

async fn run_forward_all_from(mqtt_addr: String) -> Result<(), anyhow::Error> {
    let pulsar = connect_pulsar().await?;
    let mut observer = create_observer(&pulsar, EGRESS_TOPIC, &nonce()).await?;

    let client_id = format!("egress-from-{}", nonce());
    let mqtt = mqtt_v311(&mqtt_addr, &client_id).await?;

    let payload = tagged_payload("egress-from");
    mqtt.publish(TOPIC_FROM, &payload, QoS::AtLeastOnce, false).await?;

    let msg = recv_pulsar_payload(&mut observer, &payload, RECV_TIMEOUT).await?;
    expect_property(&msg, "from_clientid", &client_id)?;
    // node.id = 1 in the pulsar suite config.
    expect_property(&msg, "from_node", "1")?;
    for key in ["from_type", "from_username"] {
        if property(&msg, key).is_none() {
            return Err(anyhow::anyhow!(
                "Pulsar property {key:?} missing with forward_all_from = true (all properties: {:?})",
                properties(&msg)
            ));
        }
    }
    ack_pulsar_message(&mut observer, &msg).await?;

    let _ = mqtt.disconnect().await;
    Ok(())
}

egress_case!(PulsarEgressForwardAllFromTest, "pulsar_egress_forward_all_from", run_forward_all_from);

// ---------------------------------------------------------------------------
// EG-08 skip_levels: the forwarded `topic` property is level-trimmed
// ---------------------------------------------------------------------------

async fn run_skip_levels(mqtt_addr: String) -> Result<(), anyhow::Error> {
    let pulsar = connect_pulsar().await?;
    let mut observer = create_observer(&pulsar, EGRESS_SKIP_TOPIC, &nonce()).await?;
    let mqtt = mqtt_v311(&mqtt_addr, &format!("egress-skip-{}", nonce())).await?;

    let payload = tagged_payload("egress-skip");
    mqtt.publish(TOPIC_SKIP_LEVELS, &payload, QoS::AtLeastOnce, false).await?;

    let msg = recv_pulsar_payload(&mut observer, &payload, RECV_TIMEOUT).await?;
    // `skip_levels = 2` drops the first two MQTT topic levels: the forwarded
    // `topic` property of `pulsar/skip/a/data` becomes `a/data` (the delivered
    // Pulsar topic is still `remote.topic`).
    expect_property(&msg, "topic", "a/data")?;
    ack_pulsar_message(&mut observer, &msg).await?;

    let _ = mqtt.disconnect().await;
    Ok(())
}

egress_case!(PulsarEgressSkipLevelsTest, "pulsar_egress_skip_levels", run_skip_levels);

// ---------------------------------------------------------------------------
// PS-02 one egress round trip used as a smoke check of the whole suite
// ---------------------------------------------------------------------------

async fn run_smoke(mqtt_addr: String) -> Result<(), anyhow::Error> {
    let pulsar = connect_pulsar().await?;
    let mut observer = create_observer(&pulsar, EGRESS_TOPIC, &nonce()).await?;
    let mqtt = mqtt_v311(&mqtt_addr, &format!("egress-smoke-{}", nonce())).await?;

    let payload = tagged_payload("egress-smoke");
    mqtt.publish(TOPIC_CONTROL, &payload, QoS::AtLeastOnce, false).await?;
    let msg = recv_pulsar_payload(&mut observer, &payload, RECV_TIMEOUT).await?;
    expect_property(&msg, "topic", TOPIC_CONTROL)?;
    ack_pulsar_message(&mut observer, &msg).await?;

    // The broker must stay healthy while the bridge plugins run.
    if !mqtt.is_connected() {
        return Err(anyhow::anyhow!("MQTT connection dropped while the egress bridge was active"));
    }
    let _ = mqtt.disconnect().await;
    Ok(())
}

egress_case!(PulsarEgressSmokeTest, "pulsar_bridge_smoke_egress", run_smoke);
