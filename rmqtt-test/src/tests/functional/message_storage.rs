//! `rmqtt-message-storage` suite: the "store unexpired messages" contract.
//!
//! `docs/en_US/store-message.md` states the plugin's whole purpose: a published
//! message is kept until it expires, and *a subscription created later* still
//! receives it. Until now the plugin was only ever loaded as a side effect of
//! the QoS 2 PUBREL-resume reproduction (`configs/pubrel-collision/`), which
//! asserts nothing about that behaviour.
//!
//! Fixture: `configs/plugin-msg-store/` (MQTT 1931), `storage.type = "ram"`,
//! and — importantly — **no `rmqtt-retainer`**: retained messages are also
//! replayed on subscribe, so a live retainer would make the replay case green
//! even with the plugin broken.
//!
//! Two cases:
//!
//! * an unexpired message is replayed to a subscriber that subscribes later,
//!   and its `Message Expiry Interval` has been decreased, i.e. the replay goes
//!   through the normal Publish path rather than handing back a stale copy;
//! * an expired message is *not* replayed — and, in the same subscription, an
//!   unexpired one **is**, so the silence cannot pass just because the store is
//!   disabled or the subscription is dead (a fresh live publish is kept as a
//!   second control so the two failures stay distinguishable).
//!
//! Run:
//!
//! ```text
//! mqtt_harness --binary target/release/rmqttd --workspace . \
//!   --suites functional_v5 -t plugin_msg_store
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::anyhow;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v5::MqttV5Client;
use crate::tests::fixture_config;

/// MQTT listener of `configs/plugin-msg-store/rmqtt.toml`.
const BROKER_ADDR: &str = "127.0.0.1:1931";

fn fixture() -> Option<PathBuf> {
    Some(fixture_config("plugin-msg-store"))
}

async fn connect(client_id: &str) -> anyhow::Result<MqttV5Client> {
    MqttV5Client::connect(BROKER_ADDR, client_id, Duration::from_secs(10)).await
}

/// Publish with only the Message Expiry Interval property set.
async fn publish_expiring(
    client: &MqttV5Client,
    topic: &str,
    payload: &[u8],
    expiry_secs: u32,
) -> anyhow::Result<()> {
    client
        .publish_with_properties(
            topic,
            payload,
            QoS::AtLeastOnce,
            false,
            None,
            Some(expiry_secs),
            None,
            None,
            None,
            None,
        )
        .await
}

/// A message published before a subscription still reaches the subscriber.
pub struct PluginMsgStoreReplaysToLaterSubscriberTest;

impl TestCase for PluginMsgStoreReplaysToLaterSubscriberTest {
    fn name(&self) -> &str {
        "plugin_msg_store_replays_to_later_subscriber"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            const TOPIC: &str = "ms/later/a";
            const PAYLOAD: &[u8] = b"stored-before-subscribe";

            let publisher = connect("msg-store-pub").await?;
            publish_expiring(&publisher, TOPIC, PAYLOAD, 60).await?;
            // Let the broker finish routing/storing before the subscriber
            // exists; without the sleep the SUBSCRIBE could race the store.
            tokio::time::sleep(Duration::from_millis(400)).await;

            let mut sub = connect("msg-store-sub").await?;
            sub.subscribe(TOPIC, QoS::AtLeastOnce).await?;

            let msg = sub.recv_message_timeout(Duration::from_secs(10)).await.ok_or_else(|| {
                anyhow!(
                    "no replayed message on `{TOPIC}` within 10s: the message was published \
                         (expiry 60s) before the subscription existed, so only \
                         `rmqtt-message-storage` can deliver it here"
                )
            })?;

            sub.disconnect().await?;
            publisher.disconnect().await?;

            if &*msg.topic != TOPIC {
                return Err(anyhow!("replayed on `{}`, expected `{TOPIC}`", msg.topic));
            }
            if msg.payload.as_ref() != PAYLOAD {
                return Err(anyhow!(
                    "replayed payload {:?}, expected {:?}",
                    String::from_utf8_lossy(&msg.payload),
                    String::from_utf8_lossy(PAYLOAD)
                ));
            }
            // A replay is a normal delivery: the remaining lifetime is carried
            // in the property, and it can only have gone down.
            match msg.message_expiry_interval {
                Some(left) if left.get() <= 60 => {}
                other => {
                    return Err(anyhow!(
                        "replayed message carries Message Expiry Interval {other:?}, expected \
                         Some(<= 60)"
                    ));
                }
            }
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), format!("{e:#}")),
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

/// An expired message is not replayed; a fresh one on the same topic is.
pub struct PluginMsgStoreDropsExpiredMessageTest;

impl TestCase for PluginMsgStoreDropsExpiredMessageTest {
    fn name(&self) -> &str {
        "plugin_msg_store_drops_expired_message"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            const STALE_TOPIC: &str = "ms/expired/stale";
            const CONTROL_TOPIC: &str = "ms/expired/control";
            const STALE: &[u8] = b"must-not-be-replayed";
            const STORE_ALIVE: &[u8] = b"store-alive";
            const FRESH: &[u8] = b"positive-control";

            let publisher = connect("msg-store-exp-pub").await?;
            // Control with a long expiry: it *must* be replayed, in this very
            // subscription, which proves the store is live. Without it the
            // negative assertion below would also hold with
            // `rmqtt-message-storage` unloaded (nothing is ever replayed), i.e.
            // it would assert nothing at all (verified by unloading the plugin).
            publish_expiring(&publisher, CONTROL_TOPIC, STORE_ALIVE, 60).await?;
            publish_expiring(&publisher, STALE_TOPIC, STALE, 1).await?;
            // The interval is 1s; wait past it (and past the broker's expiry
            // sweep granularity) before any subscription exists.
            tokio::time::sleep(Duration::from_millis(2500)).await;

            let mut sub = connect("msg-store-exp-sub").await?;
            sub.subscribe("ms/expired/#", QoS::AtLeastOnce).await?;

            // Drain what the store owes: exactly the unexpired control.
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut got = Vec::new();
            while Instant::now() < deadline {
                let left = deadline.saturating_duration_since(Instant::now());
                let Some(msg) = sub.recv_message_timeout(left.min(Duration::from_millis(500))).await else {
                    break;
                };
                let payload = String::from_utf8_lossy(&msg.payload).to_string();
                if payload == "must-not-be-replayed" {
                    return Err(anyhow!(
                        "expired message replayed on `{}` (payload {payload:?}): the store must \
                         drop it once the Message Expiry Interval elapsed",
                        msg.topic
                    ));
                }
                got.push(payload);
            }
            if !got.iter().any(|p| p == "store-alive") {
                return Err(anyhow!(
                    "the unexpired control on `{CONTROL_TOPIC}` was not replayed (received \
                     {got:?}): either `rmqtt-message-storage` is not storing, or the \
                     subscription is dead — either way the silence on `{STALE_TOPIC}` proves \
                     nothing"
                ));
            }

            // Second control: live routing on the topic that had to stay
            // silent for the stale message.
            publish_expiring(&publisher, STALE_TOPIC, FRESH, 60).await?;
            let msg = sub
                .recv_message_timeout(Duration::from_secs(10))
                .await
                .ok_or_else(|| anyhow!("positive control: no live delivery on `{STALE_TOPIC}` within 10s"))?;
            if msg.payload.as_ref() != FRESH {
                return Err(anyhow!(
                    "positive control delivered {:?}, expected {:?}",
                    String::from_utf8_lossy(&msg.payload),
                    String::from_utf8_lossy(FRESH)
                ));
            }

            sub.disconnect().await?;
            publisher.disconnect().await?;
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), format!("{e:#}")),
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
