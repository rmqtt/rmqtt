//! Message lifetime regressions reported in GitHub issue #513
//! (<https://github.com/rmqtt/rmqtt/issues/513>).
//!
//! Three independent defects in how the broker handles the lifetime of a
//! stored or queued Application Message. Each test is written against the
//! MQTT 5.0 requirement it covers, so it FAILED on the broker as reported
//! (reproducing the issue) and PASSES once the defect is fixed. Every test
//! carries its own control arm, mirroring the reproducers attached to the
//! issue, so a failure is attributed to the defect under test instead of to
//! unrelated behaviour: whenever a control fails, the test says so explicitly
//! and declines to claim a reproduction.
//!
//! ## 1. `retained_message_expiry_not_decremented_v5` — [MQTT-3.3.2-6]
//!
//! "The PUBLISH packet sent to a Client by the Server MUST contain a Message
//! Expiry Interval set to the received value minus the time that the
//! Application Message has been waiting in the Server."
//!
//! `_send_retain_messages` (`rmqtt/src/session.rs`) used to rewrite
//! `publish.create_time` to "now" on every retained delivery. The expiry
//! arithmetic in `message_expiry_check` (`rmqtt/src/hook.rs`) is
//! `remaining = now - create_time`, so after the rewrite it always computed a
//! remaining interval equal to the original value: the time the message spent
//! in the retain store was never subtracted. The rewrite is gone, because the
//! timestamp stamped when the server first received the PUBLISH is the only
//! clock available on this path — the storage backend keeps the message
//! verbatim and, under the default `retained_message_ttl = "0m"`, holds no
//! deadline of its own to consult instead. PASSES.
//!
//! CONTROL — the same property on a live delivery carries approximately the
//! published value, proving the broker relays the property at all. An
//! unchanged value in the arm is therefore a storage-path defect.
//!
//! ## 2. `message_expiry_deletes_qos2_inflight_v5` — [MQTT-4.3.3-7] / [MQTT-4.4.0-1]
//!
//! The Message Expiry Interval governs an Application Message the Server is
//! *holding*. Once the receiver has answered PUBREC the delivery is under way:
//! [MQTT-4.3.3-7] states that the sender "MUST NOT apply Application Message
//! expiry if a PUBLISH packet has been sent", [MQTT-3.3.2-5] only permits
//! deleting a copy whose onward delivery has *not* started, and section 4.4
//! requires the owed PUBREL to be re-sent on the resumed Session using the
//! original Packet Identifier.
//!
//! `Session::reforward` (`rmqtt/src/session.rs`) used to run the expiry check
//! first in its `MomentStatus::UnComplete` branch and return early when it
//! reported expiry: the owed PUBREL was never sent, the message was not
//! re-registered in the outbound inflight window, and — unlike the
//! `Session::deliver` path — no `message_dropped` hook was raised either, so
//! the loss was silent. The gate is gone; the branch now always re-sends the
//! PUBREL, which also covers the same drop on the online retry path
//! (`deliver_timeout_delay` → `pop_front_timeout` → `reforward`). PASSES.
//!
//! CONTROL — the identical cut on a message published with NO Message Expiry
//! Interval still gets its PUBREL resent, so the loss of the arm was
//! attributable to the expiry mechanism rather than to session recovery in
//! general.
//!
//! ## 3. `oversized_queued_message_stalls_queue_v5` — [MQTT-3.1.2.11.4]
//!
//! "Where a Packet is too large to send, the Server MUST discard it without
//! sending it and then behave as if it had completed sending that Application
//! Message." The queue must therefore keep moving and the connection must stay
//! usable.
//!
//! A queued message larger than the client's Maximum Packet Size makes the v5
//! codec return `EncodeError::OverMaxPacketSize`
//! (`rmqtt-codec/src/v5/codec.rs`). That error is propagated with `?` through
//! `Sink::publish` and `Session::deliver` out of the session event loop
//! (`rmqtt/src/session.rs`), which closes the connection: the oversized message
//! is lost and every message behind it in the queue is never delivered.
//!
//! CONTROL — a second identity with an identical queue resuming with a large
//! Maximum Packet Size receives both messages, proving they were queued.
//!
//! ## 4. `retained_oversized_message_keeps_session_v5` — [MQTT-3.1.2-25]
//!
//! The retained-message path reaches the same `Session::deliver` as a queued
//! message, via `entry.publish` (`_send_retain_messages` in
//! `rmqtt/src/session.rs`). A retained Application Message is not consumed by
//! a delivery, so before the fix a client that declared a Maximum Packet Size
//! below the retained payload was disconnected on *every* SUBSCRIBE —
//! reconnect, subscribe, disconnect, forever.
//!
//! CONTROL — the same retained message, subscribed to by a client declaring a
//! large Maximum Packet Size, arrives intact: proof that it really is in the
//! retain store and that the delivery path itself works.
//!
//! ARM — a client declaring a Maximum Packet Size of 1024 must (a) receive the
//! SUBACK, (b) receive no packet above 1024 bytes — the oversized message is
//! discarded rather than sent — and (c) end up with a *usable* connection,
//! proven by completing a second SUBSCRIBE round trip on it.

use std::time::{Duration, Instant};

use rmqtt_codec::v5::SubscribeAckReason;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v5::MqttV5Client;

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// CONNECT with the option set the three reproducers need; everything else
/// keeps the client defaults (keep alive 60 s, no Will, no authentication).
async fn connect(
    addr: &str,
    client_id: &str,
    connect_timeout: Duration,
    clean_start: bool,
    session_expiry_interval: Option<u32>,
    max_packet_size: Option<u32>,
) -> anyhow::Result<MqttV5Client> {
    MqttV5Client::connect_with_options(
        addr,
        client_id,
        connect_timeout,
        clean_start,
        60,
        None,
        None,
        None,
        session_expiry_interval,
        None,
        max_packet_size,
    )
    .await
}

/// Whether every reason code in a SUBACK grants the subscription.
fn granted(status: &[SubscribeAckReason]) -> bool {
    !status.is_empty()
        && status.iter().all(|s| {
            matches!(
                s,
                SubscribeAckReason::GrantedQos0
                    | SubscribeAckReason::GrantedQos1
                    | SubscribeAckReason::GrantedQos2
            )
        })
}

fn describe_status(status: &[SubscribeAckReason]) -> String {
    format!("{status:?}")
}

/// PUBLISH at QoS 1 and wait for the PUBACK, returning its reason code so a
/// rejected publication surfaces as a precondition failure.
async fn publish_qos1(
    client: &mut MqttV5Client,
    topic: &str,
    payload: &[u8],
    retain: bool,
    message_expiry_interval: Option<u32>,
) -> anyhow::Result<u8> {
    client
        .publish_with_properties(
            topic,
            payload,
            QoS::AtLeastOnce,
            retain,
            None,
            message_expiry_interval,
            None,
            None,
            None,
            None,
        )
        .await?;
    let (_pid, reason_code) = client
        .recv_puback_reason(Duration::from_secs(5))
        .await
        .ok_or_else(|| anyhow::anyhow!("no PUBACK for the publish to {topic}"))?;
    Ok(reason_code)
}

/// Collect incoming payloads for up to `budget`, stopping early when the
/// broker closes the connection (which is itself one of the observed
/// symptoms of defect 3).
async fn drain_payloads(client: &mut MqttV5Client, budget: Duration) -> Vec<Vec<u8>> {
    let mut received = Vec::new();
    let deadline = Instant::now() + budget;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let slice = left.min(Duration::from_millis(500));
        let next = client.recv_message_timeout(slice).await;
        match next {
            Some(msg) => received.push(msg.payload.to_vec()),
            None => {
                if !client.is_connected() {
                    break;
                }
            }
        }
    }
    received
}

// ---------------------------------------------------------------------------
// 1. H-13 — retained Message Expiry Interval is not decremented
// ---------------------------------------------------------------------------

/// Message Expiry Interval published with the retained message.
const RETAINED_EXPIRY_SECS: u32 = 60;
/// How long the retained message is left in the store before read-back.
const RETAINED_HOLD_SECS: u64 = 20;
/// A conformant broker reports at most `RETAINED_EXPIRY_SECS - RETAINED_HOLD_SECS`
/// (40 s) plus rounding slack; the current broker reports the untouched 60 s.
const RETAINED_MAX_REMAINING: u32 = 45;

pub struct RetainedMessageExpiryNotDecrementedV5Test;

impl TestCase for RetainedMessageExpiryNotDecrementedV5Test {
    fn name(&self) -> &str {
        "retained_message_expiry_not_decremented_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        if let Some(r) = ctx.guard_retain_required(self.name(), "functional_v5", start) {
            return r;
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let live_topic = format!("issue513/retained/{uid}/live");
            let retained_topic = format!("issue513/retained/{uid}/stored");
            let addr = ctx.config.broker_addr.clone();
            let timeout = ctx.config.connect_timeout;

            // ---- CONTROL: an immediate live delivery carries the property.
            let mut live_sub =
                connect(&addr, &format!("issue513-ctl-sub-{uid}"), timeout, true, None, None).await?;
            let sub = live_sub.subscribe(&live_topic, QoS::AtLeastOnce).await?;
            if !granted(&sub.status) {
                return Err(anyhow::anyhow!(
                    "control: SUBACK refused the filter ({})",
                    describe_status(&sub.status)
                ));
            }
            // SUBACK says the filter was accepted, not that routing already
            // matches it; this pause is setup, never a measured deadline.
            tokio::time::sleep(Duration::from_millis(100)).await;

            let mut publisher =
                connect(&addr, &format!("issue513-pub-{uid}"), timeout, true, None, None).await?;
            let ack =
                publish_qos1(&mut publisher, &live_topic, b"live", false, Some(RETAINED_EXPIRY_SECS)).await?;
            if ack >= 0x80 {
                return Err(anyhow::anyhow!(
                    "control: the live publication was refused (PUBACK 0x{ack:02x})"
                ));
            }

            let live_val = live_sub
                .recv_message_timeout(Duration::from_secs(5))
                .await
                .and_then(|m| m.message_expiry_interval.map(|v| v.get()));
            let live_val = live_val.ok_or_else(|| {
                anyhow::anyhow!(
                    "control failed: the broker forwards no Message Expiry Interval even on a live \
                     delivery, so the retained read-back cannot be judged on it"
                )
            })?;
            if live_val > RETAINED_EXPIRY_SECS {
                return Err(anyhow::anyhow!(
                    "control failed: the live delivery carried Message Expiry Interval {live_val}, \
                     above the published {RETAINED_EXPIRY_SECS}"
                ));
            }
            let _ = live_sub.disconnect().await;

            // ---- ARM: retain, wait, read back with a fresh subscriber.
            let ack =
                publish_qos1(&mut publisher, &retained_topic, b"retained", true, Some(RETAINED_EXPIRY_SECS))
                    .await?;
            let _ = publisher.disconnect().await;
            if ack >= 0x80 {
                return Err(anyhow::anyhow!("the retained publication was refused (PUBACK 0x{ack:02x})"));
            }

            tokio::time::sleep(Duration::from_secs(RETAINED_HOLD_SECS)).await;

            let mut reader =
                connect(&addr, &format!("issue513-arm-sub-{uid}"), timeout, true, None, None).await?;
            let sub = reader.subscribe(&retained_topic, QoS::AtLeastOnce).await?;
            if !granted(&sub.status) {
                return Err(anyhow::anyhow!(
                    "arm: SUBACK refused the filter ({})",
                    describe_status(&sub.status)
                ));
            }

            let arm_val = match reader.recv_message_timeout(Duration::from_secs(5)).await {
                Some(m) if m.payload.as_ref() == b"retained" => m.message_expiry_interval.map(|v| v.get()),
                Some(m) => {
                    return Err(anyhow::anyhow!(
                        "arm: unexpected payload {:?} (expected the retained message)",
                        String::from_utf8_lossy(&m.payload)
                    ));
                }
                None => {
                    return Err(anyhow::anyhow!(
                        "arm: no retained message came back at all; that is a different defect"
                    ));
                }
            };
            let _ = reader.disconnect().await;

            match arm_val {
                Some(v) if v <= RETAINED_MAX_REMAINING => Ok(()),
                Some(v) => Err(anyhow::anyhow!(
                    "BUG REPRODUCED [MQTT-3.3.2-6]: retained message published with Message Expiry \
                     Interval {RETAINED_EXPIRY_SECS} and held {RETAINED_HOLD_SECS} s. \
                     live control delivery: {live_val}. retained read-back: {v}. \
                     A decrementing store returns at most {} — the received value minus the time \
                     the message waited. `_send_retain_messages` must not stamp \
                     `publish.create_time` with the delivery time, or `message_expiry_check` \
                     subtracts nothing",
                    RETAINED_EXPIRY_SECS as u64 - RETAINED_HOLD_SECS
                )),
                None => Err(anyhow::anyhow!(
                    "BUG REPRODUCED [MQTT-3.3.2-6]: the retained read-back dropped the Message \
                     Expiry Interval entirely (live control delivery carried {live_val})"
                )),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        // 20 s hold plus connection setup and retainer batching.
        Duration::from_secs(120)
    }
}

// ---------------------------------------------------------------------------
// 2. X-02 — Message Expiry deletes an in-flight QoS 2 exchange
// ---------------------------------------------------------------------------

const INFLIGHT_EXPIRY_SECS: u32 = 5;
const INFLIGHT_WAIT_SECS: u64 = 12;

/// State captured while the QoS 2 exchange is still incomplete, i.e. after the
/// subscriber answered PUBREC (so the broker owes a PUBREL) but before PUBCOMP.
struct InflightArm {
    client_id: String,
    pubrel_packet_id: u16,
}

/// Start one arm of the reproducer and cut it abruptly right after the broker
/// has sent its PUBREL, leaving the exchange incomplete.
async fn start_inflight_arm(
    addr: &str,
    connect_timeout: Duration,
    uid: &str,
    label: &str,
    message_expiry_interval: Option<u32>,
) -> anyhow::Result<InflightArm> {
    let topic = format!("issue513/inflight/{uid}/{label}");
    let client_id = format!("issue513-inflight-sub-{uid}-{label}");
    let payload = format!("inflight-{uid}-{label}").into_bytes();

    // Persistent session with a QoS 2 subscription.
    let mut subscriber = connect(addr, &client_id, connect_timeout, false, Some(300), None).await?;
    let sub = subscriber.subscribe(&topic, QoS::ExactlyOnce).await?;
    if !granted(&sub.status) {
        return Err(anyhow::anyhow!("{label}: QoS 2 subscribe refused ({})", describe_status(&sub.status)));
    }
    // Keep the exchange incomplete: never answer an incoming PUBREL with
    // PUBCOMP. Set before any delivery so the reader loop cannot auto-complete
    // the exchange in the meantime.
    subscriber.set_auto_pubcomp(false);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Publish the message and finish the publisher-side QoS 2 handshake so the
    // broker's inbound exchange does not linger.
    let mut publisher =
        connect(addr, &format!("issue513-inflight-pub-{uid}-{label}"), connect_timeout, true, None, None)
            .await?;
    publisher
        .publish_with_properties(
            &topic,
            &payload,
            QoS::ExactlyOnce,
            false,
            None,
            message_expiry_interval,
            None,
            None,
            None,
            None,
        )
        .await?;
    let (pub_packet_id, _reason) = publisher
        .recv_pubrec_reason(Duration::from_secs(5))
        .await
        .ok_or_else(|| anyhow::anyhow!("{label}: no PUBREC for our PUBLISH"))?;
    publisher.send_pubrel(pub_packet_id).await?;
    let _ = publisher.disconnect().await;

    // Subscriber: the PUBLISH (the client auto-replies PUBREC), then the
    // broker's PUBREL — proof that the exchange reached UnComplete.
    let msg = subscriber
        .recv_message_timeout(Duration::from_secs(5))
        .await
        .ok_or_else(|| anyhow::anyhow!("{label}: no QoS 2 PUBLISH delivered"))?;
    if msg.payload.as_ref() != payload.as_slice() {
        return Err(anyhow::anyhow!("{label}: unexpected payload {:?}", msg.payload));
    }
    let pubrel_packet_id = subscriber
        .recv_pubrel_timeout(Duration::from_secs(5))
        .await
        .ok_or_else(|| anyhow::anyhow!("{label}: no PUBREL after PUBREC"))?;

    // Abrupt close without PUBCOMP, leaving the exchange incomplete.
    let _ = subscriber.abort_connection().await;

    Ok(InflightArm { client_id, pubrel_packet_id })
}

/// Resume the session and report whether the owed PUBREL came back.
async fn resume_inflight_arm(
    addr: &str,
    connect_timeout: Duration,
    arm: &InflightArm,
) -> anyhow::Result<(bool, Option<u16>)> {
    let mut resumed = connect(addr, &arm.client_id, connect_timeout, false, Some(300), None).await?;
    let session_present = resumed.connack().session_present;
    let pubrel = resumed.recv_pubrel_timeout(Duration::from_secs(8)).await;
    let _ = resumed.disconnect().await;
    Ok((session_present, pubrel))
}

pub struct MessageExpiryDeletesQos2InflightV5Test;

impl TestCase for MessageExpiryDeletesQos2InflightV5Test {
    fn name(&self) -> &str {
        "message_expiry_deletes_qos2_inflight_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let addr = ctx.config.broker_addr.clone();
            let timeout = ctx.config.connect_timeout;

            // ---- start both exchanges, then cut them after PUBREC
            let control = start_inflight_arm(&addr, timeout, &uid, "control", None).await?;
            let arm = start_inflight_arm(&addr, timeout, &uid, "arm", Some(INFLIGHT_EXPIRY_SECS)).await?;

            // Past the Message Expiry Interval of the arm. The wait is shared
            // so the control arm costs no extra time.
            tokio::time::sleep(Duration::from_secs(INFLIGHT_WAIT_SECS)).await;

            // ---- CONTROL resume (the message had no Message Expiry Interval)
            let (control_session_present, control_pubrel) =
                resume_inflight_arm(&addr, timeout, &control).await?;

            // ---- ARM resume (Message Expiry Interval elapsed while offline)
            let (arm_session_present, arm_pubrel) = resume_inflight_arm(&addr, timeout, &arm).await?;

            if !control_session_present {
                return Err(anyhow::anyhow!(
                    "control failed: the session was not resumed (session_present=0), so the \
                     recovery path under test was never reached"
                ));
            }
            match control_pubrel {
                Some(_) => {}
                None => {
                    return Err(anyhow::anyhow!(
                        "control failed: the owed PUBREL was not resent even WITHOUT a Message \
                         Expiry Interval, so the loss is not attributable to expiry — that is a \
                         separate [MQTT-4.4.0-1] problem (see qos2_pubrel_resend_on_resume)"
                    ));
                }
            }

            // [MQTT-4.4.0-1] also requires the original Packet Identifier.
            match (arm_session_present, arm_pubrel) {
                (true, Some(pid)) if pid == arm.pubrel_packet_id => Ok(()),
                (true, Some(pid)) => Err(anyhow::anyhow!(
                    "PUBREL resent with packet id {pid} on resume, expected the original {} \
                     [MQTT-4.4.0-1]",
                    arm.pubrel_packet_id
                )),
                (true, None) => Err(anyhow::anyhow!(
                    "BUG REPRODUCED [MQTT-4.4.0-1]: control (no Message Expiry Interval) got its \
                     owed PUBREL {control_pubrel:?} resent, but the arm (Message Expiry Interval \
                     {INFLIGHT_EXPIRY_SECS}s, cut {INFLIGHT_WAIT_SECS}s ago) got nothing although \
                     its session resumed. The exchange had already passed PUBREC (original packet \
                     id {}), so the PUBLISH had been sent and onward delivery had started: \
                     [MQTT-4.3.3-7] forbids the sender to apply Message Expiry from that point on, \
                     and [MQTT-4.4.0-1] requires the owed PUBREL to be re-sent with its original \
                     Packet Identifier",
                    arm.pubrel_packet_id
                )),
                (false, _) => Err(anyhow::anyhow!(
                    "arm: the session was not resumed (session_present=0), so the reproduction \
                     target was never reached"
                )),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(120)
    }
}

// ---------------------------------------------------------------------------
// 3. H-14 — an oversized queued message stalls the traffic behind it
// ---------------------------------------------------------------------------

/// Size of the queued message that exceeds the arm's Maximum Packet Size.
const OVERSIZED_LEN: usize = 40 * 1024;
const MARKER: &[u8] = b"marker";
/// Maximum Packet Size declared by the arm on resume.
const ARM_MAX_PACKET_SIZE: u32 = 1024;
/// Maximum Packet Size declared by the control on resume: large enough for
/// both queued messages.
const CONTROL_MAX_PACKET_SIZE: u32 = 1024 * 1024;

pub struct OversizedQueuedMessageStallsQueueV5Test;

impl TestCase for OversizedQueuedMessageStallsQueueV5Test {
    fn name(&self) -> &str {
        "oversized_queued_message_stalls_queue_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let topic = format!("issue513/maxpkt/{uid}");
            let arm_client_id = format!("issue513-maxpkt-arm-{uid}");
            let control_client_id = format!("issue513-maxpkt-ctl-{uid}");
            let addr = ctx.config.broker_addr.clone();
            let timeout = ctx.config.connect_timeout;

            // ---- two persistent sessions subscribed to the same topic
            for (label, client_id) in [("control", &control_client_id), ("arm", &arm_client_id)] {
                let mut sub = connect(&addr, client_id, timeout, false, Some(300), None).await?;
                let ack = sub.subscribe(&topic, QoS::AtLeastOnce).await?;
                if !granted(&ack.status) {
                    return Err(anyhow::anyhow!(
                        "{label}: SUBACK refused the filter ({})",
                        describe_status(&ack.status)
                    ));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
                let _ = sub.disconnect().await;
            }

            // ---- queue the oversized message, then a small marker behind it
            let oversized = vec![b'B'; OVERSIZED_LEN];
            let mut publisher =
                connect(&addr, &format!("issue513-maxpkt-pub-{uid}"), timeout, true, None, None).await?;
            let ack = publish_qos1(&mut publisher, &topic, &oversized, false, None).await?;
            if ack >= 0x80 {
                return Err(anyhow::anyhow!(
                    "the oversized publication was refused (PUBACK 0x{ack:02x}), so nothing was queued"
                ));
            }
            let ack = publish_qos1(&mut publisher, &topic, MARKER, false, None).await?;
            let _ = publisher.disconnect().await;
            if ack >= 0x80 {
                return Err(anyhow::anyhow!(
                    "the marker publication was refused (PUBACK 0x{ack:02x}), so nothing was queued"
                ));
            }

            // ---- CONTROL: resume with a large Maximum Packet Size
            let mut control =
                connect(&addr, &control_client_id, timeout, false, Some(300), Some(CONTROL_MAX_PACKET_SIZE))
                    .await?;
            let control_payloads = drain_payloads(&mut control, Duration::from_secs(8)).await;
            let control_marker = control_payloads.iter().any(|p| p.as_slice() == MARKER);
            let _ = control.disconnect().await;
            if !control_marker {
                return Err(anyhow::anyhow!(
                    "control failed: nothing survived the cut even without a packet size limit \
                     ({} message(s) received), so the queue never held the marker",
                    control_payloads.len()
                ));
            }

            // ---- ARM: resume declaring a Maximum Packet Size below the queue
            let mut arm =
                connect(&addr, &arm_client_id, timeout, false, Some(300), Some(ARM_MAX_PACKET_SIZE)).await?;
            let arm_payloads = drain_payloads(&mut arm, Duration::from_secs(8)).await;
            let arm_marker = arm_payloads.iter().any(|p| p.as_slice() == MARKER);
            // The Server must not send a packet that exceeds the Maximum Packet
            // Size the client declared (section 3.1.2.11.4), so an oversized
            // payload arriving here is a violation even if the marker follows.
            let arm_over_limit = arm_payloads.iter().find(|p| p.len() as u32 > ARM_MAX_PACKET_SIZE);
            let arm_usable = arm.is_connected();
            let _ = arm.disconnect().await;

            if let Some(p) = arm_over_limit {
                return Err(anyhow::anyhow!(
                    "BUG REPRODUCED [MQTT-3.1.2.11.4]: the server sent a {} byte payload on a \
                     connection that declared Maximum Packet Size {ARM_MAX_PACKET_SIZE}",
                    p.len()
                ));
            }
            if arm_marker && arm_usable {
                return Ok(());
            }

            Err(anyhow::anyhow!(
                "BUG REPRODUCED [MQTT-3.1.2.11.4]: queue was [{OVERSIZED_LEN} byte message, \
                 {} byte marker]. resume with Maximum Packet Size {CONTROL_MAX_PACKET_SIZE}: \
                 {} message(s), marker present. resume with Maximum Packet Size \
                 {ARM_MAX_PACKET_SIZE}: {} message(s), marker {}, connection {}. The oversized \
                 message may be discarded, but the queue must then continue on a usable \
                 connection — instead `EncodeError::OverMaxPacketSize` is propagated out of the \
                 session event loop and the connection is closed",
                MARKER.len(),
                control_payloads.len(),
                arm_payloads.len(),
                if arm_marker { "present" } else { "ABSENT" },
                if arm_usable { "usable" } else { "closed by broker" }
            ))
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(90)
    }
}

// ---------------------------------------------------------------------------
// 4. H-14 on the retained path — an oversized retained message must not kill
//    the session that subscribes to it
// ---------------------------------------------------------------------------

/// Payload size of the retained message: far above the arm's Maximum Packet
/// Size, far below the control's and below the broker's inbound limit.
const RETAINED_OVERSIZED_LEN: usize = 40 * 1024;
/// Maximum Packet Size declared by the arm on SUBSCRIBE.
const RETAINED_ARM_MAX_PACKET_SIZE: u32 = 1024;
/// Maximum Packet Size declared by the control: large enough for the payload.
const RETAINED_CONTROL_MAX_PACKET_SIZE: u32 = 1024 * 1024;
/// How long each subscriber waits for the retained delivery to show up.
const RETAINED_DRAIN: Duration = Duration::from_secs(5);

pub struct RetainedOversizedMessageKeepsSessionV5Test;

impl TestCase for RetainedOversizedMessageKeepsSessionV5Test {
    fn name(&self) -> &str {
        "retained_oversized_message_keeps_session_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        if let Some(r) = ctx.guard_retain_required(self.name(), "functional_v5", start) {
            return r;
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let topic = format!("issue513/retained-maxpkt/{uid}");
            let probe_topic = format!("issue513/retained-maxpkt/{uid}/probe");
            let addr = ctx.config.broker_addr.clone();
            let timeout = ctx.config.connect_timeout;

            // ---- store the oversized retained message
            let payload = vec![b'R'; RETAINED_OVERSIZED_LEN];
            let mut publisher =
                connect(&addr, &format!("issue513-retmax-pub-{uid}"), timeout, true, None, None).await?;
            let ack = publish_qos1(&mut publisher, &topic, &payload, true, None).await?;
            let _ = publisher.disconnect().await;
            if ack >= 0x80 {
                return Err(anyhow::anyhow!(
                    "the retained publication was refused (PUBACK 0x{ack:02x}), so nothing was stored"
                ));
            }
            // The retainer stores asynchronously; give it a moment before the
            // control arm reads back. Setup pause, not a measured deadline.
            tokio::time::sleep(Duration::from_millis(200)).await;

            // ---- CONTROL: a large Maximum Packet Size receives it intact
            let mut control = connect(
                &addr,
                &format!("issue513-retmax-ctl-{uid}"),
                timeout,
                true,
                None,
                Some(RETAINED_CONTROL_MAX_PACKET_SIZE),
            )
            .await?;
            let sub = control.subscribe(&topic, QoS::AtLeastOnce).await?;
            let control_status = describe_status(&sub.status);
            let control_payloads = drain_payloads(&mut control, RETAINED_DRAIN).await;
            let control_got_it = control_payloads.iter().any(|p| p.len() == RETAINED_OVERSIZED_LEN);
            let _ = control.disconnect().await;

            if !granted(&sub.status) {
                return Err(anyhow::anyhow!(
                    "control failed: SUBACK refused the filter ({control_status}), so the retained \
                     read-back was never attempted"
                ));
            }
            if !control_got_it {
                return Err(anyhow::anyhow!(
                    "control failed: with Maximum Packet Size {RETAINED_CONTROL_MAX_PACKET_SIZE} the \
                     retained message never arrived ({} payload(s), lengths {:?}), so it was either \
                     not stored or not deliverable — that is a different defect and the arm below \
                     cannot be judged on it",
                    control_payloads.len(),
                    control_payloads.iter().map(|p| p.len()).collect::<Vec<_>>()
                ));
            }

            // ---- ARM: a small Maximum Packet Size
            let mut arm = connect(
                &addr,
                &format!("issue513-retmax-arm-{uid}"),
                timeout,
                true,
                None,
                Some(RETAINED_ARM_MAX_PACKET_SIZE),
            )
            .await?;
            let arm_sub = arm.subscribe(&topic, QoS::AtLeastOnce).await;
            let arm_status = match &arm_sub {
                Ok(ack) => describe_status(&ack.status),
                Err(e) => format!("SUBACK error: {e}"),
            };
            let arm_payloads = drain_payloads(&mut arm, RETAINED_DRAIN).await;
            let arm_over_limit = arm_payloads.iter().find(|p| p.len() as u32 > RETAINED_ARM_MAX_PACKET_SIZE);
            let arm_connected = arm.is_connected();

            // Prove the session survived: a second SUBSCRIBE must still complete
            // a full round trip on this same connection.
            let arm_probe = arm.subscribe(&probe_topic, QoS::AtLeastOnce).await;
            let arm_probe_granted = arm_probe.as_ref().map(|ack| granted(&ack.status)).unwrap_or(false);
            let arm_probe_err = arm_probe.err().map(|e| e.to_string());
            let _ = arm.disconnect().await;

            if !arm_sub.is_ok() {
                return Err(anyhow::anyhow!(
                    "BUG REPRODUCED [MQTT-3.1.2-25]: the SUBACK for the filter carrying a \
                     {RETAINED_OVERSIZED_LEN} byte retained message never arrived ({arm_status}); \
                     the oversized message is discarded, but the subscribe itself must still \
                     succeed"
                ));
            }
            if let Some(p) = arm_over_limit {
                return Err(anyhow::anyhow!(
                    "BUG REPRODUCED [MQTT-3.1.2-24]: the server sent a {} byte payload on a \
                     connection that declared Maximum Packet Size {RETAINED_ARM_MAX_PACKET_SIZE}",
                    p.len()
                ));
            }

            if arm_probe_granted {
                return Ok(());
            }

            Err(anyhow::anyhow!(
                "BUG REPRODUCED [MQTT-3.1.2-25]: published a {RETAINED_OVERSIZED_LEN} byte retained \
                 message. control (Maximum Packet Size {RETAINED_CONTROL_MAX_PACKET_SIZE}): {} \
                 payload(s), received the retained message {}. arm (Maximum Packet Size \
                 {RETAINED_ARM_MAX_PACKET_SIZE}): SUBACK {}, {} payload(s), connection {}. A \
                 follow-up SUBSCRIBE on the arm did not complete ({}) — the session is gone. The \
                 oversized message may be discarded, but the Server must then behave as if it had \
                 completed sending that Application Message, keeping the connection usable; \
                 instead `EncodeError::OverMaxPacketSize` escapes `Session::deliver` and the \
                 session is torn down. A retained message is not consumed by a delivery, so such a \
                 client is disconnected on every SUBSCRIBE",
                control_payloads.len(),
                if control_got_it { "intact" } else { "NOT" },
                arm_status,
                arm_payloads.len(),
                if arm_connected { "usable" } else { "closed by broker" },
                arm_probe_err.as_deref().unwrap_or("no SUBACK")
            ))
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(90)
    }
}
