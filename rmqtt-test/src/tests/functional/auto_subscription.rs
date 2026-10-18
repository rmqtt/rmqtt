//! `rmqtt-auto-subscription` suite.
//!
//! The plugin subscribes a client to configured filters as soon as it
//! connected — including the `${clientid}` / `${username}` placeholders
//! (`rmqtt-plugins/rmqtt-auto-subscription/src/lib.rs`). It had **no fixture
//! and no test** before this suite.
//!
//! Fixture: `configs/plugin-auto-sub/` (MQTT 1937) with
//!
//! ```toml
//! subscribes = [
//!     { topic_filter = "as/${clientid}/#", qos = 1 },
//!     { topic_filter = "asu/${username}/#", qos = 1 },
//! ]
//! ```
//!
//! Each case proves the substitution **and** the resulting filter's boundary:
//! a message for a *different* client id / username must not arrive, otherwise
//! the case would pass on a wildcard that was never substituted (e.g. a literal
//! `${clientid}` level or a bare `as/#`).
//!
//! Run:
//!
//! ```text
//! mqtt_harness --binary target/release/rmqttd --workspace . \
//!   --suites functional_v311 -t plugin_auto_sub
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use bytestring::ByteString;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v311::MqttV311Client;
use crate::tests::fixture_config;

/// MQTT listener of `configs/plugin-auto-sub/rmqtt.toml`.
const BROKER_ADDR: &str = "127.0.0.1:1937";

fn fixture() -> Option<PathBuf> {
    Some(fixture_config("plugin-auto-sub"))
}

async fn connect(client_id: &str, username: Option<&str>) -> anyhow::Result<MqttV311Client> {
    MqttV311Client::connect_with_options(
        BROKER_ADDR,
        client_id,
        Duration::from_secs(10),
        true,
        60,
        None,
        username.map(ByteString::from),
        None,
    )
    .await
}

/// `${clientid}` in the configured filter is replaced by the real client id.
pub struct PluginAutoSubscriptionClientIdTest;

impl TestCase for PluginAutoSubscriptionClientIdTest {
    fn name(&self) -> &str {
        "plugin_auto_sub_clientid_placeholder"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            const CLIENT: &str = "as-cid-1";

            // The client itself never subscribes: everything it receives comes
            // from the auto-subscription rule.
            let mut sub = connect(CLIENT, None).await?;
            // `client_connected` fires right after CONNACK; give the injected
            // SUBSCRIBE a moment to land before publishing.
            tokio::time::sleep(Duration::from_millis(500)).await;

            let publisher = connect("as-pub", None).await?;
            publisher.publish(&format!("as/{CLIENT}/live"), b"for-me", QoS::AtLeastOnce, false).await?;
            let msg = sub.recv_message_timeout(Duration::from_secs(10)).await.ok_or_else(|| {
                anyhow!(
                    "no delivery on `as/{CLIENT}/live`: the `${{clientid}}` rule \
                     (`as/${{clientid}}/#`) did not produce a subscription for `{CLIENT}`"
                )
            })?;
            if msg.payload.as_ref() != b"for-me" {
                return Err(anyhow!("payload {:?}", String::from_utf8_lossy(&msg.payload)));
            }

            // Boundary: the substituted filter must not cover other client ids.
            publisher.publish("as/as-cid-2/live", b"not-for-me", QoS::AtLeastOnce, false).await?;
            if let Some(msg) = sub.recv_message_timeout(Duration::from_secs(2)).await {
                return Err(anyhow!(
                    "received `{}` meant for another client id; the rule subscribed to a filter \
                     wider than `as/{CLIENT}/#`",
                    msg.topic
                ));
            }

            sub.disconnect().await?;
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

/// `${username}` in the configured filter is replaced by the CONNECT username.
pub struct PluginAutoSubscriptionUsernameTest;

impl TestCase for PluginAutoSubscriptionUsernameTest {
    fn name(&self) -> &str {
        "plugin_auto_sub_username_placeholder"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            const CLIENT: &str = "as-username-client";
            const USER: &str = "as-user-1";

            let mut sub = connect(CLIENT, Some(USER)).await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            let publisher = connect("asu-pub", None).await?;
            publisher.publish(&format!("asu/{USER}/live"), b"for-user", QoS::AtLeastOnce, false).await?;
            let msg = sub.recv_message_timeout(Duration::from_secs(10)).await.ok_or_else(|| {
                anyhow!(
                    "no delivery on `asu/{USER}/live`: the `${{username}}` rule \
                     (`asu/${{username}}/#`) did not produce a subscription for username `{USER}` \
                     (client id `{CLIENT}`)"
                )
            })?;
            if msg.payload.as_ref() != b"for-user" {
                return Err(anyhow!("payload {:?}", String::from_utf8_lossy(&msg.payload)));
            }

            publisher.publish("asu/other-user/live", b"not-for-me", QoS::AtLeastOnce, false).await?;
            if let Some(msg) = sub.recv_message_timeout(Duration::from_secs(2)).await {
                return Err(anyhow!(
                    "received `{}` addressed to another username; the substituted filter is wider \
                     than `asu/{USER}/#`",
                    msg.topic
                ));
            }

            sub.disconnect().await?;
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
