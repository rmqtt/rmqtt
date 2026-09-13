//! rmqtt-delayed plugin functional tests (`$delayed/<interval>/<topic>`)
//!
//! Grouped in the standalone `delayed` suite so the plugin can be tested in
//! isolation: `--suites delayed`. Every harness-broker test declares a
//! `broker_config()`, so the suite automatically splits into these
//! sub-suites (one broker instance each, config switched at suite
//! boundaries by the scheduler):
//!
//! - `delayed@delayed`: plugin defaults (`publish_max = 100_000`, `publish_immediate = true`), plus retainer and http-api.
//! - `delayed@delayed-max1`: `publish_max = 1`, overflow forwarded immediately (`publish_immediate = true`).
//! - `delayed@delayed-max1-drop`: `publish_max = 1`, overflow dropped (`publish_immediate = false`).
//! - `delayed@delayed-flush`: plugin defaults, dedicated broker for the plugin-unload/flush test (unloading the plugin inside the test must not disturb the other delayed tests).
//! - `delayed` (plain): the cluster test only; it spawns its own 2-node broadcast cluster and never touches the harness broker.
//!
//! # How to run
//!
//! ```bash
//! cargo build -p rmqttd && cargo build -p rmqtt-test
//! ./target/debug/mqtt_harness --binary target/debug/rmqttd \
//!   --workspace . --suites delayed --workers 4
//! ```
//!
//! # Behaviour pinned here (implementation refs)
//!
//! - `session.rs::_publish` calls `delayed_sender.parse()` when the plugin
//!   is loaded: `$delayed/<interval>/<topic>` sets `delay_interval` and
//!   rewrites the topic; a non-integer interval returns an error.
//! - `session.rs::forwards` queues the message via `delay_publish`;
//!   `MemDelayedSender` (per-node `BinaryHeap`) forwards expired messages
//!   on a 500 ms tick through `SessionState::forwards` (retain storage and
//!   cross-node routing happen at expiry time, not at publish time).
//! - On plugin stop the pending queue is flushed according to
//!   `publish_immediate` (forward immediately / drop with
//!   `Reason::DelayedPublishRefused`).
//!
//! The 500 ms dispatch tick adds jitter to every timing assertion; the
//! windows below leave ample headroom.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::anyhow;

use crate::framework::context::{TestConfig, TestContext};
use crate::framework::testcase::{Expectation, TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v5::client::{IncomingMessage, MqttV5Client};

/// 500 ms background dispatch tick of `MemDelayedSender` (timing jitter).
const TICK: Duration = Duration::from_millis(500);
/// Time to wait before concluding "no message was delivered".
const SILENT_WINDOW: Duration = Duration::from_millis(1200);

/// Broker config for the default delayed suite (plugin defaults).
fn delayed_config() -> Option<PathBuf> {
    Some(crate::tests::config_path("delayed"))
}

/// Broker config with `publish_max = 1`, `publish_immediate = true`.
fn delayed_max1_config() -> Option<PathBuf> {
    Some(crate::tests::config_path("delayed-max1"))
}

/// Broker config with `publish_max = 1`, `publish_immediate = false`.
fn delayed_max1_drop_config() -> Option<PathBuf> {
    Some(crate::tests::config_path("delayed-max1-drop"))
}

/// Broker config for the plugin-unload/flush test (isolated broker).
fn delayed_flush_config() -> Option<PathBuf> {
    Some(crate::tests::config_path("delayed-flush"))
}

// ---------------------------------------------------------------------------
// Shared helpers (v5 client)
// ---------------------------------------------------------------------------

/// Connect a v5 client and subscribe, then drain anything stale so later
/// assertions start from a clean slate (tests may run concurrently, so all
/// payload assertions are filtered by unique payloads anyway).
async fn connect_and_subscribe(
    cfg: &TestConfig,
    client_id: &str,
    filter: &str,
    qos: QoS,
) -> anyhow::Result<MqttV5Client> {
    let mut c = MqttV5Client::connect(&cfg.broker_addr, client_id, cfg.connect_timeout).await?;
    c.subscribe(filter, qos).await?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    while c.recv_message_timeout(Duration::from_millis(100)).await.is_some() {}
    Ok(c)
}

/// Wait for a message with exactly `payload`; skips unrelated messages from
/// concurrently running tests. Returns `(topic, message, elapsed)`.
async fn wait_for_payload(
    client: &mut MqttV5Client,
    payload: &[u8],
    timeout: Duration,
) -> Option<(bytestring::ByteString, IncomingMessage, Duration)> {
    let start = Instant::now();
    while start.elapsed() < timeout {
        match client.recv_message_timeout(Duration::from_millis(200)).await {
            Some(m) if m.payload.as_ref() == payload => {
                return Some((m.topic.clone(), m, start.elapsed()));
            }
            Some(_) => continue, // unrelated message from another test
            None => continue,
        }
    }
    None
}

/// Assert that no message with `payload` arrives within `window`.
async fn assert_silent(client: &mut MqttV5Client, payload: &[u8], window: Duration) -> anyhow::Result<()> {
    let start = Instant::now();
    while start.elapsed() < window {
        if let Some(m) = client.recv_message_timeout(Duration::from_millis(200)).await {
            if m.payload.as_ref() == payload {
                return Err(anyhow!(
                    "message arrived unexpectedly after {:?} (should have been suppressed)",
                    start.elapsed()
                ));
            }
        }
    }
    Ok(())
}

/// Minimal HTTP/1.1 PUT (no external HTTP client dependency). Returns the
/// response status code.
async fn http_put_status(addr: &str, path: &str) -> anyhow::Result<u16> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(addr).await?;
    let req =
        format!("PUT {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    let head = String::from_utf8_lossy(&buf);
    head.split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or_else(|| anyhow!("malformed HTTP response: {head:.80}"))
}

/// Standard test wrapper: run an async body on a fresh runtime, map the
/// result onto a `TestResult` of the `delayed` suite. The future may borrow
/// the `TestContext` (`block_on` does not require `'static`), matching the
/// inline `rt.block_on(async { .. })` pattern of the other functional tests.
fn run<'a>(name: &str, fut: impl std::future::Future<Output = anyhow::Result<()>> + 'a) -> TestResult {
    let start = Instant::now();
    let rt = tokio::runtime::Runtime::new().unwrap();
    match rt.block_on(fut) {
        Ok(()) => TestResult::passed(name, "delayed", start.elapsed()),
        Err(e) => TestResult::failed(name, "delayed", start.elapsed(), e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// P1 — basic delivery, routing, QoS, ordering
// ---------------------------------------------------------------------------

/// P1: a `$delayed/2/<topic>` publish is delivered ~2s later on the
/// stripped topic, not during the pending window (v5).
pub struct DelayedBasicDeliveryV5Test;

impl TestCase for DelayedBasicDeliveryV5Test {
    fn name(&self) -> &str {
        "delayed_basic_delivery_v5"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        delayed_config()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(self.name(), async {
            let mut sub =
                connect_and_subscribe(&ctx.config, "dly-basic-v5-sub", "dly/basic/t", QoS::AtLeastOnce)
                    .await?;
            let pubc = MqttV5Client::connect(
                &ctx.config.broker_addr,
                "dly-basic-v5-pub",
                ctx.config.connect_timeout,
            )
            .await?;

            let t0 = Instant::now();
            pubc.publish("$delayed/2/dly/basic/t", b"basic-payload", QoS::AtMostOnce, false).await?;

            // Nothing must be delivered while the message is pending.
            assert_silent(&mut sub, b"basic-payload", Duration::from_millis(1200)).await?;

            let (topic, msg, _wait) = wait_for_payload(&mut sub, b"basic-payload", Duration::from_secs(6))
                .await
                .ok_or_else(|| anyhow!("delayed message not delivered within 6s"))?;
            let total = t0.elapsed();

            if &*topic != "dly/basic/t" {
                return Err(anyhow!("delivered on wrong topic: {topic} (expected dly/basic/t)"));
            }
            if total < Duration::from_millis(1500) {
                return Err(anyhow!("delivered too early ({total:?}); the ~2s delay was not honoured"));
            }
            if msg.retain {
                return Err(anyhow!("delayed delivery must not be flagged as retained"));
            }

            pubc.disconnect().await?;
            sub.disconnect().await?;
            Ok(())
        })
    }
}

/// P1: same as `delayed_basic_delivery_v5` but over MQTT 3.1.1 (the delayed
/// parse/queue path is protocol-agnostic in the broker).
pub struct DelayedBasicDeliveryV311Test;

impl TestCase for DelayedBasicDeliveryV311Test {
    fn name(&self) -> &str {
        "delayed_basic_delivery_v311"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        delayed_config()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(self.name(), async {
            let mut sub = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "dly-basic-v311-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            sub.subscribe("dly/basic311/t", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(200)).await;
            while sub.recv_message_timeout(Duration::from_millis(100)).await.is_some() {}

            let pubc = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "dly-basic-v311-pub",
                ctx.config.connect_timeout,
            )
            .await?;

            let t0 = Instant::now();
            pubc.publish("$delayed/2/dly/basic311/t", b"basic311-payload", QoS::AtMostOnce, false).await?;

            // Nothing while pending.
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(1200) {
                if let Some(m) = sub.recv_message_timeout(Duration::from_millis(200)).await {
                    if m.payload.as_ref() == b"basic311-payload" {
                        return Err(anyhow!("delivered during the pending window"));
                    }
                }
            }

            // Arrives after expiry, on the stripped topic.
            let deadline = Instant::now() + Duration::from_secs(6);
            let got = loop {
                if Instant::now() >= deadline {
                    return Err(anyhow!("delayed message not delivered within 6s"));
                }
                if let Some(m) = sub.recv_message_timeout(Duration::from_millis(200)).await {
                    if m.payload.as_ref() == b"basic311-payload" {
                        break m;
                    }
                }
            };
            let total = t0.elapsed();
            if &*got.topic != "dly/basic311/t" {
                return Err(anyhow!("delivered on wrong topic: {}", got.topic));
            }
            if total < Duration::from_millis(1500) {
                return Err(anyhow!("delivered too early ({total:?})"));
            }

            pubc.disconnect().await?;
            sub.disconnect().await?;
            Ok(())
        })
    }
}

/// P1: routing semantics — the message leaves through the stripped topic
/// only; `#` does not see it while pending and `$delayed/#` subscribers
/// never see it at all.
pub struct DelayedRoutingNoDollarLeakTest;

impl TestCase for DelayedRoutingNoDollarLeakTest {
    fn name(&self) -> &str {
        "delayed_routing_no_dollar_leak"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        delayed_config()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(self.name(), async {
            let mut wild =
                connect_and_subscribe(&ctx.config, "dly-route-wild", "#", QoS::AtLeastOnce).await?;
            let mut dollar =
                connect_and_subscribe(&ctx.config, "dly-route-dollar", "$delayed/#", QoS::AtLeastOnce)
                    .await?;
            let pubc =
                MqttV5Client::connect(&ctx.config.broker_addr, "dly-route-pub", ctx.config.connect_timeout)
                    .await?;

            pubc.publish("$delayed/2/dly/route/x", b"route-payload", QoS::AtMostOnce, false).await?;

            // While pending: neither `#` nor `$delayed/#` sees anything.
            assert_silent(&mut wild, b"route-payload", SILENT_WINDOW).await?;

            // After expiry: `#` receives it on the stripped topic.
            let (topic, _, _) = wait_for_payload(&mut wild, b"route-payload", Duration::from_secs(6))
                .await
                .ok_or_else(|| anyhow!("# subscriber did not receive the delayed message"))?;
            if &*topic != "dly/route/x" {
                return Err(anyhow!("delivered on wrong topic: {topic}"));
            }

            // `$delayed/#` must never receive the (pending or forwarded)
            // message: the published topic was rewritten at parse time.
            assert_silent(&mut dollar, b"route-payload", Duration::from_secs(2)).await?;

            pubc.disconnect().await?;
            wild.disconnect().await?;
            dollar.disconnect().await?;
            Ok(())
        })
    }
}

/// P1: QoS is preserved end-to-end through the delay queue: QoS 1 in
/// (PUBACKed at schedule time) → QoS 1 out; QoS 2 in → QoS 2 out.
pub struct DelayedQosPreserveTest;

impl TestCase for DelayedQosPreserveTest {
    fn name(&self) -> &str {
        "delayed_qos_preserve"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        delayed_config()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(40)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(self.name(), async {
            let mut sub =
                connect_and_subscribe(&ctx.config, "dly-qos-sub", "dly/qos/#", QoS::ExactlyOnce).await?;
            let mut pubc =
                MqttV5Client::connect(&ctx.config.broker_addr, "dly-qos-pub", ctx.config.connect_timeout)
                    .await?;

            // QoS 1: the schedule-time PUBACK must be a success.
            pubc.publish("$delayed/2/dly/qos/q1", b"q1-payload", QoS::AtLeastOnce, false).await?;
            let ack = pubc
                .recv_puback_reason(Duration::from_secs(5))
                .await
                .ok_or_else(|| anyhow!("no PUBACK for the QoS 1 delayed publish"))?;
            if ack.1 != 0 {
                return Err(anyhow!("QoS 1 delayed publish PUBACK reason {} != 0", ack.1));
            }

            // QoS 2: the schedule-time PUBREC must be a success.
            pubc.publish("$delayed/2/dly/qos/q2", b"q2-payload", QoS::ExactlyOnce, false).await?;
            let rec = pubc
                .recv_pubrec_reason(Duration::from_secs(5))
                .await
                .ok_or_else(|| anyhow!("no PUBREC for the QoS 2 delayed publish"))?;
            if rec.1 != 0 {
                return Err(anyhow!("QoS 2 delayed publish PUBREC reason {} != 0", rec.1));
            }

            // Deliveries keep their QoS. Arrival order between the two is
            // unspecified (same trigger window) — match by payload.
            let mut got_q1 = false;
            let mut got_q2 = false;
            let deadline = Instant::now() + Duration::from_secs(8);
            while (!got_q1 || !got_q2) && Instant::now() < deadline {
                let Some(m) = sub.recv_message_timeout(Duration::from_millis(300)).await else {
                    continue;
                };
                match m.payload.as_ref() {
                    b"q1-payload" => {
                        if m.qos != QoS::AtLeastOnce {
                            return Err(anyhow!("QoS 1 delayed message delivered as {:?}", m.qos));
                        }
                        got_q1 = true;
                    }
                    b"q2-payload" => {
                        if m.qos != QoS::ExactlyOnce {
                            return Err(anyhow!("QoS 2 delayed message delivered as {:?}", m.qos));
                        }
                        got_q2 = true;
                    }
                    _ => {}
                }
            }
            if !got_q1 || !got_q2 {
                return Err(anyhow!("missing delivery (q1={got_q1}, q2={got_q2}) within 8s"));
            }

            pubc.disconnect().await?;
            sub.disconnect().await?;
            Ok(())
        })
    }
}

/// P1: multiple delayed messages with different intervals are delivered in
/// trigger-time order (3s queued first, then 1s, then 2s → 1, 2, 3).
pub struct DelayedOrderingByExpiryTest;

impl TestCase for DelayedOrderingByExpiryTest {
    fn name(&self) -> &str {
        "delayed_ordering_by_expiry"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        delayed_config()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(40)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(self.name(), async {
            let mut sub =
                connect_and_subscribe(&ctx.config, "dly-order-sub", "dly/order/o", QoS::AtLeastOnce).await?;
            let pubc =
                MqttV5Client::connect(&ctx.config.broker_addr, "dly-order-pub", ctx.config.connect_timeout)
                    .await?;

            pubc.publish("$delayed/3/dly/order/o", b"o3", QoS::AtMostOnce, false).await?;
            pubc.publish("$delayed/1/dly/order/o", b"o1", QoS::AtMostOnce, false).await?;
            pubc.publish("$delayed/2/dly/order/o", b"o2", QoS::AtMostOnce, false).await?;

            // Collect the three payloads in arrival order.
            let mut order: Vec<&[u8]> = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(9);
            while order.len() < 3 && Instant::now() < deadline {
                if let Some(m) = sub.recv_message_timeout(Duration::from_millis(300)).await {
                    if matches!(m.payload.as_ref(), b"o1" | b"o2" | b"o3") {
                        order.push(Box::leak(m.payload.to_vec().into_boxed_slice()));
                    }
                }
            }
            if order != [b"o1".as_slice(), b"o2".as_slice(), b"o3".as_slice()] {
                return Err(anyhow!(
                    "wrong delivery order: {:?} (expected [o1, o2, o3] = trigger-time order)",
                    order.iter().map(|p| String::from_utf8_lossy(p).to_string()).collect::<Vec<_>>()
                ));
            }

            pubc.disconnect().await?;
            sub.disconnect().await?;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// P2 — subscription timing, properties, error paths, semantics pinning
// ---------------------------------------------------------------------------

/// P2: unsubscribing before the trigger time suppresses the delivery.
pub struct DelayedUnsubscribeBeforeExpiryTest;

impl TestCase for DelayedUnsubscribeBeforeExpiryTest {
    fn name(&self) -> &str {
        "delayed_unsubscribe_before_expiry"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        delayed_config()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(self.name(), async {
            let mut sub =
                connect_and_subscribe(&ctx.config, "dly-unsub-sub", "dly/unsub/u", QoS::AtLeastOnce).await?;
            let pubc =
                MqttV5Client::connect(&ctx.config.broker_addr, "dly-unsub-pub", ctx.config.connect_timeout)
                    .await?;

            pubc.publish("$delayed/3/dly/unsub/u", b"unsub-payload", QoS::AtMostOnce, false).await?;
            tokio::time::sleep(Duration::from_millis(500)).await;
            sub.unsubscribe("dly/unsub/u").await?;

            assert_silent(&mut sub, b"unsub-payload", Duration::from_secs(6)).await?;

            pubc.disconnect().await?;
            sub.disconnect().await?;
            Ok(())
        })
    }
}

/// P2: subscribing AFTER the delayed publish (but before expiry) still
/// receives the message at trigger time.
pub struct DelayedSubscribeAfterPublishTest;

impl TestCase for DelayedSubscribeAfterPublishTest {
    fn name(&self) -> &str {
        "delayed_subscribe_after_publish"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        delayed_config()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(self.name(), async {
            let pubc =
                MqttV5Client::connect(&ctx.config.broker_addr, "dly-late-pub", ctx.config.connect_timeout)
                    .await?;

            let t0 = Instant::now();
            pubc.publish("$delayed/3/dly/late/s", b"late-payload", QoS::AtMostOnce, false).await?;
            tokio::time::sleep(Duration::from_millis(700)).await;

            let mut sub =
                connect_and_subscribe(&ctx.config, "dly-late-sub", "dly/late/s", QoS::AtLeastOnce).await?;
            wait_for_payload(&mut sub, b"late-payload", Duration::from_secs(6))
                .await
                .ok_or_else(|| anyhow!("late subscriber did not receive the delayed message"))?;

            let total = t0.elapsed();
            if total < Duration::from_millis(2500) {
                return Err(anyhow!("delivered at subscribe time ({total:?}) instead of trigger time (~3s)"));
            }

            pubc.disconnect().await?;
            sub.disconnect().await?;
            Ok(())
        })
    }
}

/// P2: v5 Message Expiry Interval survives the delay and is decremented by
/// the pending time (60s set at publish, ~2s pending → remaining < 60).
pub struct DelayedExpiryIntervalPassthroughV5Test;

impl TestCase for DelayedExpiryIntervalPassthroughV5Test {
    fn name(&self) -> &str {
        "delayed_expiry_interval_passthrough_v5"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        delayed_config()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(self.name(), async {
            let mut sub =
                connect_and_subscribe(&ctx.config, "dly-exp-sub", "dly/exp/e", QoS::AtLeastOnce).await?;
            let pubc =
                MqttV5Client::connect(&ctx.config.broker_addr, "dly-exp-pub", ctx.config.connect_timeout)
                    .await?;

            pubc.publish_with_properties(
                "$delayed/2/dly/exp/e",
                b"exp-payload",
                QoS::AtMostOnce,
                false,
                None,
                Some(60),
                None,
                None,
                None,
                None,
            )
            .await?;

            let (_, msg, _) = wait_for_payload(&mut sub, b"exp-payload", Duration::from_secs(6))
                .await
                .ok_or_else(|| anyhow!("delayed message not delivered within 6s"))?;
            match msg.message_expiry_interval {
                Some(v) => {
                    let remaining = v.get();
                    if remaining > 60 {
                        return Err(anyhow!("message expiry interval grew: {remaining} > 60"));
                    }
                    if remaining < 30 {
                        return Err(anyhow!(
                            "message expiry interval shrank too much: {remaining} (expected ~58)"
                        ));
                    }
                }
                None => {
                    return Err(anyhow!("message expiry interval was dropped by the delay path"));
                }
            }

            pubc.disconnect().await?;
            sub.disconnect().await?;
            Ok(())
        })
    }
}

/// P2: v5 publish properties (user properties, content type, response
/// topic, correlation data, payload format indicator) survive the delay
/// queue unchanged.
pub struct DelayedPropertiesPassthroughV5Test;

impl TestCase for DelayedPropertiesPassthroughV5Test {
    fn name(&self) -> &str {
        "delayed_properties_passthrough_v5"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        delayed_config()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(self.name(), async {
            let mut sub =
                connect_and_subscribe(&ctx.config, "dly-props-sub", "dly/props/p", QoS::AtLeastOnce).await?;
            let pubc =
                MqttV5Client::connect(&ctx.config.broker_addr, "dly-props-pub", ctx.config.connect_timeout)
                    .await?;

            pubc.publish_with_properties(
                "$delayed/2/dly/props/p",
                b"props-payload",
                QoS::AtMostOnce,
                false,
                Some(true),                                    // payload format indicator
                None,                                          // message expiry (covered elsewhere)
                Some("dly/props/resp"),                        // response topic
                Some(b"cor-1".as_slice()),                     // correlation data
                Some("text/plain"),                            // content type
                Some(&[("pk".to_string(), "pv".to_string())]), // user properties
            )
            .await?;

            let (_, msg, _) = wait_for_payload(&mut sub, b"props-payload", Duration::from_secs(6))
                .await
                .ok_or_else(|| anyhow!("delayed message not delivered within 6s"))?;

            if msg.content_type.as_deref() != Some("text/plain") {
                return Err(anyhow!("content_type not preserved: {:?}", msg.content_type));
            }
            if msg.response_topic.as_deref() != Some("dly/props/resp") {
                return Err(anyhow!("response_topic not preserved: {:?}", msg.response_topic));
            }
            if msg.correlation_data.as_deref() != Some(b"cor-1".as_slice()) {
                return Err(anyhow!("correlation_data not preserved: {:?}", msg.correlation_data));
            }
            if !msg.user_properties.iter().any(|(k, v)| &**k == "pk" && &**v == "pv") {
                return Err(anyhow!("user properties not preserved: {:?}", msg.user_properties));
            }

            pubc.disconnect().await?;
            sub.disconnect().await?;
            Ok(())
        })
    }
}

/// P2: a non-integer delay interval (`$delayed/abc/...`) is rejected by
/// `parse()` with an error; the broker must terminate the connection
/// (DISCONNECT or close) and must not route the message.
pub struct DelayedInvalidIntervalTest;

impl TestCase for DelayedInvalidIntervalTest {
    fn name(&self) -> &str {
        "delayed_invalid_interval"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        delayed_config()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(self.name(), async {
            let mut sub =
                connect_and_subscribe(&ctx.config, "dly-inv-sub", "dly/inv/x", QoS::AtLeastOnce).await?;
            let pubc =
                MqttV5Client::connect(&ctx.config.broker_addr, "dly-inv-pub", ctx.config.connect_timeout)
                    .await?;

            pubc.publish("$delayed/abc/dly/inv/x", b"inv-payload", QoS::AtMostOnce, false).await?;

            // The connection must be terminated (DISCONNECT or close) —
            // the parse error propagates out of the publish path.
            let deadline = Instant::now() + Duration::from_secs(3);
            while pubc.is_connected() && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if pubc.is_connected() {
                return Err(anyhow!(
                    "broker kept the connection open after a publish with a non-integer \
                     $delayed interval (expected DISCONNECT/close)"
                ));
            }

            // The message must not be routed.
            assert_silent(&mut sub, b"inv-payload", Duration::from_secs(4)).await?;

            sub.disconnect().await?;
            Ok(())
        })
    }
}

/// P2: record-type probe of unspecified `$delayed` topic shapes. No
/// assertion (Expectation::Info) — the actual broker behaviour is recorded
/// as observations so semantics can be pinned later:
///
/// - `$delayed/5` (missing topic segment): the two-segment form does NOT
///   match `splitn(3)`'s third element, so the publish should pass through
///   unchanged as a literal `$delayed/5` topic publish;
/// - `$delayed/5/` (empty topic): parse rewrites the topic to "" — the
///   broker's reaction to an empty-topic publish is recorded;
/// - `$delayed/-1/...` (negative interval): `u32` parse fails → same
///   rejection path as the non-integer interval.
pub struct DelayedMalformedTopicShapesTest;

impl TestCase for DelayedMalformedTopicShapesTest {
    fn name(&self) -> &str {
        "delayed_malformed_topic_shapes"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        delayed_config()
    }

    fn expectation(&self) -> Expectation {
        Expectation::Info
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(40)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let observation: anyhow::Result<String> = rt.block_on(async {
            let mut notes: Vec<String> = Vec::new();

            // Probe A: `$delayed/5` — literal passthrough expected.
            {
                let mut sub =
                    connect_and_subscribe(&ctx.config, "dly-shape-a-sub", "$delayed/5", QoS::AtLeastOnce)
                        .await?;
                let pubc = MqttV5Client::connect(
                    &ctx.config.broker_addr,
                    "dly-shape-a-pub",
                    ctx.config.connect_timeout,
                )
                .await?;
                pubc.publish("$delayed/5", b"shape-plain", QoS::AtMostOnce, false).await?;
                let delivered =
                    wait_for_payload(&mut sub, b"shape-plain", Duration::from_secs(3)).await.is_some();
                notes.push(format!(
                    "$delayed/5 (no topic segment): {}",
                    if delivered { "routed as a literal topic publish" } else { "NOT routed" }
                ));
                let _ = pubc.disconnect().await;
                let _ = sub.disconnect().await;
            }

            // Probe B: `$delayed/5/` — empty topic after the rewrite.
            {
                let pubc = MqttV5Client::connect(
                    &ctx.config.broker_addr,
                    "dly-shape-b-pub",
                    ctx.config.connect_timeout,
                )
                .await?;
                pubc.publish("$delayed/5/", b"shape-empty", QoS::AtMostOnce, false).await?;
                let deadline = Instant::now() + Duration::from_secs(2);
                while pubc.is_connected() && Instant::now() < deadline {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                notes.push(format!(
                    "$delayed/5/ (empty topic): {}",
                    if pubc.is_connected() { "connection kept open" } else { "connection closed" }
                ));
                let _ = pubc.disconnect().await;
            }

            // Probe C: `$delayed/-1/...` — negative interval.
            {
                let pubc = MqttV5Client::connect(
                    &ctx.config.broker_addr,
                    "dly-shape-c-pub",
                    ctx.config.connect_timeout,
                )
                .await?;
                pubc.publish("$delayed/-1/dly/shape/x", b"shape-neg", QoS::AtMostOnce, false).await?;
                let deadline = Instant::now() + Duration::from_secs(2);
                while pubc.is_connected() && Instant::now() < deadline {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                notes.push(format!(
                    "$delayed/-1/... (negative interval): {}",
                    if pubc.is_connected() { "connection kept open" } else { "connection closed" }
                ));
                let _ = pubc.disconnect().await;
            }

            Ok(notes.join("; "))
        });

        match observation {
            Ok(note) => TestResult::passed_with_note(self.name(), "delayed", start.elapsed(), &note),
            Err(e) => TestResult::failed(self.name(), "delayed", start.elapsed(), e.to_string()),
        }
    }
}

/// P2: `$delayed` prefixes are stripped exactly once, without re-parsing
/// the remainder: `$delayed/5/$delayed/2/a` is queued for 5s and delivered
/// on the literal topic `$delayed/2/a` (never on `a`).
pub struct DelayedNestedDollarPrefixTest;

impl TestCase for DelayedNestedDollarPrefixTest {
    fn name(&self) -> &str {
        "delayed_nested_dollar_prefix"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        delayed_config()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(40)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(self.name(), async {
            let mut sub_literal =
                connect_and_subscribe(&ctx.config, "dly-nest-lit", "$delayed/2/dly/nest/a", QoS::AtLeastOnce)
                    .await?;
            let mut sub_plain =
                connect_and_subscribe(&ctx.config, "dly-nest-plain", "dly/nest2/z", QoS::AtLeastOnce).await?;
            let pubc =
                MqttV5Client::connect(&ctx.config.broker_addr, "dly-nest-pub", ctx.config.connect_timeout)
                    .await?;

            pubc.publish("$delayed/5/$delayed/2/dly/nest/a", b"nest-a", QoS::AtMostOnce, false).await?;
            pubc.publish("$delayed/5/$delayed/2/dly/nest2/z", b"nest-z", QoS::AtMostOnce, false).await?;

            // The literal-topic subscriber must receive the message at the
            // OUTER trigger time (5s), on the one-level-stripped topic.
            let (topic, _, _) =
                wait_for_payload(&mut sub_literal, b"nest-a", Duration::from_secs(8)).await.ok_or_else(
                    || anyhow!("message not delivered on the literal topic $delayed/2/dly/nest/a"),
                )?;
            if &*topic != "$delayed/2/dly/nest/a" {
                return Err(anyhow!("delivered on wrong topic: {topic}"));
            }

            // The inner `$delayed/2/` must NOT be interpreted: no delivery
            // on the plain topic.
            assert_silent(&mut sub_plain, b"nest-z", Duration::from_secs(3)).await?;

            pubc.disconnect().await?;
            sub_literal.disconnect().await?;
            sub_plain.disconnect().await?;
            Ok(())
        })
    }
}

/// P2: retain semantics of delayed publishes. The retained store is written
/// only when the message is forwarded at trigger time (via `inner_forwards`),
/// so:
/// 1. during the pending window a fresh subscriber receives NO retained copy
///    — the first `ret-payload` message it sees must be the normal live
///    delivery at trigger time (retain flag cleared, since Retain As
///    Published is not set on the subscription);
/// 2. after expiry a fresh subscriber receives the message as retained.
///
/// Both subscribers connect WITHOUT the settle/drain of
/// `connect_and_subscribe`: a retained copy arrives right after SUBACK and
/// would be swallowed by the drain window. Any retained message left behind
/// by a previous (failed) run is cleared up front with an empty-payload
/// retained publish.
pub struct DelayedRetainSemanticsTest;

impl TestCase for DelayedRetainSemanticsTest {
    fn name(&self) -> &str {
        "delayed_retain_semantics"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        delayed_config()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(40)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(self.name(), async {
            // Clear any retained message left by a previous run of this test
            // (an empty retained payload deletes the stored entry).
            let cleaner =
                MqttV5Client::connect(&ctx.config.broker_addr, "dly-ret-clean", ctx.config.connect_timeout)
                    .await?;
            cleaner.publish("dly/ret/t", b"", QoS::AtMostOnce, true).await?;
            tokio::time::sleep(Duration::from_millis(300)).await;
            cleaner.disconnect().await?;

            let pubc =
                MqttV5Client::connect(&ctx.config.broker_addr, "dly-ret-pub", ctx.config.connect_timeout)
                    .await?;

            let t0 = Instant::now();
            pubc.publish("$delayed/2/dly/ret/t", b"ret-payload", QoS::AtMostOnce, true).await?;

            // (1) While pending: a fresh subscriber must not receive a
            // RETAINED copy (the store is written only at trigger time). It
            // WILL receive the live delivery at expiry — so the first
            // message must be the live one (retain=false), never a retained
            // copy (retain=true). No drain: the retained copy would arrive
            // right after SUBACK and must not be swallowed.
            tokio::time::sleep(Duration::from_millis(500)).await;
            let mut early =
                MqttV5Client::connect(&ctx.config.broker_addr, "dly-ret-early", ctx.config.connect_timeout)
                    .await?;
            early.subscribe("dly/ret/t", QoS::AtLeastOnce).await?;
            let (topic, msg, _) = wait_for_payload(&mut early, b"ret-payload", Duration::from_secs(6))
                .await
                .ok_or_else(|| anyhow!("early subscriber received no live delivery at trigger time"))?;
            if msg.retain {
                return Err(anyhow!(
                    "retained copy was available during the pending window \
                     (retain stored at publish time instead of trigger time)"
                ));
            }
            if &*topic != "dly/ret/t" {
                return Err(anyhow!("live delivery on wrong topic: {topic}"));
            }
            if t0.elapsed() < Duration::from_millis(1500) {
                return Err(anyhow!("live delivery arrived too early ({:?})", t0.elapsed()));
            }
            early.disconnect().await?;

            // (2) After expiry: a fresh subscriber must receive the retained
            // message (retain flag set). No drain here either — the retained
            // copy arrives right after SUBACK.
            let elapsed = t0.elapsed();
            if elapsed < Duration::from_secs(3) {
                tokio::time::sleep(Duration::from_secs(3) - elapsed).await;
            }
            let mut late =
                MqttV5Client::connect(&ctx.config.broker_addr, "dly-ret-late", ctx.config.connect_timeout)
                    .await?;
            late.subscribe("dly/ret/t", QoS::AtLeastOnce).await?;
            let (_, msg, _) = wait_for_payload(&mut late, b"ret-payload", Duration::from_secs(4))
                .await
                .ok_or_else(|| anyhow!("retained delayed message not delivered after expiry"))?;
            if !msg.retain {
                return Err(anyhow!("message delivered after expiry without the retain flag"));
            }

            pubc.disconnect().await?;
            late.disconnect().await?;
            Ok(())
        })
    }
}

/// P2: `$delayed/0/...` (zero interval) is delivered promptly — either
/// immediately or on the next 500 ms dispatch tick, never seconds later.
pub struct DelayedIntervalZeroTest;

impl TestCase for DelayedIntervalZeroTest {
    fn name(&self) -> &str {
        "delayed_interval_zero"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        delayed_config()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(self.name(), async {
            let mut sub =
                connect_and_subscribe(&ctx.config, "dly-zero-sub", "dly/zero/z", QoS::AtLeastOnce).await?;
            let pubc =
                MqttV5Client::connect(&ctx.config.broker_addr, "dly-zero-pub", ctx.config.connect_timeout)
                    .await?;

            pubc.publish("$delayed/0/dly/zero/z", b"zero-payload", QoS::AtMostOnce, false).await?;
            wait_for_payload(&mut sub, b"zero-payload", Duration::from_millis(2500))
                .await
                .ok_or_else(|| anyhow!("$delayed/0 message not delivered within 2.5s"))?;

            pubc.disconnect().await?;
            sub.disconnect().await?;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// P2 — publish_max overflow (dedicated broker configs)
// ---------------------------------------------------------------------------

/// P2 (`delayed-max1` config: publish_max=1, publish_immediate=true): the
/// message that hits the limit is forwarded immediately (no delay), while
/// the queued one still arrives at its trigger time.
pub struct DelayedMaxOverflowImmediateTest;

impl TestCase for DelayedMaxOverflowImmediateTest {
    fn name(&self) -> &str {
        "delayed_max_overflow_immediate"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        delayed_max1_config()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(40)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(self.name(), async {
            let mut sub =
                connect_and_subscribe(&ctx.config, "dly-max1-sub", "dly/max1/m", QoS::AtLeastOnce).await?;
            let pubc =
                MqttV5Client::connect(&ctx.config.broker_addr, "dly-max1-pub", ctx.config.connect_timeout)
                    .await?;

            let t0 = Instant::now();
            // First: queued (heap was empty, 0 < 1).
            pubc.publish("$delayed/4/dly/max1/m", b"m-first", QoS::AtMostOnce, false).await?;
            // Second: hits publish_max → forwarded immediately.
            pubc.publish("$delayed/4/dly/max1/m", b"m-second", QoS::AtMostOnce, false).await?;

            let (_, _, waited) = wait_for_payload(&mut sub, b"m-second", Duration::from_secs(2))
                .await
                .ok_or_else(|| anyhow!("overflow message was not forwarded immediately"))?;
            let second_at = t0.elapsed();
            let _ = waited;

            // The queued message still arrives at its trigger time.
            wait_for_payload(&mut sub, b"m-first", Duration::from_secs(8))
                .await
                .ok_or_else(|| anyhow!("queued message not delivered after its trigger time"))?;
            let first_at = t0.elapsed();
            if first_at < Duration::from_secs(3) {
                return Err(anyhow!(
                    "queued message delivered too early ({first_at:?}); the delay was not honoured"
                ));
            }
            if second_at >= Duration::from_secs(2) {
                return Err(anyhow!(
                    "overflow message was delayed ({second_at:?}) instead of forwarded immediately"
                ));
            }

            pubc.disconnect().await?;
            sub.disconnect().await?;
            Ok(())
        })
    }
}

/// P2 (`delayed-max1-drop` config: publish_max=1, publish_immediate=false):
/// the message that hits the limit is dropped (never delivered), while the
/// queued one is delivered normally.
pub struct DelayedMaxOverflowDropTest;

impl TestCase for DelayedMaxOverflowDropTest {
    fn name(&self) -> &str {
        "delayed_max_overflow_drop"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        delayed_max1_drop_config()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(40)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(self.name(), async {
            let mut sub =
                connect_and_subscribe(&ctx.config, "dly-drop-sub", "dly/drop/m", QoS::AtLeastOnce).await?;
            let pubc =
                MqttV5Client::connect(&ctx.config.broker_addr, "dly-drop-pub", ctx.config.connect_timeout)
                    .await?;

            // First: queued. Second: dropped (message_dropped hook with
            // Reason::DelayedPublishRefused fires inside the broker).
            pubc.publish("$delayed/4/dly/drop/m", b"d-first", QoS::AtMostOnce, false).await?;
            pubc.publish("$delayed/4/dly/drop/m", b"d-second", QoS::AtMostOnce, false).await?;

            // Nothing before the trigger time...
            assert_silent(&mut sub, b"d-first", Duration::from_millis(1500)).await?;

            // ...then only the queued message arrives.
            wait_for_payload(&mut sub, b"d-first", Duration::from_secs(7))
                .await
                .ok_or_else(|| anyhow!("queued message not delivered"))?;

            // The dropped message must never arrive.
            assert_silent(&mut sub, b"d-second", Duration::from_secs(2)).await?;

            pubc.disconnect().await?;
            sub.disconnect().await?;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// P3 — plugin lifecycle and cluster
// ---------------------------------------------------------------------------

/// P3 (`delayed-flush` config, dedicated broker): unloading the plugin via
/// the HTTP API flushes the pending queue according to
/// `publish_immediate = true` — pending messages are forwarded immediately
/// in trigger-time order (MemDelayedSender::flush_on_exit).
pub struct DelayedPluginUnloadFlushTest;

impl TestCase for DelayedPluginUnloadFlushTest {
    fn name(&self) -> &str {
        "delayed_plugin_unload_flush"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        delayed_flush_config()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(60)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(self.name(), async {
            let mut sub =
                connect_and_subscribe(&ctx.config, "dly-flush-sub", "dly/flush/#", QoS::AtLeastOnce).await?;
            let pubc =
                MqttV5Client::connect(&ctx.config.broker_addr, "dly-flush-pub", ctx.config.connect_timeout)
                    .await?;

            pubc.publish("$delayed/6/dly/flush/a", b"flush-a", QoS::AtMostOnce, false).await?;
            pubc.publish("$delayed/7/dly/flush/b", b"flush-b", QoS::AtMostOnce, false).await?;
            tokio::time::sleep(Duration::from_millis(1000)).await;

            // Unload the plugin (node 1, default config). The sender is
            // dropped → the background task flushes pending messages
            // within one 500 ms tick, oldest trigger time first.
            let status = http_put_status("127.0.0.1:6060", "/api/v1/plugins/1/rmqtt-delayed/unload").await?;
            if status != 200 {
                return Err(anyhow!("plugin unload HTTP status {status} != 200"));
            }

            wait_for_payload(&mut sub, b"flush-a", Duration::from_secs(6))
                .await
                .ok_or_else(|| anyhow!("pending message 'flush-a' not flushed on plugin unload"))?;
            wait_for_payload(&mut sub, b"flush-b", Duration::from_secs(6))
                .await
                .ok_or_else(|| anyhow!("pending message 'flush-b' not flushed on plugin unload"))?;

            pubc.disconnect().await?;
            sub.disconnect().await?;
            Ok(())
        })
    }
}

/// P3: cross-node delayed delivery — the message is queued on node 1 and,
/// at trigger time, routed through the cluster to the subscriber on
/// node 2 (2-node `rmqtt-cluster-broadcast` cluster, both nodes loading
/// `rmqtt-delayed`).
///
/// Self-managed like the issue-#475 cluster tests: configs are generated
/// under `target/delayed-cluster/node{1,2}/` (MQTT 1895/1896, gRPC
/// 5373/5374 — free ports relative to harness 1883/5363, cleanup 1884/5364,
/// cluster-sled 1886-1890/5366-5370, boundary 1891/5365, auth 1892-1893 and
/// 5371-5372). An immediate cross-node publish probe first waits for the
/// cluster to converge before the delayed scenario runs.
pub struct DelayedClusterCrossNodeTest;

impl TestCase for DelayedClusterCrossNodeTest {
    fn name(&self) -> &str {
        "delayed_cluster_cross_node"
    }

    // No broker_config: the harness broker is not used at all. Grouping it
    // with the default-config group keeps this suite self-contained under
    // `--suites delayed`.
    fn timeout(&self) -> Duration {
        Duration::from_secs(150)
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        match rt.block_on(run_cluster_cross_node()) {
            Ok(()) => TestResult::passed(self.name(), "delayed", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "delayed", start.elapsed(), e.to_string()),
        }
    }
}

/// MQTT/gRPC ports for the self-managed delayed cluster nodes.
const CLUSTER_NODE1_ADDR: &str = "127.0.0.1:1895";
const CLUSTER_NODE2_ADDR: &str = "127.0.0.1:1896";
const CLUSTER_RPC1: &str = "0.0.0.0:5373";
const CLUSTER_RPC2: &str = "0.0.0.0:5374";
const CLUSTER_START_TIMEOUT: Duration = Duration::from_secs(30);
const CLUSTER_PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// Generate the self-contained 2-node broadcast-cluster configs (both
/// nodes load `rmqtt-delayed`) under `target/delayed-cluster/`.
fn write_cluster_configs() -> anyhow::Result<(PathBuf, PathBuf)> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    let mk = |node_id: u64, mqtt_addr: &str, rpc_addr: &str| -> anyhow::Result<PathBuf> {
        let dir = root.join("target").join("delayed-cluster").join(format!("node{node_id}"));
        std::fs::create_dir_all(dir.join("plugins"))?;

        // TOML dir strings: `/` works on Windows too; escape `\` anyway.
        let plugins_dir = dir.join("plugins").to_string_lossy().replace('\\', "/");

        let mut body = String::new();
        body.push_str(&format!("node.id = {node_id}\n"));
        body.push_str(&format!("rpc.server_addr = \"{rpc_addr}\"\n"));
        body.push_str("rpc.server_workers = 4\n");
        body.push_str("rpc.batch_size = 128\n");
        body.push_str("rpc.client_concurrency_limit = 128\n");
        body.push_str("rpc.client_timeout = \"5s\"\n");
        body.push_str("log.to = \"console\"\n");
        body.push_str("log.level = \"info\"\n");
        body.push_str("log.dir = \".\"\n");
        body.push_str("log.file = \"rmqtt_delayed_cluster.log\"\n");
        body.push_str(&format!("plugins.dir = \"{plugins_dir}\"\n"));
        body.push_str("plugins.default_startups = [\"rmqtt-cluster-broadcast\", \"rmqtt-delayed\"]\n");
        body.push_str(
            "plugins.disabled_default_startups = [\"rmqtt-acl\", \"rmqtt-counter\", \"rmqtt-http-api\"]\n",
        );
        body.push_str(&format!("listener.tcp.external.addr = \"{mqtt_addr}\"\n"));
        body.push_str("listener.tcp.external.workers = 8\n");
        body.push_str("listener.tcp.external.max_connections = 102400\n");
        body.push_str("listener.tcp.external.max_handshaking_limit = 500\n");
        body.push_str("listener.tcp.external.handshake_timeout = \"30s\"\n");
        body.push_str("listener.tcp.external.max_packet_size = \"1m\"\n");
        body.push_str("listener.tcp.external.backlog = 1024\n");
        body.push_str("listener.tcp.external.allow_anonymous = true\n");
        body.push_str("listener.tcp.external.min_keepalive = 0\n");
        body.push_str("listener.tcp.external.max_qos_allowed = 2\n");
        body.push_str("listener.tcp.internal.enable = false\n");
        body.push_str("listener.tls.external.enable = false\n");
        body.push_str("listener.ws.external.enable = false\n");
        body.push_str("listener.wss.external.enable = false\n");
        body.push_str("listener.quic.external.enable = false\n");
        std::fs::write(dir.join("rmqtt.toml"), body)?;

        // Broadcast cluster: both node gRPC addresses.
        std::fs::write(
            dir.join("plugins").join("rmqtt-cluster-broadcast.toml"),
            "message_type = 98\n\
             node_grpc_addrs = [\"1@127.0.0.1:5373\", \"2@127.0.0.1:5374\"]\n\
             node_grpc_batch_size = 128\n\
             node_grpc_client_concurrency_limit = 128\n\
             node_grpc_client_timeout = \"15s\"\n\
             task_exec_queue_workers = 500\n\
             task_exec_queue_max = 100_000\n",
        )?;
        // Plugin defaults are sufficient here.
        std::fs::write(
            dir.join("plugins").join("rmqtt-delayed.toml"),
            "# defaults: publish_max = 100_000, publish_immediate = true\n",
        )?;
        Ok(dir.join("rmqtt.toml"))
    };

    let cfg1 = mk(1, CLUSTER_NODE1_ADDR, CLUSTER_RPC1)?;
    let cfg2 = mk(2, CLUSTER_NODE2_ADDR, CLUSTER_RPC2)?;
    Ok((cfg1, cfg2))
}

async fn run_cluster_cross_node() -> anyhow::Result<()> {
    use crate::tests::functional::cluster_session_restart::{rmqttd_binary, ClusterNode};

    let binary = rmqttd_binary();
    if !binary.exists() {
        return Err(anyhow!(
            "rmqttd binary not found at {:?}; build it first (cargo build -p rmqttd)",
            binary
        ));
    }
    let (cfg1, cfg2) = write_cluster_configs()?;
    let log_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target");
    let mut node1 = ClusterNode::new(cfg1, CLUSTER_NODE1_ADDR, log_dir.join("delayed-cluster-node1.log"));
    let mut node2 = ClusterNode::new(cfg2, CLUSTER_NODE2_ADDR, log_dir.join("delayed-cluster-node2.log"));

    node1.spawn(&binary)?;
    node2.spawn(&binary)?;
    if !node1.wait_healthy(CLUSTER_START_TIMEOUT) {
        return Err(anyhow!("cluster node1 did not become healthy"));
    }
    if !node2.wait_healthy(CLUSTER_START_TIMEOUT) {
        return Err(anyhow!("cluster node2 did not become healthy"));
    }

    // Subscriber on node 2; publisher on node 1. The `#` filter covers both
    // the convergence probe (`dly/clu/probe`) and the delayed topic
    // (`dly/clu/x`); payload-based matching keeps the two phases apart.
    let mut sub = MqttV5Client::connect(CLUSTER_NODE2_ADDR, "dly-clu-sub", Duration::from_secs(10)).await?;
    sub.subscribe("dly/clu/#", QoS::AtLeastOnce).await?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    while sub.recv_message_timeout(Duration::from_millis(100)).await.is_some() {}

    let pubc = MqttV5Client::connect(CLUSTER_NODE1_ADDR, "dly-clu-pub", Duration::from_secs(10)).await?;

    // Cluster convergence probe: an immediate cross-node publish must
    // arrive before the delayed scenario starts.
    pubc.publish("dly/clu/probe", b"clu-probe", QoS::AtLeastOnce, false).await?;
    wait_for_payload(&mut sub, b"clu-probe", CLUSTER_PROBE_TIMEOUT)
        .await
        .ok_or_else(|| anyhow!("cluster did not converge: immediate cross-node publish was not delivered"))?;

    // Delayed scenario: queued on node 1, delivered on node 2 at trigger
    // time (~3s + dispatch tick), not before.
    pubc.publish("$delayed/3/dly/clu/x", b"clu-payload", QoS::AtLeastOnce, false).await?;
    assert_silent(&mut sub, b"clu-payload", Duration::from_millis(1200)).await?;
    let (topic, _, _) = wait_for_payload(&mut sub, b"clu-payload", Duration::from_secs(8))
        .await
        .ok_or_else(|| anyhow!("delayed message not delivered cross-node within 8s of the trigger window"))?;
    if &*topic != "dly/clu/x" {
        return Err(anyhow!("delivered on wrong topic: {topic}"));
    }

    pubc.disconnect().await?;
    sub.disconnect().await?;
    node1.kill();
    node2.kill();
    Ok(())
}
