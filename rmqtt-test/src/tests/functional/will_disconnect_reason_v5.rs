//! MQTT 5.0: Will Message publication vs the DISCONNECT Reason Code.
//!
//! GitHub issue #514: the Will Message is not published after a DISCONNECT with
//! Reason Code `0x04` (Disconnect with Will Message).
//!
//! [MQTT-3.1.2-8/9] a stored Will Message "MUST be published after the Network
//! Connection is subsequently closed", and [MQTT-3.14.2.1] section 3.1.2.5 lets
//! the Server delete the Will in exactly one case: "unless the Will Message has
//! been deleted by the Server on receipt of a DISCONNECT packet with Reason Code
//! 0x00 (Normal disconnection)". `0x04` is not `0x00`.
//!
//! The broker distinguishes the Reason Code nowhere: `Session::last_will_enable`
//! suppresses the Will as soon as `StateFlags::DisconnectReceived` is set, which
//! the V5 DISCONNECT reader sets unconditionally (`rmqtt/src/session.rs`). The
//! reproducer below keeps a control arm — an abrupt close, which is known to
//! publish the Will — inside the same execution, on the same watcher and topic,
//! so a failure can only be attributed to the `0x04` path.

use std::time::{Duration, Instant};

use bytestring::ByteString;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v5::MqttV5Client;

/// How long to wait for a Will that is due immediately.
const WILL_WINDOW: Duration = Duration::from_secs(8);

/// How long to wait for the control probe, which only proves the watcher sees
/// ordinary traffic on the Will topic.
const PROBE_WINDOW: Duration = Duration::from_secs(5);

/// Build a Will message of the shape used by issue #514: QoS 1, no retain, and
/// a Will Delay Interval explicitly set to 0 so the Will is due immediately.
fn issue_will(topic: &str, payload: &'static [u8]) -> rmqtt_codec::v5::LastWill {
    rmqtt_codec::v5::LastWill {
        qos: QoS::AtLeastOnce,
        retain: false,
        topic: ByteString::from(topic),
        message: bytes::Bytes::from_static(payload),
        will_delay_interval_sec: Some(0),
        correlation_data: None,
        message_expiry_interval: None,
        content_type: None,
        user_properties: Vec::new(),
        is_utf8_payload: None,
        response_topic: None,
    }
}

/// Wait up to `window` for an incoming PUBLISH whose payload equals `expected`,
/// discarding anything else. Returns whether it arrived.
async fn wait_for_payload(sub: &mut MqttV5Client, expected: &[u8], window: Duration) -> bool {
    let deadline = Instant::now() + window;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match sub.recv_message_timeout(left).await {
            Some(m) if m.payload.as_ref() == expected => return true,
            Some(_) => continue,
            None => return false,
        }
    }
    false
}

/// Watch the Will topic for `window` and report whether a payload equal to
/// `forbidden` was ever delivered.
async fn seen_payload(sub: &mut MqttV5Client, forbidden: &[u8], window: Duration) -> bool {
    let deadline = Instant::now() + window;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match sub.recv_message_timeout(left).await {
            Some(m) if m.payload.as_ref() == forbidden => return true,
            Some(_) => continue,
            None => return false,
        }
    }
    false
}

/// Publish an ordinary QoS 1 message on the Will topic and require the watcher
/// to see it, so that a later "the Will was not published" result cannot be
/// explained by a blind watcher or a subscription whose routing has not been
/// established yet.
async fn probe_watcher(
    ctx: &TestContext,
    client_id: &str,
    topic: &str,
    watcher: &mut MqttV5Client,
) -> anyhow::Result<()> {
    let probe = b"probe";
    let publisher =
        MqttV5Client::connect(&ctx.config.broker_addr, client_id, ctx.config.connect_timeout).await?;
    publisher.publish(topic, probe, QoS::AtLeastOnce, false).await?;
    let seen = wait_for_payload(watcher, probe, PROBE_WINDOW).await;
    let _ = publisher.disconnect().await;
    if !seen {
        return Err(anyhow::anyhow!(
            "control failed: the watcher did not see an ordinary publish on {topic}, \
             so Will results cannot be interpreted"
        ));
    }
    Ok(())
}

/// Connect a client carrying a Will and subscribe a watcher to its Will topic.
async fn watcher_for(ctx: &TestContext, client_id: &str, topic: &str) -> anyhow::Result<MqttV5Client> {
    let mut watcher =
        MqttV5Client::connect(&ctx.config.broker_addr, client_id, ctx.config.connect_timeout).await?;
    watcher.subscribe(topic, QoS::AtLeastOnce).await?;
    // A SUBACK says the filter was accepted, not that routing already matches
    // it; pause so the first arm of the test cannot fail for an unrelated
    // reason. This is setup, never a measured deadline.
    tokio::time::sleep(Duration::from_millis(100)).await;
    Ok(watcher)
}

/// Connect a client whose only interesting property is the Will it carries.
async fn client_with_will(
    ctx: &TestContext,
    client_id: &str,
    will_topic: &str,
    will_payload: &'static [u8],
) -> anyhow::Result<MqttV5Client> {
    MqttV5Client::connect_with_options(
        &ctx.config.broker_addr,
        client_id,
        ctx.config.connect_timeout,
        true,
        60,
        Some(issue_will(will_topic, will_payload)),
        None,
        None,
        None,
        None,
        None,
    )
    .await
}

/// Reproducer for issue #514.
///
/// CONTROL  an ordinary publication on the Will topic, proving the watcher is
///          subscribed and capturing.
/// ARM A    connect with a Will, then close abruptly. The Will MUST be
///          published — this is the control arm that fails if the broker or the
///          watcher is broken.
/// ARM B    connect with the same Will, then send `DISCONNECT` with Reason Code
///          `0x04` (Disconnect with Will Message). The Will MUST be published
///          exactly as in ARM A. [MQTT-3.1.2-8/9, MQTT-3.14.2.1]
pub struct WillPublishedOnDisconnectRc0x04V5Test;

impl TestCase for WillPublishedOnDisconnectRc0x04V5Test {
    fn name(&self) -> &str {
        "will_published_on_disconnect_rc_0x04_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let will_topic = "test/v5/willrc/rc04";
            let mut watcher = watcher_for(ctx, "willrc-rc04-watcher", will_topic).await?;
            probe_watcher(ctx, "willrc-rc04-probe", will_topic, &mut watcher).await?;

            // ARM A: abrupt close, no DISCONNECT at all.
            let arm_a = client_with_will(ctx, "willrc-rc04-rst", will_topic, b"will-rst").await?;
            arm_a.abort_connection().await?;
            let rst_seen = wait_for_payload(&mut watcher, b"will-rst", WILL_WINDOW).await;

            // ARM B: the behaviour under test.
            let arm_b = client_with_will(ctx, "willrc-rc04-disc", will_topic, b"will-0x04").await?;
            arm_b.disconnect_with_reason(Some(0x04)).await?;
            let rc04_seen = wait_for_payload(&mut watcher, b"will-0x04", WILL_WINDOW).await;

            let _ = watcher.disconnect().await;

            if !rst_seen {
                return Err(anyhow::anyhow!(
                    "control failed: the Will was not published after an abrupt close either, so the \
                     broker or the watcher is broken and the 0x04 result would be meaningless"
                ));
            }
            if !rc04_seen {
                return Err(anyhow::anyhow!(
                    "Will NOT published: abrupt close published the Will, DISCONNECT with Reason Code \
                     0x04 (Disconnect with Will Message) did not. The Server may delete the Will only on \
                     DISCONNECT with Reason Code 0x00 [MQTT-3.1.2-8/9, MQTT-3.14.2.1]. GitHub issue #514"
                ));
            }
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        // ARM A + ARM B sequentially, each with its own WILL_WINDOW.
        Duration::from_secs(40)
    }
}

/// Negative guard: `DISCONNECT` with Reason Code `0x00` (Normal disconnection)
/// is the one and only case in which the Server deletes the Will Message, so no
/// Will may be published afterwards. [MQTT-3.1.2-8/9, MQTT-3.14.2.1]
///
/// Pairs with the `0x04` reproducer: it pins the opposite direction, so a fix
/// cannot simply publish the Will on every DISCONNECT.
pub struct WillNotPublishedOnDisconnectRc0x00V5Test;

impl TestCase for WillNotPublishedOnDisconnectRc0x00V5Test {
    fn name(&self) -> &str {
        "will_not_published_on_disconnect_rc_0x00_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let will_topic = "test/v5/willrc/rc00";
            let mut watcher = watcher_for(ctx, "willrc-rc00-watcher", will_topic).await?;
            probe_watcher(ctx, "willrc-rc00-probe", will_topic, &mut watcher).await?;

            let client = client_with_will(ctx, "willrc-rc00-disc", will_topic, b"will-rc00").await?;
            client.disconnect_with_reason(Some(0x00)).await?;

            let leaked = seen_payload(&mut watcher, b"will-rc00", Duration::from_secs(2)).await;
            let _ = watcher.disconnect().await;

            if leaked {
                return Err(anyhow::anyhow!(
                    "the Will was published after a DISCONNECT with Reason Code 0x00, the only case in \
                     which the Server must delete it [MQTT-3.1.2-8/9, MQTT-3.14.2.1]"
                ));
            }
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// DISCONNECT Reason Codes other than 0x00 that a Client is allowed to send
/// (section 3.14.2.1 lists them as "Client or Server"), each with a distinct
/// Will payload so a delivery can never be attributed to the wrong iteration.
const NON_NORMAL_REASON_CODES: &[(u8, &str, &[u8])] = &[
    (0x80, "0x80", b"will-0x80"), // Unspecified error
    (0x82, "0x82", b"will-0x82"), // Protocol Error
    (0x93, "0x93", b"will-0x93"), // Receive Maximum exceeded
];

/// The Will Message must be published after a DISCONNECT carrying any Reason
/// Code other than 0x00, not only after `0x04`. [MQTT-3.1.2-8/9, MQTT-3.14.2.1]
///
/// Reason Code 0x00 is the single code that deletes the Will, so a fix that
/// special-cases `0x04` alone would turn the #514 reproducer green and still
/// violate the spec; this case pins the general rule.
pub struct WillPublishedOnDisconnectRcNot0x00V5Test;

impl TestCase for WillPublishedOnDisconnectRcNot0x00V5Test {
    fn name(&self) -> &str {
        "will_published_on_disconnect_rc_not_0x00_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            // One watcher on a wildcard, so every iteration shares the setup
            // while each Will keeps its own topic and payload.
            let base = "test/v5/willrc/other";
            let mut watcher = watcher_for(ctx, "willrc-other-watcher", &format!("{base}/#")).await?;
            probe_watcher(ctx, "willrc-other-probe", &format!("{base}/probe"), &mut watcher).await?;

            let mut missing: Vec<&str> = Vec::new();
            for (code, label, payload) in NON_NORMAL_REASON_CODES {
                let will_topic = format!("{base}/{label}");
                let client =
                    client_with_will(ctx, &format!("willrc-other-{label}"), &will_topic, payload).await?;
                client.disconnect_with_reason(Some(*code)).await?;
                if !wait_for_payload(&mut watcher, payload, WILL_WINDOW).await {
                    missing.push(label);
                }
            }

            let _ = watcher.disconnect().await;

            if !missing.is_empty() {
                return Err(anyhow::anyhow!(
                    "the Will was NOT published after DISCONNECT with Reason Code {}. Only Reason \
                     Code 0x00 may delete the Will; every other code leaves it due for publication \
                     [MQTT-3.1.2-8/9, MQTT-3.14.2.1]",
                    missing.join(", ")
                ));
            }
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        // One WILL_WINDOW per Reason Code, plus setup.
        Duration::from_secs(45)
    }
}

/// Baseline: a Will is published after an abrupt close, with no DISCONNECT of
/// any kind involved. Asserted standalone so the `0x04` reproducer's control arm
/// is independently covered. [MQTT-3.1.2-8/9]
pub struct WillPublishedOnAbruptCloseV5Test;

impl TestCase for WillPublishedOnAbruptCloseV5Test {
    fn name(&self) -> &str {
        "will_published_on_abrupt_close_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let will_topic = "test/v5/willrc/abrupt";
            let mut watcher = watcher_for(ctx, "willrc-abrupt-watcher", will_topic).await?;
            probe_watcher(ctx, "willrc-abrupt-probe", will_topic, &mut watcher).await?;

            let client = client_with_will(ctx, "willrc-abrupt", will_topic, b"will-abrupt").await?;
            client.abort_connection().await?;
            let seen = wait_for_payload(&mut watcher, b"will-abrupt", WILL_WINDOW).await;

            let _ = watcher.disconnect().await;

            if !seen {
                return Err(anyhow::anyhow!(
                    "the Will was not published after an abrupt close [MQTT-3.1.2-8/9]"
                ));
            }
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}
