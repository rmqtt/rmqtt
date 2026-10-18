//! Listener quota + limited-subscription suite (`rmqtt/src/session.rs`).
//!
//! These knobs were previously written into fixtures only as *baseline* values
//! (`max_subscriptions = 0`, `max_topic_levels = 0`, `max_qos_allowed = 2`), so
//! nothing ever proved the broker enforces them. Fixture
//! `configs/plugin-limits/` (MQTT 1932) turns them down low enough to observe:
//!
//! ```text
//! max_subscriptions  = 2
//! max_topic_levels   = 3
//! max_qos_allowed    = 1
//! limit_subscription = true
//! ```
//!
//! Case-by-case contract (`Session::_subscribe`):
//!
//! | knob | observable |
//! |---|---|
//! | `max_subscriptions` | the 3rd filter of one session is refused in SUBACK and never matches |
//! | `max_topic_levels` | a 4-level filter is refused in SUBACK and never matches |
//! | `max_qos_allowed` | a QoS 2 request is *granted as QoS 1* (SUBACK 0x01), and delivery is QoS 1 |
//! | `$limit/<n>/`, `$exclusive/` | the n-th subscriber of the same filter is refused |
//!
//! Note on the refusal code: the SUBSCRIBE path reports every per-filter error
//! as `UnspecifiedError` (0x80) — see `Session::subscribes_v5` — even though
//! `MqttError::TooManySubscriptions`/`SubscribeLimited` map to `QuotaExceeded`
//! on the *disconnect* path. The cases therefore assert "not granted" plus the
//! absence of the routing effect, which is the behaviour that matters and stays
//! valid if the code is later tightened to 0x97.
//!
//! Run:
//!
//! ```text
//! mqtt_harness --binary target/release/rmqttd --workspace . \
//!   --suites functional_v5 -t plugin_limit
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use rmqtt_codec::v5::SubscribeAckReason;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v5::MqttV5Client;
use crate::tests::fixture_config;

/// MQTT listener of `configs/plugin-limits/rmqtt.toml`.
const BROKER_ADDR: &str = "127.0.0.1:1932";

fn fixture() -> Option<PathBuf> {
    Some(fixture_config("plugin-limits"))
}

async fn connect(client_id: &str) -> anyhow::Result<MqttV5Client> {
    MqttV5Client::connect(BROKER_ADDR, client_id, Duration::from_secs(10)).await
}

fn is_granted(reason: &SubscribeAckReason) -> bool {
    matches!(
        reason,
        SubscribeAckReason::GrantedQos0 | SubscribeAckReason::GrantedQos1 | SubscribeAckReason::GrantedQos2
    )
}

async fn subscribe(client: &mut MqttV5Client, filter: &str, qos: QoS) -> anyhow::Result<SubscribeAckReason> {
    let ack = client.subscribe(filter, qos).await?;
    ack.status.first().copied().ok_or_else(|| anyhow!("SUBACK for `{filter}` carried no reason code"))
}

/// `max_subscriptions`: the third filter of one session is refused.
pub struct PluginLimitMaxSubscriptionsTest;

impl TestCase for PluginLimitMaxSubscriptionsTest {
    fn name(&self) -> &str {
        "plugin_limit_max_subscriptions"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            let mut sub = connect("limit-subs").await?;
            let publisher = connect("limit-subs-pub").await?;

            for filter in ["lim/a", "lim/b"] {
                let reason = subscribe(&mut sub, filter, QoS::AtLeastOnce).await?;
                if !is_granted(&reason) {
                    return Err(anyhow!(
                        "`{filter}` refused ({reason:?}) although only 1 filter was registered \
                         and `max_subscriptions = 2`"
                    ));
                }
            }

            let reason = subscribe(&mut sub, "lim/c", QoS::AtLeastOnce).await?;
            if is_granted(&reason) {
                return Err(anyhow!(
                    "the 3rd filter was granted as {reason:?} although `max_subscriptions = 2`"
                ));
            }

            // The refused filter must not be routed to either: publishing to it
            // has to stay silent, while a granted filter still delivers.
            publisher.publish("lim/c", b"refused-filter", QoS::AtLeastOnce, false).await?;
            if let Some(msg) = sub.recv_message_timeout(Duration::from_secs(2)).await {
                return Err(anyhow!(
                    "a message arrived on `{}` although the SUBSCRIBE for `lim/c` was refused \
                     ({reason:?})",
                    msg.topic
                ));
            }

            publisher.publish("lim/a", b"granted-filter", QoS::AtLeastOnce, false).await?;
            let msg = sub
                .recv_message_timeout(Duration::from_secs(5))
                .await
                .ok_or_else(|| anyhow!("positive control: no delivery on `lim/a` within 5s"))?;
            if msg.payload.as_ref() != b"granted-filter" {
                return Err(anyhow!("positive control payload {:?}", String::from_utf8_lossy(&msg.payload)));
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

/// `max_topic_levels`: a 4-level filter is refused, a 3-level one is granted.
pub struct PluginLimitMaxTopicLevelsTest;

impl TestCase for PluginLimitMaxTopicLevelsTest {
    fn name(&self) -> &str {
        "plugin_limit_max_topic_levels"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            let mut sub = connect("limit-levels").await?;
            let publisher = connect("limit-levels-pub").await?;

            let deep = "lvl/a/b/c";
            let reason = subscribe(&mut sub, deep, QoS::AtLeastOnce).await?;
            if is_granted(&reason) {
                return Err(anyhow!(
                    "`{deep}` (4 levels) was granted as {reason:?} although `max_topic_levels = 3`"
                ));
            }

            publisher.publish(deep, b"too-deep", QoS::AtLeastOnce, false).await?;
            if let Some(msg) = sub.recv_message_timeout(Duration::from_secs(2)).await {
                return Err(anyhow!(
                    "a message arrived on `{}` although the SUBSCRIBE was refused ({reason:?})",
                    msg.topic
                ));
            }

            // Positive control: exactly at the limit.
            let ok = "lvl/a/b";
            let reason = subscribe(&mut sub, ok, QoS::AtLeastOnce).await?;
            if !is_granted(&reason) {
                return Err(anyhow!("`{ok}` (3 levels) refused as {reason:?}, the limit is 3"));
            }
            publisher.publish(ok, b"at-limit", QoS::AtLeastOnce, false).await?;
            let msg = sub
                .recv_message_timeout(Duration::from_secs(5))
                .await
                .ok_or_else(|| anyhow!("positive control: no delivery on `{ok}` within 5s"))?;
            if msg.payload.as_ref() != b"at-limit" {
                return Err(anyhow!("positive control payload {:?}", String::from_utf8_lossy(&msg.payload)));
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

/// `max_qos_allowed`: a QoS 2 request is granted as QoS 1 and delivered as QoS 1.
pub struct PluginLimitMaxQosAllowedTest;

impl TestCase for PluginLimitMaxQosAllowedTest {
    fn name(&self) -> &str {
        "plugin_limit_max_qos_allowed"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            let mut sub = connect("limit-qos").await?;
            let publisher = connect("limit-qos-pub").await?;

            let reason = subscribe(&mut sub, "qos/capped", QoS::ExactlyOnce).await?;
            if reason != SubscribeAckReason::GrantedQos1 {
                return Err(anyhow!(
                    "SUBACK for a QoS 2 request was {reason:?}, expected GrantedQos1 with \
                     `max_qos_allowed = 1`"
                ));
            }

            // Delivered QoS is `min(publish, granted)` == 1.
            publisher.publish("qos/capped", b"capped", QoS::ExactlyOnce, false).await?;
            let msg = sub
                .recv_message_timeout(Duration::from_secs(5))
                .await
                .ok_or_else(|| anyhow!("no delivery on `qos/capped` within 5s"))?;
            if msg.qos != QoS::AtLeastOnce {
                return Err(anyhow!(
                    "delivery QoS is {:?}, expected {:?} (subscription was granted as QoS 1)",
                    msg.qos,
                    QoS::AtLeastOnce
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

/// `$limit/<n>/<filter>` and `$exclusive/<filter>` (rmqtt extensions).
///
/// `$exclusive/x` is sugar for `$limit/1/x` (`types::parse_topic_filter`), and
/// both compare the *stripped* filter, so two clients that write the prefix
/// differently still compete for the same slot.
pub struct PluginLimitSubscriptionFormsTest;

impl TestCase for PluginLimitSubscriptionFormsTest {
    fn name(&self) -> &str {
        "plugin_limit_subscription_forms"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            let mut first = connect("limit-form-1").await?;
            let mut second = connect("limit-form-2").await?;
            let publisher = connect("limit-form-pub").await?;

            // `$limit/1/lk/x` -> filter `lk/x`, one subscriber.
            let reason = subscribe(&mut first, "$limit/1/lk/x", QoS::AtLeastOnce).await?;
            if !is_granted(&reason) {
                return Err(anyhow!("first `$limit/1/lk/x` subscriber refused as {reason:?}"));
            }
            let reason = subscribe(&mut second, "$limit/1/lk/x", QoS::AtLeastOnce).await?;
            if is_granted(&reason) {
                return Err(anyhow!(
                    "second `$limit/1/lk/x` subscriber granted as {reason:?}, the limit is 1"
                ));
            }

            // `$exclusive/lk/y` -> filter `lk/y`, one subscriber.
            let reason = subscribe(&mut first, "$exclusive/lk/y", QoS::AtLeastOnce).await?;
            if !is_granted(&reason) {
                return Err(anyhow!("first `$exclusive/lk/y` subscriber refused as {reason:?}"));
            }
            let reason = subscribe(&mut second, "$exclusive/lk/y", QoS::AtLeastOnce).await?;
            if is_granted(&reason) {
                return Err(anyhow!("second `$exclusive/lk/y` subscriber granted as {reason:?}"));
            }

            // One publish per filter: only `first` must receive, and it must
            // receive both (the prefixes are stripped, so the plain topic is
            // what gets published).
            publisher.publish("lk/x", b"limit-x", QoS::AtLeastOnce, false).await?;
            publisher.publish("lk/y", b"exclusive-y", QoS::AtLeastOnce, false).await?;

            let mut got = Vec::new();
            while got.len() < 2 {
                let Some(msg) = first.recv_message_timeout(Duration::from_secs(5)).await else {
                    break;
                };
                got.push(String::from_utf8_lossy(&msg.payload).to_string());
            }
            got.sort();
            if got != ["exclusive-y".to_string(), "limit-x".to_string()] {
                return Err(anyhow!("first subscriber received {got:?}, expected both payloads"));
            }

            if let Some(msg) = second.recv_message_timeout(Duration::from_millis(500)).await {
                return Err(anyhow!(
                    "the refused subscriber received `{}`, but both SUBSCRIBEs were refused",
                    msg.topic
                ));
            }

            first.disconnect().await?;
            second.disconnect().await?;
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
