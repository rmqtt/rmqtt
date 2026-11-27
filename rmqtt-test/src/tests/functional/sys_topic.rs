//! `$SYS` system-topic suite (`rmqtt-sys-topic`).
//!
//! The plugin ships in every production build, but the harness only ever
//! loaded it as a side effect: no case asserted *what* it publishes. The
//! default harness config does not start it either, so this suite owns a
//! dedicated fixture (`configs/plugin-sys-topic/`, MQTT 1930) with
//! `publish_interval = "1s"` (the plugin clamps the interval to a 1s floor).
//!
//! Contract under test:
//!
//! * `$SYS/brokers/{node}/stats` and `$SYS/brokers/{node}/metrics` are
//!   republished every interval, with the stats/metrics JSON body the HTTP API
//!   hands out as well (`rmqtt/src/stats.rs::to_json`, `Metrics::to_json`);
//! * the per-event topics `$SYS/brokers/{node}/session/{clientid}/created` and
//!   `$SYS/brokers/{node}/clients/{clientid}/connected` fire on session
//!   creation / connection, and only for the event that actually happened
//!   (`rmqtt-plugins/rmqtt-sys-topic/src/lib.rs`, `SystemTopicHandler`).
//!
//! Everything is published with `retain = false`, so a fresh subscriber only
//! sees messages published *after* its SUBACK — the cases subscribe first and
//! then wait, which makes the assertions meaningful.
//!
//! Run:
//!
//! ```text
//! mqtt_harness --binary target/release/rmqttd --workspace . \
//!   --suites functional_v311 -t plugin_sys_topic
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use serde_json::Value;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v311::MqttV311Client;
use crate::tests::fixture_config;

/// MQTT listener of `configs/plugin-sys-topic/rmqtt.toml`.
const BROKER_ADDR: &str = "127.0.0.1:1930";

/// How long the cases wait for the periodic (1s) publications. Generous
/// enough to absorb a slow CI machine, short enough to fail loudly when the
/// plugin is not running.
const WAIT_BUDGET: Duration = Duration::from_secs(12);

fn fixture() -> Option<PathBuf> {
    Some(fixture_config("plugin-sys-topic"))
}

async fn connect(client_id: &str, timeout: Duration) -> anyhow::Result<MqttV311Client> {
    // The fixture keeps `allow_anonymous = true` and starts no auth plugin, so
    // `None`/`None` credentials are the normal path.
    MqttV311Client::connect_with_options(BROKER_ADDR, client_id, timeout, true, 60, None, None, None).await
}

/// Periodic `stats` / `metrics` publication.
///
/// Both topics must appear within the wait budget, and both payloads must be
/// the JSON objects the HTTP API serves: `stats` carries the flat
/// `"<category>.<field>"` keys (`sessions.count`, `connections.count`), and
/// `metrics` carries the dotted counter names the `Metrics` derive generates
/// (`client.connect`). A bare "some payload arrived" assertion would not catch
/// a stats/metrics payload swap, which is why the key checks are per topic.
pub struct PluginSysTopicStatsMetricsTest;

impl TestCase for PluginSysTopicStatsMetricsTest {
    fn name(&self) -> &str {
        "plugin_sys_topic_stats_and_metrics"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            let mut sub = connect("sys-stats-sub", Duration::from_secs(10)).await?;
            sub.subscribe("$SYS/brokers/1/#", QoS::AtLeastOnce).await?;

            let deadline = Instant::now() + WAIT_BUDGET;
            let (mut stats, mut metrics) = (None, None);
            while (stats.is_none() || metrics.is_none()) && Instant::now() < deadline {
                let left = deadline.saturating_duration_since(Instant::now());
                let Some(msg) = sub.recv_message_timeout(left.min(Duration::from_secs(2))).await else {
                    continue;
                };
                match &*msg.topic {
                    "$SYS/brokers/1/stats" => stats = Some(msg.payload),
                    "$SYS/brokers/1/metrics" => metrics = Some(msg.payload),
                    _ => {}
                }
            }

            let stats = stats.ok_or_else(|| {
                anyhow!(
                    "no `$SYS/brokers/1/stats` within {}s; the sys-topic plugin publishes it \
                     every `publish_interval` (1s in this fixture)",
                    WAIT_BUDGET.as_secs()
                )
            })?;
            let stats: Value = serde_json::from_slice(&stats)
                .map_err(|e| anyhow!("`$SYS/brokers/1/stats` payload is not JSON: {e}"))?;
            for key in ["sessions.count", "connections.count"] {
                if stats.get(key).is_none() {
                    return Err(anyhow!("`$SYS/brokers/1/stats` has no `{key}` key, payload: {stats}"));
                }
            }

            let metrics = metrics.ok_or_else(|| {
                anyhow!(
                    "no `$SYS/brokers/1/metrics` within {}s; the sys-topic plugin publishes it \
                     every `publish_interval` (1s in this fixture)",
                    WAIT_BUDGET.as_secs()
                )
            })?;
            let metrics: Value = serde_json::from_slice(&metrics)
                .map_err(|e| anyhow!("`$SYS/brokers/1/metrics` payload is not JSON: {e}"))?;
            // `Metrics::to_json` (rmqtt-macros) maps `client_connect` to
            // `client.connect`; the connecting subscriber in this very case
            // guarantees the key exists.
            if metrics.get("client.connect").is_none() {
                return Err(anyhow!(
                    "`$SYS/brokers/1/metrics` has no `client.connect` counter, payload: {metrics}"
                ));
            }

            sub.disconnect().await?;
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
        Duration::from_secs(45)
    }
}

/// Per-event topics for session creation and connection.
///
/// The subscriber watches the two wildcards *before* the second client
/// connects, then expects exactly those two events for that client id. The
/// payload must name the client, so a case that only matched the topic shape
/// (and delivered the event of a different session) would still fail.
pub struct PluginSysTopicClientEventsTest;

impl TestCase for PluginSysTopicClientEventsTest {
    fn name(&self) -> &str {
        "plugin_sys_topic_client_events"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            const VICTIM: &str = "sys-evt-victim";

            let mut sub = connect("sys-evt-sub", Duration::from_secs(10)).await?;
            sub.subscribe("$SYS/brokers/1/session/+/created", QoS::AtLeastOnce).await?;
            sub.subscribe("$SYS/brokers/1/clients/+/connected", QoS::AtLeastOnce).await?;

            // The victim connects *after* the subscriptions are in place: the
            // event publications are not retained, so this is the only way to
            // observe them deterministically.
            let victim = connect(VICTIM, Duration::from_secs(10)).await?;

            let want_created = format!("$SYS/brokers/1/session/{VICTIM}/created");
            let want_connected = format!("$SYS/brokers/1/clients/{VICTIM}/connected");
            let deadline = Instant::now() + WAIT_BUDGET;
            let (mut created, mut connected) = (false, false);
            while (!created || !connected) && Instant::now() < deadline {
                let left = deadline.saturating_duration_since(Instant::now());
                let Some(msg) = sub.recv_message_timeout(left.min(Duration::from_secs(2))).await else {
                    continue;
                };
                if *msg.topic != want_created && *msg.topic != want_connected {
                    continue;
                }
                let body: Value = match serde_json::from_slice(&msg.payload) {
                    Ok(v) => v,
                    Err(e) => {
                        return Err(anyhow!(
                            "`{}` payload is not JSON: {e} (payload: {:?})",
                            msg.topic,
                            String::from_utf8_lossy(&msg.payload)
                        ));
                    }
                };
                if body.get("clientid").and_then(Value::as_str) != Some(VICTIM) {
                    return Err(anyhow!("`{}` does not name the client, payload: {body}", msg.topic));
                }
                if *msg.topic == want_created {
                    created = true;
                } else {
                    connected = true;
                }
            }

            victim.disconnect().await?;
            sub.disconnect().await?;

            if !created {
                return Err(anyhow!("no `{want_created}` within {}s", WAIT_BUDGET.as_secs()));
            }
            if !connected {
                return Err(anyhow!("no `{want_connected}` within {}s", WAIT_BUDGET.as_secs()));
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
        Duration::from_secs(45)
    }
}
