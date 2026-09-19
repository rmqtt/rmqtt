//! Shared helpers for the Pulsar bridge suite (`--suites pulsar`).
//!
//! The suite verifies both Pulsar bridge plugins end to end:
//!   * `rmqtt-bridge-egress-pulsar`  : MQTT publish   -> Pulsar topic
//!   * `rmqtt-bridge-ingress-pulsar` : Pulsar message -> MQTT publish
//!
//! The observers are deliberately *independent* of the plugins: every test
//! builds its own `pulsar::Pulsar` client (a producer to inject messages, a
//! consumer to observe them). That way a failure can be attributed to the
//! plugin under test instead of to a second bridge, and no egress->ingress
//! loop is needed to observe traffic.
//!
//! Requirements: an **external** Apache Pulsar service at
//! `pulsar://127.0.0.1:6650` (see `designs/pulsar-bridge-test-plan.md`).
//! When it is unreachable, every test reports `Skipped` -- the suite never
//! fails just because the service is absent.

use std::collections::HashMap;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use futures::StreamExt;
use pulsar::consumer::{Consumer, InitialPosition};
use pulsar::{consumer, producer, ConsumerOptions, Pulsar, SubType, TokioExecutor};

use crate::framework::testcase::TestResult;
use crate::mqtt::v311::MqttV311Client;
use crate::mqtt::v5::MqttV5Client;

/// Suite name reported in every verdict.
pub const SUITE: &str = "pulsar";

/// Pulsar service URL (provided by the environment, never started by the tests).
pub const PULSAR_URL: &str = "pulsar://127.0.0.1:6650";
/// Plain TCP endpoint used for the availability probe.
const PULSAR_TCP_ADDR: &str = "127.0.0.1:6650";

/// Default harness broker address. The `pulsar` config keeps 1883 so the
/// harness health check (which always targets `--addr`) keeps working, but the
/// tests take the address from `TestContext` so they also work in `--no-broker`
/// mode / with `--addr` overrides.
pub const DEFAULT_MQTT_ADDR: &str = "127.0.0.1:1883";

/// Remote Pulsar topics, must stay in sync with
/// `rmqtt-test/configs/pulsar/plugins/rmqtt-bridge-{egress,ingress}-pulsar.toml`.
pub const EGRESS_TOPIC: &str = "persistent://public/default/rmqtt-test-egress";
pub const EGRESS_SKIP_TOPIC: &str = "persistent://public/default/rmqtt-test-egress-skip";
pub const INGRESS_TOPIC: &str = "persistent://public/default/rmqtt-test-ingress";
pub const INGRESS_PH_TOPIC: &str = "persistent://public/default/rmqtt-test-ingress-ph";

/// MQTT topic prefix routed to `EGRESS_TOPIC` by the egress plugin.
pub const MQTT_EGRESS_PREFIX: &str = "pulsar/egress";
/// MQTT topic the ingress plugin publishes to (fixed mapping).
pub const MQTT_INGRESS_TOPIC: &str = "pulsar/ingress/pulsar";

/// MQTT / Pulsar I/O timeout for a single client operation.
pub const IO_TIMEOUT: Duration = Duration::from_secs(10);
/// Time budget for one bridge round trip (egress or ingress hop).
pub const RECV_TIMEOUT: Duration = Duration::from_secs(20);
/// Short budget used to prove that *nothing* arrives.
pub const NEGATIVE_TIMEOUT: Duration = Duration::from_secs(4);
/// Loopback connect probe; 500 ms is plenty.
const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

pub type PulsarClient = Pulsar<TokioExecutor>;
pub type PulsarProducer = producer::Producer<TokioExecutor>;
pub type PulsarConsumer = Consumer<Vec<u8>, TokioExecutor>;

// ---------------------------------------------------------------------------
// Environment probing
// ---------------------------------------------------------------------------

/// Whether the external Pulsar service accepts TCP connections.
pub fn pulsar_available() -> bool {
    match PULSAR_TCP_ADDR.parse() {
        Ok(addr) => TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).is_ok(),
        Err(_) => false,
    }
}

/// `Some(Skipped verdict)` when the external Pulsar service is unreachable.
///
/// Call this first in every test:
/// ```ignore
/// if let Some(skip) = skip_if_pulsar_down(self.name(), start) { return skip; }
/// ```
pub fn skip_if_pulsar_down(name: &str, start: Instant) -> Option<TestResult> {
    if pulsar_available() {
        None
    } else {
        Some(TestResult::skipped(
            name,
            SUITE,
            start.elapsed(),
            "external Pulsar service unreachable at 127.0.0.1:6650",
        ))
    }
}

/// Unique marker for one test run, embedded in payloads so a test can ignore
/// messages left behind by previous runs on the persistent topics.
pub fn nonce() -> String {
    format!("pulsar-test-{}", uuid::Uuid::new_v4())
}

/// Payload carrying a `prefix` plus a unique nonce.
pub fn tagged_payload(prefix: &str) -> Vec<u8> {
    let mut payload = prefix.as_bytes().to_vec();
    payload.push(b'-');
    payload.extend_from_slice(nonce().as_bytes());
    payload
}

// ---------------------------------------------------------------------------
// Pulsar clients (the independent observers)
// ---------------------------------------------------------------------------

/// Connects to the external Pulsar service.
pub async fn connect_pulsar() -> Result<PulsarClient, anyhow::Error> {
    Ok(Pulsar::builder(PULSAR_URL, TokioExecutor).build().await?)
}

/// Creates a producer for `topic` with a unique name.
pub async fn create_producer(client: &PulsarClient, topic: &str) -> Result<PulsarProducer, anyhow::Error> {
    let name = format!("rmqtt-test-producer-{}", nonce());
    Ok(client.producer().with_topic(topic).with_name(name).build().await?)
}

/// Creates a **non-durable, latest-position** observer consumer.
///
/// * The subscription name is unique per call: the ingress plugin owns its own
///   exclusive subscription, and a same-named test consumer would compete with
///   it for messages.
/// * `InitialPosition::Latest` means only messages published *after* this call
///   are seen, so leftover data on the persistent topics cannot pollute the
///   assertion.
pub async fn create_observer(
    client: &PulsarClient,
    topic: &str,
    suffix: &str,
) -> Result<PulsarConsumer, anyhow::Error> {
    let name = format!("rmqtt-test-observer-{suffix}");
    let consumer = client
        .consumer()
        .with_topic(topic)
        .with_consumer_name(name.clone())
        .with_subscription(name)
        .with_subscription_type(SubType::Exclusive)
        .with_options(ConsumerOptions {
            durable: Some(false),
            initial_position: InitialPosition::Latest,
            ..Default::default()
        })
        .build::<Vec<u8>>()
        .await?;
    Ok(consumer)
}

/// Publishes one message and waits for the send receipt, so the test never
/// races the service.
pub async fn publish_pulsar_message(
    producer: &mut PulsarProducer,
    payload: &[u8],
    properties: HashMap<String, String>,
) -> Result<(), anyhow::Error> {
    let message = producer::Message { payload: payload.to_vec(), properties, ..Default::default() };
    let fut = producer.send_non_blocking(message).await?;
    fut.await?;
    Ok(())
}

/// Receives the next message whose payload equals `expected` within `timeout`.
///
/// Messages that do not match are acknowledged and skipped (possible leftovers
/// of an earlier run), so the assertion only sees the current run's traffic.
pub async fn recv_pulsar_payload(
    consumer: &mut PulsarConsumer,
    expected: &[u8],
    timeout: Duration,
) -> Result<consumer::Message<Vec<u8>>, anyhow::Error> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(anyhow::anyhow!(
                "timed out after {timeout:?} waiting for a Pulsar message with the expected \
                 payload ({} bytes)",
                expected.len()
            ));
        }
        match tokio::time::timeout(remaining, consumer.next()).await {
            // Timed out: the deadline check at the top of the loop reports it.
            Err(_) => continue,
            Ok(None) => return Err(anyhow::anyhow!("Pulsar consumer stream ended unexpectedly")),
            Ok(Some(Ok(msg))) => {
                if msg.payload.data == expected {
                    return Ok(msg);
                }
                consumer
                    .ack(&msg)
                    .await
                    .map_err(|e| anyhow::anyhow!("failed to ack a stale Pulsar message: {e}"))?;
            }
            Ok(Some(Err(e))) => return Err(anyhow::anyhow!("Pulsar consumer error: {e}")),
        }
    }
}

/// Acknowledges a message that a test consumed as evidence.
pub async fn ack_pulsar_message(
    consumer: &mut PulsarConsumer,
    msg: &consumer::Message<Vec<u8>>,
) -> Result<(), anyhow::Error> {
    consumer.ack(msg).await.map_err(|e| anyhow::anyhow!("failed to ack Pulsar message: {e}"))
}

/// Asserts that no message arrives within `timeout`.
pub async fn expect_no_pulsar_message(
    consumer: &mut PulsarConsumer,
    timeout: Duration,
) -> Result<(), anyhow::Error> {
    match tokio::time::timeout(timeout, consumer.next()).await {
        Err(_) | Ok(None) => Ok(()),
        Ok(Some(Ok(msg))) => Err(anyhow::anyhow!(
            "unexpected Pulsar message: topic={}, payload={:?}",
            msg.topic,
            String::from_utf8_lossy(&msg.payload.data)
        )),
        Ok(Some(Err(e))) => Err(anyhow::anyhow!("Pulsar consumer error: {e}")),
    }
}

/// Value of a Pulsar message property, if present.
pub fn property(msg: &consumer::Message<Vec<u8>>, key: &str) -> Option<String> {
    msg.payload.metadata.properties.iter().find(|kv| kv.key == key).map(|kv| kv.value.clone())
}

/// All Pulsar message properties as key/value pairs (for diagnostics).
pub fn properties(msg: &consumer::Message<Vec<u8>>) -> Vec<(String, String)> {
    msg.payload.metadata.properties.iter().map(|kv| (kv.key.clone(), kv.value.clone())).collect()
}

// ---------------------------------------------------------------------------
// MQTT clients
// ---------------------------------------------------------------------------

/// Connects a v3.1.1 client to the broker under test.
pub async fn mqtt_v311(mqtt_addr: &str, client_id: &str) -> Result<MqttV311Client, anyhow::Error> {
    MqttV311Client::connect(mqtt_addr, client_id, IO_TIMEOUT).await.map_err(|e| anyhow::anyhow!("{e}"))
}

/// Connects a v5 client to the broker under test (needed to assert User Properties).
pub async fn mqtt_v5(mqtt_addr: &str, client_id: &str) -> Result<MqttV5Client, anyhow::Error> {
    MqttV5Client::connect(mqtt_addr, client_id, IO_TIMEOUT).await.map_err(|e| anyhow::anyhow!("{e}"))
}

// ---------------------------------------------------------------------------
// Verdict plumbing
// ---------------------------------------------------------------------------

/// Runs one asynchronous test body and maps the outcome onto a verdict:
///
/// 1. probe the external Pulsar service -> `Skipped` when unreachable;
/// 2. run the body on a **dedicated thread with a 32 MiB stack**. Each body
///    inlines a Pulsar client, a producer/consumer and an MQTT client, so its
///    future is far larger than the 1 MiB Windows main-thread stack: polling
///    it directly from the harness main thread overflows the stack before the
///    first test reports anything;
/// 3. `Passed` / `Failed(..)` from the body's result.
///
/// `mqtt_addr` is the broker address reported by `TestContext`, so the same
/// tests can run against the harness-managed broker (1883), a custom `--addr`
/// or an externally started broker in `--no-broker` mode.
pub fn run_async<F, Fut>(name: &str, mqtt_addr: String, body: F) -> TestResult
where
    F: FnOnce(String) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<(), anyhow::Error>> + Send + 'static,
{
    let start = Instant::now();
    if let Some(skip) = skip_if_pulsar_down(name, start) {
        return skip;
    }
    match run_on_test_thread(body, mqtt_addr) {
        Ok(()) => TestResult::passed(name, SUITE, start.elapsed()),
        Err(e) => TestResult::failed(name, SUITE, start.elapsed(), e.to_string()),
    }
}

/// Like [`run_async`], but the body may return an observation `note`, which is
/// surfaced in the report as `passed_with_note` instead of failing the test.
///
/// Used for behaviours that are *observed* rather than required, e.g. whether
/// the ingress bridge preserves the order of consecutive messages.
pub fn run_async_note<F, Fut>(name: &str, mqtt_addr: String, body: F) -> TestResult
where
    F: FnOnce(String) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<Option<String>, anyhow::Error>> + Send + 'static,
{
    let start = Instant::now();
    if let Some(skip) = skip_if_pulsar_down(name, start) {
        return skip;
    }
    match run_on_test_thread(body, mqtt_addr) {
        Ok(Some(note)) => TestResult::passed_with_note(name, SUITE, start.elapsed(), &note),
        Ok(None) => TestResult::passed(name, SUITE, start.elapsed()),
        Err(e) => TestResult::failed(name, SUITE, start.elapsed(), e.to_string()),
    }
}

/// Runs `body` (which builds its own tokio runtime) on a thread with a 32 MiB
/// stack and returns its result.
fn run_on_test_thread<F, Fut, T>(body: F, mqtt_addr: String) -> Result<T, anyhow::Error>
where
    F: FnOnce(String) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, anyhow::Error>> + Send + 'static,
    T: Send + 'static,
{
    let spawned = std::thread::Builder::new()
        .name("pulsar-test".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| anyhow::anyhow!("failed to create tokio runtime: {e}"))?;
            rt.block_on(Box::pin(body(mqtt_addr)))
        });

    match spawned {
        Ok(handle) => match handle.join() {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!("test thread panicked")),
        },
        Err(e) => Err(anyhow::anyhow!("failed to spawn test thread: {e}")),
    }
}
