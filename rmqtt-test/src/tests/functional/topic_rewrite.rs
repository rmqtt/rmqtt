//! `rmqtt-topic-rewrite` suite.
//!
//! The plugin rewrites PUBLISH topics and SUBSCRIBE/UNSUBSCRIBE filters
//! according to `rules = [{ action, source_topic_filter, dest_topic, regex }]`.
//! It had **no fixture and no test**, although it is the only supported way to
//! migrate subscriptions for devices that cannot be reconfigured.
//!
//! Fixture: `configs/plugin-topic-rewrite/` (MQTT 1938)
//!
//! ```toml
//! rules = [
//!     { action = "publish",   source_topic_filter = "rw/pub/#",     dest_topic = "rw/out/$1",   regex = "^rw/pub/(.+)$" },
//!     { action = "subscribe", source_topic_filter = "rw/sub/old/#", dest_topic = "rw/sub/new/#" },
//! ]
//! ```
//!
//! Cases:
//!
//! * a PUBLISH on `rw/pub/<rest>` reaches a `rw/out/#` subscriber on
//!   `rw/out/<rest>`, and **not** on the original topic (a rewrite, not a copy);
//! * a SUBSCRIBE on `rw/sub/old/<x>` matches publishes on `rw/sub/new/<x>`, and
//!   the documented `$share/<group>/` rule holds: only the filter behind the
//!   group prefix is rewritten, the group itself is kept
//!   (`docs/en_US/topic-rewrite.md`; the hook receives the already-stripped
//!   filter — `Session::_subscribe` -> `hook.client_subscribe(&sub)`).
//!
//! Run:
//!
//! ```text
//! mqtt_harness --binary target/release/rmqttd --workspace . \
//!   --suites functional_v311 -t plugin_topic_rewrite
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::anyhow;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v311::MqttV311Client;
use crate::tests::fixture_config;

/// MQTT listener of `configs/plugin-topic-rewrite/rmqtt.toml`.
const BROKER_ADDR: &str = "127.0.0.1:1938";

fn fixture() -> Option<PathBuf> {
    Some(fixture_config("plugin-topic-rewrite"))
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

/// `action = "publish"`: the topic is rewritten before routing.
pub struct PluginTopicRewritePublishTest;

impl TestCase for PluginTopicRewritePublishTest {
    fn name(&self) -> &str {
        "plugin_topic_rewrite_publish_action"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            let mut rewritten = connect("rw-pub-out").await?;
            let mut original = connect("rw-pub-orig").await?;
            rewritten.subscribe("rw/out/#", QoS::AtLeastOnce).await?;
            original.subscribe("rw/pub/#", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(300)).await;

            let publisher = connect("rw-pub-src").await?;
            publisher.publish("rw/pub/a/b", b"rewritten", QoS::AtLeastOnce, false).await?;

            let msg = rewritten.recv_message_timeout(Duration::from_secs(5)).await.ok_or_else(|| {
                anyhow!(
                    "no delivery on `rw/out/a/b`: the publish rule \
                     (`rw/pub/# -> rw/out/$1`) did not rewrite `rw/pub/a/b`"
                )
            })?;
            if &*msg.topic != "rw/out/a/b" {
                return Err(anyhow!("delivered on `{}`, expected `rw/out/a/b`", msg.topic));
            }
            if msg.payload.as_ref() != b"rewritten" {
                return Err(anyhow!("payload {:?}", String::from_utf8_lossy(&msg.payload)));
            }

            // A rewrite must not leave the original topic routed as well.
            if let Some(msg) = original.recv_message_timeout(Duration::from_secs(2)).await {
                return Err(anyhow!(
                    "the message was ALSO routed on the original topic `{}`; the rewrite must \
                     replace the topic, not duplicate the publish",
                    msg.topic
                ));
            }

            rewritten.disconnect().await?;
            original.disconnect().await?;
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

/// `action = "subscribe"`: the filter is rewritten, including behind `$share/`.
pub struct PluginTopicRewriteSubscribeTest;

impl TestCase for PluginTopicRewriteSubscribeTest {
    fn name(&self) -> &str {
        "plugin_topic_rewrite_subscribe_action"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            // Plain filter rewrite.
            let mut sub = connect("rw-sub-plain").await?;
            sub.subscribe("rw/sub/old/x", QoS::AtLeastOnce).await?;
            // Shared form: only the filter behind `$share/g/` is rewritten, the
            // group is preserved (so the delivery must still be *shared*).
            let mut shared = connect("rw-sub-shared").await?;
            shared.subscribe("$share/g/rw/sub/old/y", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(300)).await;

            let publisher = connect("rw-sub-pub").await?;
            publisher.publish("rw/sub/new/x", b"plain", QoS::AtLeastOnce, false).await?;
            publisher.publish("rw/sub/new/y", b"shared", QoS::AtLeastOnce, false).await?;

            let plain = sub.recv_message_timeout(Duration::from_secs(5)).await.ok_or_else(|| {
                anyhow!(
                    "no delivery on `rw/sub/new/x` for a subscriber of `rw/sub/old/x`: the \
                     subscribe rule (`rw/sub/old/# -> rw/sub/new/#`) did not rewrite the filter"
                )
            })?;
            if plain.payload.as_ref() != b"plain" {
                return Err(anyhow!(
                    "plain subscriber payload {:?}",
                    String::from_utf8_lossy(&plain.payload)
                ));
            }

            let shared_msg = shared.recv_message_timeout(Duration::from_secs(5)).await.ok_or_else(|| {
                anyhow!(
                    "no delivery on `rw/sub/new/y` for a subscriber of `$share/g/rw/sub/old/y`: \
                     the rewrite must apply to the filter behind the `$share/<group>/` prefix and \
                     keep the group"
                )
            })?;
            if shared_msg.payload.as_ref() != b"shared" {
                return Err(anyhow!(
                    "shared subscriber payload {:?}",
                    String::from_utf8_lossy(&shared_msg.payload)
                ));
            }

            // The old topic must not be routed to the rewritten subscription.
            publisher.publish("rw/sub/old/x", b"stale", QoS::AtLeastOnce, false).await?;
            if let Some(msg) = sub.recv_message_timeout(Duration::from_secs(2)).await {
                return Err(anyhow!(
                    "a publish on the pre-rewrite topic `rw/sub/old/x` reached the subscriber as \
                     `{}`; the subscription must have moved to `rw/sub/new/#`",
                    msg.topic
                ));
            }

            sub.disconnect().await?;
            shared.disconnect().await?;
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
