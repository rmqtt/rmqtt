//! Offline-queue cap suite (`max_mqueue_len`) and its **overflow policy**.
//!
//! `max_mqueue_len` bounds the per-session queue of messages that arrive while
//! the client is offline. What happens when it is full is decided per message
//! QoS in `Session::deliver_queue_channel` (`rmqtt/src/session.rs:1372`):
//!
//! ```text
//! if p.qos == QoS::AtMostOnce { Policy::Current } else { Policy::Early }
//! ```
//!
//! * `Policy::Early`  — drop the **oldest** queued message and keep the new one
//!   (`queue::Sender::send`, `rmqtt/src/queue.rs:147`);
//! * `Policy::Current` — drop the **incoming** message and keep the queue.
//!
//! No case asserted any of this: fixtures only ever carried
//! `max_mqueue_len = 1000` as a baseline, so both arms of the policy were free
//! to change silently. Fixture `configs/plugin-queue-limits/` (MQTT 1933) pins
//! the cap to 3 and starts no plugin (`rmqtt-message-storage` in particular is
//! absent, so nothing may replay what the queue dropped).
//!
//! Run:
//!
//! ```text
//! mqtt_harness --binary target/release/rmqttd --workspace . \
//!   --suites functional_v311 -t plugin_queue
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::anyhow;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v311::MqttV311Client;
use crate::tests::fixture_config;

/// MQTT listener of `configs/plugin-queue-limits/rmqtt.toml`.
const BROKER_ADDR: &str = "127.0.0.1:1933";

/// The cap the fixture pins.
const QUEUE_CAP: usize = 3;

/// Messages published while the subscriber is away.
const SENT: usize = 6;

fn fixture() -> Option<PathBuf> {
    Some(fixture_config("plugin-queue-limits"))
}

async fn connect(client_id: &str, clean_session: bool) -> anyhow::Result<MqttV311Client> {
    MqttV311Client::connect_with_options(
        BROKER_ADDR,
        client_id,
        Duration::from_secs(10),
        clean_session,
        60,
        None,
        None,
        None,
    )
    .await
}

/// Prepare a persistent session subscribed to `topic`, disconnect it, publish
/// `SENT` messages at `qos`, then resume and return the payloads received.
async fn overflow_run(topic: &str, client: &str, qos: QoS) -> anyhow::Result<Vec<String>> {
    // 1. Persistent session with one subscription, then offline.
    let mut sub = connect(client, false).await?;
    sub.subscribe(topic, QoS::AtLeastOnce).await?;
    sub.disconnect().await?;
    // The DISCONNECT must be processed before the messages are published,
    // otherwise they are routed to a still-connected session instead of the
    // offline queue.
    tokio::time::sleep(Duration::from_millis(400)).await;

    // 2. Publish more than the cap while the subscriber is away.
    let publisher = connect("queue-cap-pub", true).await?;
    for i in 0..SENT {
        publisher.publish(topic, format!("m{i}").as_bytes(), qos, false).await?;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 3. Resume and collect whatever the queue still owes.
    let mut resumed = connect(client, false).await?;
    let mut got = Vec::new();
    while got.len() <= QUEUE_CAP {
        let Some(msg) = resumed.recv_message_timeout(Duration::from_secs(3)).await else {
            break;
        };
        got.push(String::from_utf8_lossy(&msg.payload).to_string());
    }

    resumed.disconnect().await?;
    publisher.disconnect().await?;
    Ok(got)
}

/// QoS 1 overflow drops the **oldest** message (`Policy::Early`).
pub struct PluginQueueQos1OverflowDropsOldestTest;

impl TestCase for PluginQueueQos1OverflowDropsOldestTest {
    fn name(&self) -> &str {
        "plugin_queue_qos1_overflow_drops_oldest"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            let got = overflow_run("q/cap/qos1", "queue-cap-qos1", QoS::AtLeastOnce).await?;
            let expected: Vec<String> = (SENT - QUEUE_CAP..SENT).map(|i| format!("m{i}")).collect();
            if got != expected {
                return Err(anyhow!(
                    "resumed session received {got:?}; with `max_mqueue_len = {QUEUE_CAP}`, \
                     {SENT} QoS 1 publishes and `Policy::Early` the queue must drop the oldest \
                     and owe exactly {expected:?}"
                ));
            }
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), format!("{e:#}")),
        }
    }

    fn broker_config(&self) -> Option<PathBuf> {
        fixture()
    }

    fn broker_addr(&self) -> Option<&'static str> {
        Some(BROKER_ADDR)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(90)
    }
}

/// QoS 0 overflow drops the **incoming** message (`Policy::Current`).
pub struct PluginQueueQos0OverflowDropsNewestTest;

impl TestCase for PluginQueueQos0OverflowDropsNewestTest {
    fn name(&self) -> &str {
        "plugin_queue_qos0_overflow_drops_newest"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            let got = overflow_run("q/cap/qos0", "queue-cap-qos0", QoS::AtMostOnce).await?;
            let expected: Vec<String> = (0..QUEUE_CAP).map(|i| format!("m{i}")).collect();
            if got != expected {
                return Err(anyhow!(
                    "resumed session received {got:?}; with `max_mqueue_len = {QUEUE_CAP}`, \
                     {SENT} QoS 0 publishes and `Policy::Current` the queue must keep the oldest \
                     and drop the incoming messages beyond the cap, i.e. owe exactly {expected:?}"
                ));
            }
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), format!("{e:#}")),
        }
    }

    fn broker_config(&self) -> Option<PathBuf> {
        fixture()
    }

    fn broker_addr(&self) -> Option<&'static str> {
        Some(BROKER_ADDR)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(90)
    }
}
