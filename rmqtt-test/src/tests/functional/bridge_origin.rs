//! `rmqtt-bridge-origin` suite: bridge-direction detection.
//!
//! The plugin inspects `client_id` on `ClientConnected` and stores a
//! `BridgeOrigin { direction }` in `session.extra_attrs[attr_key]` so other
//! plugins (anti-loop, routing) can read it.
//!
//! **Observability note.** `session.extra_attrs` is not exposed anywhere: the
//! HTTP API has the field commented out (`rmqtt-http-api/src/types.rs:224`) and
//! no hook carries it. The only externally visible evidence is the plugin's own
//! `log::debug!("bridge-origin: detected … bridge client, client_id=…")`, so the
//! fixture (`configs/plugin-bridge-origin/`, MQTT 1940) runs with
//! `log.to = "file"` + `log.level = "debug"` and the cases read that file. This
//! is the same technique `bridge_egress_mqtt` uses for its diagnosability case.
//!
//! Cases:
//!
//! * an ingress-marked and an egress-marked client id each produce their line,
//!   and the marked sessions still publish/subscribe normally (the plugin must
//!   be observation-only, not a gate);
//! * a plain client id produces **no** line — with a marked client connecting
//!   right after it as a liveness control for the log pipeline, so "no line"
//!   cannot pass just because logging is broken.
//!
//! Run:
//!
//! ```text
//! mqtt_harness --binary target/release/rmqttd --workspace . \
//!   --suites functional_v311 -t plugin_bridge_origin
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::anyhow;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v311::MqttV311Client;
use crate::tests::{fixture_config, fixture_logs_dir};

/// MQTT listener of `configs/plugin-bridge-origin/rmqtt.toml`.
const BROKER_ADDR: &str = "127.0.0.1:1940";

/// Marker defaults from `rmqtt-plugins/rmqtt-bridge-origin.toml`.
///
/// Each case uses its **own** marker-bearing ids: the broker log is shared by
/// every case in the sub-suite, so reusing one id would let a later case pass on
/// lines written by an earlier one.
const INGRESS_ID: &str = "origin-detect:ingress:node-b";
const EGRESS_ID: &str = "origin-detect:egress:node-b";
/// Liveness control of the negative case (must not be one of the two above).
const CONTROL_ID: &str = "origin-control:ingress:node-z";

fn fixture() -> Option<PathBuf> {
    Some(fixture_config("plugin-bridge-origin"))
}

fn log_path() -> PathBuf {
    fixture_logs_dir("plugin-bridge-origin").join("plugin-bridge-origin.log")
}

async fn connect(client_id: &str) -> anyhow::Result<MqttV311Client> {
    MqttV311Client::connect_with_options(
        BROKER_ADDR,
        client_id,
        Duration::from_secs(10),
        true,
        60,
        None,
        None,
        None,
    )
    .await
}

/// Poll the broker log until `pred` matches its content or the budget runs out.
async fn wait_for_log_line(budget: Duration, pred: impl Fn(&str) -> bool) -> bool {
    let deadline = Instant::now() + budget;
    loop {
        if let Ok(content) = std::fs::read_to_string(log_path()) {
            if pred(&content) {
                return true;
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Both marker directions are detected.
pub struct PluginBridgeOriginDetectsMarkersTest;

impl TestCase for PluginBridgeOriginDetectsMarkersTest {
    fn name(&self) -> &str {
        "plugin_bridge_origin_detects_markers"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            let ingress = connect(INGRESS_ID).await?;
            let egress = connect(EGRESS_ID).await?;

            // The plugin must not disturb the session: a marked client behaves
            // like any other client.
            let mut ingress = ingress;
            ingress.subscribe("bridge/origin/probe", QoS::AtLeastOnce).await?;
            egress.publish("bridge/origin/probe", b"ping", QoS::AtLeastOnce, false).await?;
            let msg = ingress
                .recv_message_timeout(Duration::from_secs(5))
                .await
                .ok_or_else(|| anyhow!("marked client could not subscribe/publish normally"))?;
            if msg.payload.as_ref() != b"ping" {
                return Err(anyhow!("payload {:?}", String::from_utf8_lossy(&msg.payload)));
            }

            let want_ingress = format!("detected ingress bridge client, client_id={INGRESS_ID}");
            let want_egress = format!("detected egress bridge client, client_id={EGRESS_ID}");
            if !wait_for_log_line(Duration::from_secs(10), |c| c.contains(&want_ingress)).await {
                return Err(anyhow!(
                    "no `{want_ingress}` in {} within 10s; the plugin logs it on \
                     ClientConnected at debug level",
                    log_path().display()
                ));
            }
            if !wait_for_log_line(Duration::from_secs(10), |c| c.contains(&want_egress)).await {
                return Err(anyhow!("no `{want_egress}` in {} within 10s", log_path().display()));
            }

            ingress.disconnect().await?;
            egress.disconnect().await?;
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
        Duration::from_secs(60)
    }
}

/// A client id without a marker is never classified.
pub struct PluginBridgeOriginIgnoresPlainClientIdTest;

impl TestCase for PluginBridgeOriginIgnoresPlainClientIdTest {
    fn name(&self) -> &str {
        "plugin_bridge_origin_ignores_plain_clientid"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            const PLAIN: &str = "origin-plain-client";

            let plain = connect(PLAIN).await?;
            // Give the handler (and the log sink) a chance to react to a
            // connection it should ignore.
            tokio::time::sleep(Duration::from_millis(800)).await;

            // Liveness control: the log pipeline works *now*, so the absence
            // check below is meaningful rather than vacuous.
            let marked = connect(CONTROL_ID).await?;
            let want = format!("detected ingress bridge client, client_id={CONTROL_ID}");
            if !wait_for_log_line(Duration::from_secs(10), |c| c.contains(&want)).await {
                return Err(anyhow!(
                    "liveness control failed: no `{want}` in {} within 10s",
                    log_path().display()
                ));
            }

            // The broker logs every packet at debug level, so "the client id
            // appears somewhere" proves nothing. Only the plugin's own
            // classification lines may mention it.
            let content = std::fs::read_to_string(log_path()).unwrap_or_default();
            let ingress_hit = content.contains(&format!("detected ingress bridge client, client_id={PLAIN}"));
            let egress_hit = content.contains(&format!("detected egress bridge client, client_id={PLAIN}"));
            plain.disconnect().await?;
            marked.disconnect().await?;
            if ingress_hit || egress_hit {
                return Err(anyhow!(
                    "the plugin classified `{PLAIN}` (ingress={ingress_hit}, egress={egress_hit}) \
                     although its client id carries neither `:ingress:` nor `:egress:`"
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
        Duration::from_secs(60)
    }
}
