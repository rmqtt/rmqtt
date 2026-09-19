//! Ingress direction tests: Pulsar message -> MQTT publish.
//!
//! Every case injects messages with its own Pulsar producer (the independent
//! observer on the *other* side) and asserts what an MQTT client receives.
//!
//! Ordering rule inside each case: the MQTT subscription must be established
//! (SUBACK received) *before* the Pulsar message is produced, otherwise the
//! broker has no subscriber to deliver to.
//!
//! Non-matching deliveries (for example a retained message left behind by
//! IG-04, or a previous run) are skipped, never treated as a failure: every
//! assertion also matches on a unique nonce carried in the payload or in a
//! Pulsar message property.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::common::{
    connect_pulsar, create_producer, mqtt_v311, mqtt_v5, nonce, publish_pulsar_message, run_async,
    run_async_note, tagged_payload, INGRESS_PH_TOPIC, INGRESS_TOPIC, MQTT_INGRESS_TOPIC, RECV_TIMEOUT,
};
use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v311::client::IncomingMessage as IncomingV311;
use crate::mqtt::v5::client::IncomingMessage as IncomingV5;

/// Declares an ingress test case (struct + `TestCase` impl) over an async body.
macro_rules! ingress_case {
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

/// Builds the Pulsar message properties for one case.
fn props(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect()
}

/// Declares an ingress test case whose body reports an *observation* note
/// (see [`run_async_note`]): the test still fails on hard errors, but a
/// non-required behaviour (e.g. ordering) is surfaced as a note.
macro_rules! ingress_case_note {
    ($ty:ident, $case:literal, $body:ident) => {
        pub struct $ty;

        impl TestCase for $ty {
            fn name(&self) -> &str {
                $case
            }

            fn execute(&self, ctx: &mut TestContext) -> TestResult {
                let mqtt_addr = ctx.config.broker_addr.clone();
                run_async_note(self.name(), mqtt_addr, |addr| $body(addr))
            }

            fn timeout(&self) -> Duration {
                Duration::from_secs(120)
            }
        }
    };
}

/// Receives v3.1.1 deliveries until one carries exactly `expected`.
async fn recv_v311_until(
    client: &mut crate::mqtt::v311::MqttV311Client,
    expected: &[u8],
    timeout: Duration,
) -> Result<IncomingV311, anyhow::Error> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(anyhow::anyhow!(
                "timed out after {timeout:?} waiting for an MQTT delivery matching the expected \
                 payload ({} bytes)",
                expected.len()
            ));
        }
        match client.recv_message_timeout(remaining).await {
            Some(msg) if msg.payload.as_ref() == expected => return Ok(msg),
            // Unrelated delivery (retained leftovers): skip it.
            Some(_) => continue,
            None => {
                return Err(anyhow::anyhow!(
                    "timed out waiting for an MQTT delivery matching the expected payload"
                ))
            }
        }
    }
}

/// Receives v5 deliveries until one carries the `marker` user property.
///
/// Used by the empty-payload case, where a payload match is impossible.
async fn recv_v5_until_marker(
    client: &mut crate::mqtt::v5::MqttV5Client,
    marker: &str,
    timeout: Duration,
) -> Result<IncomingV5, anyhow::Error> {
    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(anyhow::anyhow!(
                "timed out after {timeout:?} waiting for an MQTT v5 delivery carrying the marker \
                 user property {marker:?}"
            ));
        }
        match client.recv_message_timeout(remaining).await {
            Some(msg) if user_property(&msg, "marker").as_deref() == Some(marker) => return Ok(msg),
            Some(_) => continue,
            None => return Err(anyhow::anyhow!("timed out waiting for the marked MQTT delivery")),
        }
    }
}

/// Value of an MQTT v5 user property, if present.
fn user_property(msg: &IncomingV5, key: &str) -> Option<String> {
    msg.user_properties.iter().find(|(k, _)| &**k == key).map(|(_, v)| v.to_string())
}

// ---------------------------------------------------------------------------
// IG-01 basic receive (Pulsar -> MQTT)
// ---------------------------------------------------------------------------

async fn run_basic(mqtt_addr: String) -> Result<(), anyhow::Error> {
    let pulsar = connect_pulsar().await?;
    let mut producer = create_producer(&pulsar, INGRESS_TOPIC).await?;
    let mut mqtt = mqtt_v311(&mqtt_addr, &format!("ingress-basic-{}", nonce())).await?;
    mqtt.subscribe(MQTT_INGRESS_TOPIC, QoS::AtLeastOnce).await?;

    let payload = tagged_payload("ingress-basic");
    publish_pulsar_message(&mut producer, &payload, props(&[("qos", "1")])).await?;

    let msg = recv_v311_until(&mut mqtt, &payload, RECV_TIMEOUT).await?;
    if &*msg.topic != MQTT_INGRESS_TOPIC {
        return Err(anyhow::anyhow!(
            "message delivered to {:?}, expected {:?}",
            msg.topic,
            MQTT_INGRESS_TOPIC
        ));
    }

    let _ = mqtt.disconnect().await;
    Ok(())
}

ingress_case!(PulsarIngressBasicTest, "pulsar_ingress_basic", run_basic);

// ---------------------------------------------------------------------------
// IG-02 Pulsar properties -> MQTT v5 User Properties
// ---------------------------------------------------------------------------

async fn run_user_properties(mqtt_addr: String) -> Result<(), anyhow::Error> {
    let pulsar = connect_pulsar().await?;
    let mut producer = create_producer(&pulsar, INGRESS_TOPIC).await?;
    let mut mqtt = mqtt_v5(&mqtt_addr, &format!("ingress-props-{}", nonce())).await?;
    mqtt.subscribe(MQTT_INGRESS_TOPIC, QoS::AtLeastOnce).await?;

    let marker = nonce();
    let payload = tagged_payload("ingress-props");
    publish_pulsar_message(
        &mut producer,
        &payload,
        // `qos` is a reserved key (sets the delivery QoS), `custom` must be
        // forwarded verbatim as an MQTT v5 User Property.
        props(&[("qos", "1"), ("custom", "value-42"), ("marker", &marker)]),
    )
    .await?;

    let msg = recv_v5_until_marker(&mut mqtt, &marker, RECV_TIMEOUT).await?;
    if let Some(custom) = user_property(&msg, "custom") {
        if custom != "value-42" {
            return Err(anyhow::anyhow!("user property custom = {custom:?}, expected \"value-42\""));
        }
    } else {
        return Err(anyhow::anyhow!(
            "Pulsar property `custom` was not forwarded as an MQTT v5 User Property \
             (received: {:?})",
            msg.user_properties
        ));
    }

    let _ = mqtt.disconnect().await;
    Ok(())
}

ingress_case!(PulsarIngressUserPropertiesTest, "pulsar_ingress_user_properties", run_user_properties);

// ---------------------------------------------------------------------------
// IG-03 `qos` property drives the delivery QoS (local.qos unset)
// ---------------------------------------------------------------------------

async fn run_qos_property(mqtt_addr: String) -> Result<(), anyhow::Error> {
    let pulsar = connect_pulsar().await?;
    let mut producer = create_producer(&pulsar, INGRESS_TOPIC).await?;
    let mut mqtt = mqtt_v5(&mqtt_addr, &format!("ingress-qos-{}", nonce())).await?;
    mqtt.subscribe(MQTT_INGRESS_TOPIC, QoS::AtLeastOnce).await?;

    // properties qos = 1 -> delivered as QoS 1.
    let marker_q1 = nonce();
    let payload_q1 = tagged_payload("ingress-qos1");
    publish_pulsar_message(&mut producer, &payload_q1, props(&[("qos", "1"), ("marker", &marker_q1)]))
        .await?;
    let msg = recv_v5_until_marker(&mut mqtt, &marker_q1, RECV_TIMEOUT).await?;
    if msg.qos != QoS::AtLeastOnce {
        return Err(anyhow::anyhow!("properties qos=1 delivered as {:?}, expected AtLeastOnce", msg.qos));
    }

    // properties qos = 0 -> delivered as QoS 0.
    let marker_q0 = nonce();
    let payload_q0 = tagged_payload("ingress-qos0");
    publish_pulsar_message(&mut producer, &payload_q0, props(&[("qos", "0"), ("marker", &marker_q0)]))
        .await?;
    let msg = recv_v5_until_marker(&mut mqtt, &marker_q0, RECV_TIMEOUT).await?;
    if msg.qos != QoS::AtMostOnce {
        return Err(anyhow::anyhow!("properties qos=0 delivered as {:?}, expected AtMostOnce", msg.qos));
    }

    let _ = mqtt.disconnect().await;
    Ok(())
}

ingress_case!(PulsarIngressQosPropertyTest, "pulsar_ingress_qos_property", run_qos_property);

// ---------------------------------------------------------------------------
// IG-04 `retain` property -> retained MQTT message (needs rmqtt-retainer)
// ---------------------------------------------------------------------------

async fn run_retain_property(mqtt_addr: String) -> Result<(), anyhow::Error> {
    let pulsar = connect_pulsar().await?;
    let mut producer = create_producer(&pulsar, INGRESS_TOPIC).await?;

    // Publish the retained message while no subscriber is attached.
    let payload = tagged_payload("ingress-retained");
    publish_pulsar_message(&mut producer, &payload, props(&[("qos", "1"), ("retain", "true")])).await?;

    // A subscriber arriving later must receive it as a retained delivery.
    let mut mqtt = mqtt_v311(&mqtt_addr, &format!("ingress-retain-{}", nonce())).await?;
    mqtt.subscribe(MQTT_INGRESS_TOPIC, QoS::AtLeastOnce).await?;
    let msg = recv_v311_until(&mut mqtt, &payload, RECV_TIMEOUT).await?;
    if !msg.retain {
        return Err(anyhow::anyhow!(
            "delivery has retain = false, expected a retained message (properties retain=true)"
        ));
    }
    let _ = mqtt.disconnect().await;

    // Clean up: an empty retained publish removes the retained message so it
    // cannot pollute later runs (payload is empty + properties retain=true).
    publish_pulsar_message(&mut producer, &[], props(&[("retain", "true")])).await?;
    Ok(())
}

ingress_case!(PulsarIngressRetainPropertyTest, "pulsar_ingress_retain_property", run_retain_property);

// ---------------------------------------------------------------------------
// IG-05 ordering observation: 20 consecutive messages
//
// Integrity (all 20 delivered exactly once) is a hard assertion; whether the
// delivery order is preserved is reported as an observation note, because the
// MQTT specification does not require cross-message ordering guarantees.
// ---------------------------------------------------------------------------

async fn run_ordering(mqtt_addr: String) -> Result<Option<String>, anyhow::Error> {
    const COUNT: usize = 20;

    let pulsar = connect_pulsar().await?;
    let mut producer = create_producer(&pulsar, INGRESS_TOPIC).await?;
    let mut mqtt = mqtt_v311(&mqtt_addr, &format!("ingress-order-{}", nonce())).await?;
    mqtt.subscribe(MQTT_INGRESS_TOPIC, QoS::AtLeastOnce).await?;

    let run = nonce();
    let mut expected = Vec::with_capacity(COUNT);
    for i in 0..COUNT {
        let payload = format!("order-{i}-{run}").into_bytes();
        publish_pulsar_message(&mut producer, &payload, props(&[("qos", "1")])).await?;
        expected.push(payload);
    }

    // Collect exactly COUNT deliveries belonging to this run.
    let mut received: Vec<Vec<u8>> = Vec::with_capacity(COUNT);
    let deadline = Instant::now() + RECV_TIMEOUT;
    let marker = "order-";
    while received.len() < COUNT {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(anyhow::anyhow!("timed out after receiving {}/{COUNT} messages", received.len()));
        }
        match mqtt.recv_message_timeout(remaining).await {
            Some(msg) => {
                let payload = msg.payload.to_vec();
                if payload.starts_with(marker.as_bytes()) && payload.ends_with(run.as_bytes()) {
                    received.push(payload);
                }
            }
            None => {
                return Err(anyhow::anyhow!("timed out after receiving {}/{COUNT} messages", received.len()))
            }
        }
    }
    let _ = mqtt.disconnect().await;

    // Hard assertion: no loss, no duplication.
    let mut sorted_received = received.clone();
    sorted_received.sort();
    let mut sorted_expected = expected.clone();
    sorted_expected.sort();
    if sorted_received != sorted_expected {
        return Err(anyhow::anyhow!(
            "delivered message set differs from the published set (loss or duplication): \
             sent {COUNT}, received {}",
            received.len()
        ));
    }

    // Observation: is the delivery order preserved?
    let mismatches = received.iter().zip(expected.iter()).filter(|(got, want)| got != want).count();
    if mismatches == 0 {
        return Ok(None);
    }
    let divergence = received.iter().zip(expected.iter()).position(|(got, want)| got != want);
    match divergence {
        Some(idx) => Ok(Some(format!(
            "OBSERVATION: {mismatches}/{COUNT} deliveries arrived out of the send order; first \
             divergence at index {idx}: received {:?}, expected {:?}. All {COUNT} messages were \
             delivered exactly once (no loss, no duplication).",
            String::from_utf8_lossy(&received[idx]),
            String::from_utf8_lossy(&expected[idx]),
        ))),
        None => Ok(None),
    }
}

ingress_case_note!(PulsarIngressOrderingTest, "pulsar_ingress_ordering", run_ordering);

// ---------------------------------------------------------------------------
// IG-06 binary payload integrity
// ---------------------------------------------------------------------------

async fn run_binary_payload(mqtt_addr: String) -> Result<(), anyhow::Error> {
    let pulsar = connect_pulsar().await?;
    let mut producer = create_producer(&pulsar, INGRESS_TOPIC).await?;
    let mut mqtt = mqtt_v311(&mqtt_addr, &format!("ingress-binary-{}", nonce())).await?;
    mqtt.subscribe(MQTT_INGRESS_TOPIC, QoS::AtLeastOnce).await?;

    let mut payload = vec![0x00, 0xff, 0xfe, 0x01];
    payload.extend((0u8..=255).collect::<Vec<u8>>());
    payload.extend_from_slice(nonce().as_bytes());

    publish_pulsar_message(&mut producer, &payload, props(&[("qos", "1")])).await?;
    let msg = recv_v311_until(&mut mqtt, &payload, RECV_TIMEOUT).await?;
    if msg.payload.as_ref() != payload.as_slice() {
        return Err(anyhow::anyhow!(
            "binary payload corrupted: sent {} bytes, received {} bytes",
            payload.len(),
            msg.payload.len()
        ));
    }

    let _ = mqtt.disconnect().await;
    Ok(())
}

ingress_case!(PulsarIngressBinaryPayloadTest, "pulsar_ingress_payload_binary", run_binary_payload);

// ---------------------------------------------------------------------------
// IG-07 empty payload (local.allow_empty_forward defaults to true)
// ---------------------------------------------------------------------------

async fn run_empty_payload(mqtt_addr: String) -> Result<(), anyhow::Error> {
    let pulsar = connect_pulsar().await?;
    let mut producer = create_producer(&pulsar, INGRESS_TOPIC).await?;
    let mut mqtt = mqtt_v5(&mqtt_addr, &format!("ingress-empty-{}", nonce())).await?;
    mqtt.subscribe(MQTT_INGRESS_TOPIC, QoS::AtLeastOnce).await?;

    // An empty payload carries no nonce, so the delivery is identified by a
    // marker user property instead.
    let marker = nonce();
    publish_pulsar_message(&mut producer, &[], props(&[("qos", "1"), ("marker", &marker)])).await?;

    let msg = recv_v5_until_marker(&mut mqtt, &marker, RECV_TIMEOUT).await?;
    if !msg.payload.is_empty() {
        return Err(anyhow::anyhow!(
            "empty payload was forwarded as {} bytes: {:?}",
            msg.payload.len(),
            String::from_utf8_lossy(&msg.payload)
        ));
    }

    let _ = mqtt.disconnect().await;
    Ok(())
}

ingress_case!(PulsarIngressEmptyPayloadTest, "pulsar_ingress_empty_payload", run_empty_payload);

// ---------------------------------------------------------------------------
// IG-08 `${remote.topic}` placeholder: MQTT topic == full Pulsar topic
// ---------------------------------------------------------------------------

async fn run_remote_topic_placeholder(mqtt_addr: String) -> Result<(), anyhow::Error> {
    let pulsar = connect_pulsar().await?;
    let mut producer = create_producer(&pulsar, INGRESS_PH_TOPIC).await?;
    let mut mqtt = mqtt_v311(&mqtt_addr, &format!("ingress-ph-{}", nonce())).await?;
    // The plugin maps the message to `${remote.topic}`, i.e. the MQTT topic is
    // the full Pulsar topic string.
    mqtt.subscribe(INGRESS_PH_TOPIC, QoS::AtLeastOnce).await?;

    let payload = tagged_payload("ingress-ph");
    publish_pulsar_message(&mut producer, &payload, props(&[("qos", "1")])).await?;

    let msg = recv_v311_until(&mut mqtt, &payload, RECV_TIMEOUT).await?;
    if &*msg.topic != INGRESS_PH_TOPIC {
        return Err(anyhow::anyhow!(
            "message delivered to {:?}, expected the placeholder target {:?}",
            msg.topic,
            INGRESS_PH_TOPIC
        ));
    }

    let _ = mqtt.disconnect().await;
    Ok(())
}

ingress_case!(
    PulsarIngressRemoteTopicPlaceholderTest,
    "pulsar_ingress_remote_topic_placeholder",
    run_remote_topic_placeholder
);

// ---------------------------------------------------------------------------
// PS-01 service probe: records the environment state explicitly
// ---------------------------------------------------------------------------

async fn run_service_probe(mqtt_addr: String) -> Result<(), anyhow::Error> {
    // Reached only when the Pulsar probe succeeded (see `run_async`), so the
    // client must be able to build and look up the ingress topic.
    let pulsar = connect_pulsar().await?;
    let producer = create_producer(&pulsar, INGRESS_TOPIC).await?;
    drop(producer);

    // The broker under test must be reachable as well.
    let mqtt = mqtt_v311(&mqtt_addr, &format!("pulsar-probe-{}", nonce())).await?;
    if !mqtt.is_connected() {
        return Err(anyhow::anyhow!("MQTT broker not reachable at {mqtt_addr}"));
    }
    let _ = mqtt.disconnect().await;
    Ok(())
}

ingress_case!(PulsarServiceProbeTest, "pulsar_service_probe", run_service_probe);
