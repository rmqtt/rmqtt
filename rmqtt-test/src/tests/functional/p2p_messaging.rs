//! `rmqtt-p2p-messaging` suite.
//!
//! Point-to-point delivery: the plugin parses the target `clientid` out of the
//! topic and routes the message straight to that session, so the receiver needs
//! **no subscription at all** (`docs/en_US/p2p-messaging.md`).
//!
//! Fixture: `configs/plugin-p2p/` (MQTT 1939) with `mode = "both"`, so both
//! documented forms are exercised:
//!
//! * prefix — `p2p/{clientid}/{topic}`
//! * suffix — `{topic}/p2p/{clientid}`
//!
//! In both cases the delivered **topic must be the inner one** (`sensor/temp`):
//! `parse_p2p_topic` rewrites `p.topic` before routing, and a case that only
//! checked the payload would not notice a leaked `p2p/...` prefix.
//!
//! Run:
//!
//! ```text
//! mqtt_harness --binary target/release/rmqttd --workspace . \
//!   --suites functional_v311 -t plugin_p2p
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::anyhow;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v311::MqttV311Client;
use crate::tests::fixture_config;

/// MQTT listener of `configs/plugin-p2p/rmqtt.toml`.
const BROKER_ADDR: &str = "127.0.0.1:1939";

fn fixture() -> Option<PathBuf> {
    Some(fixture_config("plugin-p2p"))
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

/// Prefix form: `p2p/{clientid}/{topic}`.
pub struct PluginP2pPrefixTest;

impl TestCase for PluginP2pPrefixTest {
    fn name(&self) -> &str {
        "plugin_p2p_prefix_routes_to_client"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            const TARGET: &str = "p2p-target-prefix";

            // The target never subscribes: only the plugin can deliver here.
            let mut target = connect(TARGET).await?;
            let publisher = connect("p2p-pub-prefix").await?;

            let wire_topic = format!("p2p/{TARGET}/sensor/temp");
            publisher.publish(&wire_topic, b"23.5", QoS::AtLeastOnce, false).await?;

            let msg = target.recv_message_timeout(Duration::from_secs(10)).await.ok_or_else(|| {
                anyhow!(
                    "no delivery for `{wire_topic}`: the prefix form must route straight to \
                     client `{TARGET}` without a subscription"
                )
            })?;
            if &*msg.topic != "sensor/temp" {
                return Err(anyhow!("delivered on `{}`, expected the inner topic `sensor/temp`", msg.topic));
            }
            if msg.payload.as_ref() != b"23.5" {
                return Err(anyhow!("payload {:?}", String::from_utf8_lossy(&msg.payload)));
            }

            // A different client id must not be reached by the same publish.
            let mut other = connect("p2p-bystander").await?;
            publisher.publish(&format!("p2p/{TARGET}/sensor/hum"), b"40", QoS::AtMostOnce, false).await?;
            if let Some(msg) = other.recv_message_timeout(Duration::from_secs(2)).await {
                return Err(anyhow!("a bystander received `{}` that was addressed to `{TARGET}`", msg.topic));
            }

            target.disconnect().await?;
            other.disconnect().await?;
            publisher.disconnect().await?;
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

/// Suffix form: `{topic}/p2p/{clientid}`.
pub struct PluginP2pSuffixTest;

impl TestCase for PluginP2pSuffixTest {
    fn name(&self) -> &str {
        "plugin_p2p_suffix_routes_to_client"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            const TARGET: &str = "p2p-target-suffix";

            let mut target = connect(TARGET).await?;
            let publisher = connect("p2p-pub-suffix").await?;

            let wire_topic = format!("sensor/temp/p2p/{TARGET}");
            publisher.publish(&wire_topic, b"suffix-payload", QoS::AtLeastOnce, false).await?;

            let msg = target.recv_message_timeout(Duration::from_secs(10)).await.ok_or_else(|| {
                anyhow!(
                    "no delivery for `{wire_topic}`: with `mode = \"both\"` the suffix form must \
                     route straight to client `{TARGET}`"
                )
            })?;
            if &*msg.topic != "sensor/temp" {
                return Err(anyhow!("delivered on `{}`, expected the inner topic `sensor/temp`", msg.topic));
            }
            if msg.payload.as_ref() != b"suffix-payload" {
                return Err(anyhow!("payload {:?}", String::from_utf8_lossy(&msg.payload)));
            }

            target.disconnect().await?;
            publisher.disconnect().await?;
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
