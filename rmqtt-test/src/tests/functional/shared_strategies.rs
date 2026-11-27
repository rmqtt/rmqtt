//! Shared-subscription **strategy** suite (`rmqtt-shared-subscription`).
//!
//! The plugin implements seven selection strategies, but the only assertion in
//! the harness was `shared_sub_v311` / `shared_sub_v5`: "both subscribers got at
//! least one of the ten messages". Every strategy satisfies that, so the
//! strategy config was effectively untested — and the same weak shape cannot
//! detect a duplicate delivery either.
//!
//! Strategies observable on a **single node** get a case each (the strategy is a
//! load-time config value, hence one fixture per strategy):
//!
//! | case | fixture | promise |
//! |---|---|---|
//! | round_robin (default) | harness default config | every message delivered exactly once |
//! | `sticky` | `plugin-shared-sticky` (1934) | one fixed subscriber per publisher, kept after a 3rd subscriber joins |
//! | `hash_clientid` | `plugin-shared-hash-clientid` (1935) | one publisher always lands on the same subscriber |
//! | `hash_topic` | `plugin-shared-hash-topic` (1936) | every message on one topic lands on the same subscriber |
//!
//! Deliberately **not** covered here (they need more than one node, so they
//! belong to the cluster suite):
//!
//! * `local` — "prefer the publisher's node" is unobservable with one node;
//! * `round_robin_per_group` — differs from `round_robin` only per node;
//! * `random` — on a 10-message sample it is indistinguishable from
//!   `round_robin`, so a case would assert nothing the round_robin case does
//!   not already assert.
//!
//! Run:
//!
//! ```text
//! mqtt_harness --binary target/release/rmqttd --workspace . \
//!   --suites functional_v311 -t plugin_shared
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::anyhow;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v311::MqttV311Client;
use crate::tests::fixture_config;

const DEFAULT_ADDR: &str = "127.0.0.1:1883";
const STICKY_ADDR: &str = "127.0.0.1:1934";
const HASH_CLIENTID_ADDR: &str = "127.0.0.1:1935";
const HASH_TOPIC_ADDR: &str = "127.0.0.1:1936";

async fn connect(addr: &str, client_id: &str) -> anyhow::Result<MqttV311Client> {
    MqttV311Client::connect_with_options(addr, client_id, Duration::from_secs(10), true, 60, None, None, None)
        .await
}

/// Drain everything a subscriber owes within `budget`, returning the payloads.
async fn drain(sub: &mut MqttV311Client, budget: Duration) -> Vec<String> {
    let deadline = Instant::now() + budget;
    let mut out = Vec::new();
    while Instant::now() < deadline {
        let left = deadline.saturating_duration_since(Instant::now());
        match sub.recv_message_timeout(left.min(Duration::from_millis(500))).await {
            Some(msg) => out.push(String::from_utf8_lossy(&msg.payload).to_string()),
            None => break,
        }
    }
    out
}

async fn publish_many(pubr: &MqttV311Client, topic: &str, prefix: &str, n: usize) -> anyhow::Result<()> {
    for i in 0..n {
        pubr.publish(topic, format!("{prefix}{i}").as_bytes(), QoS::AtLeastOnce, false).await?;
    }
    Ok(())
}

fn sorted(v: &[String]) -> Vec<String> {
    let mut v = v.to_vec();
    v.sort();
    v
}

/// `round_robin` (the default): exactly-once distribution across the group.
///
/// Runs on the harness default config, which starts
/// `rmqtt-shared-subscription` with its default strategy, so no fixture switch
/// is needed. The invariant asserted here — 12 distinct payloads, each
/// delivered exactly once — is the one every strategy must satisfy, and the one
/// the pre-existing `shared_sub_*` cases cannot see.
pub struct PluginSharedRoundRobinExactlyOnceTest;

impl TestCase for PluginSharedRoundRobinExactlyOnceTest {
    fn name(&self) -> &str {
        "plugin_shared_round_robin_exactly_once"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let addr = ctx.config.broker_addr.clone();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            const TOPIC: &str = "sh/rr/x";
            const FILTER: &str = "$share/rr/sh/rr/x";
            const COUNT: usize = 12;

            let mut s1 = connect(&addr, "sh-rr-s1").await?;
            let mut s2 = connect(&addr, "sh-rr-s2").await?;
            let mut s3 = connect(&addr, "sh-rr-s3").await?;
            for s in [&mut s1, &mut s2, &mut s3] {
                s.subscribe(FILTER, QoS::AtLeastOnce).await?;
            }
            tokio::time::sleep(Duration::from_millis(400)).await;

            let pubr = connect(&addr, "sh-rr-pub").await?;
            publish_many(&pubr, TOPIC, "rr-", COUNT).await?;
            tokio::time::sleep(Duration::from_millis(300)).await;

            let mut all = Vec::new();
            for s in [&mut s1, &mut s2, &mut s3] {
                all.extend(drain(s, Duration::from_secs(3)).await);
            }

            for s in [s1, s2, s3] {
                s.disconnect().await?;
            }
            pubr.disconnect().await?;

            // Compare as multisets: `sorted()` is lexicographic, so the
            // expected side has to go through it as well (otherwise `rr-10`
            // sorts before `rr-2` and a perfect run fails).
            let expected = sorted(&(0..COUNT).map(|i| format!("rr-{i}")).collect::<Vec<_>>());
            let got = sorted(&all);
            if got != expected {
                return Err(anyhow!(
                    "the group received {got:?}; expected each of {expected:?} exactly once \
                     ({} deliveries for {COUNT} publishes)",
                    all.len()
                ));
            }
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), format!("{e:#}")),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(90)
    }
}

/// `sticky`: a publisher keeps its subscriber, even after a 3rd member joins.
pub struct PluginSharedStickyAffinityTest;

impl TestCase for PluginSharedStickyAffinityTest {
    fn name(&self) -> &str {
        "plugin_shared_sticky_keeps_publisher_affinity"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            const TOPIC: &str = "sh/sticky/x";
            const FILTER: &str = "$share/sticky/sh/sticky/x";
            const BATCH: usize = 6;

            let mut s1 = connect(STICKY_ADDR, "sh-sticky-s1").await?;
            let mut s2 = connect(STICKY_ADDR, "sh-sticky-s2").await?;
            s1.subscribe(FILTER, QoS::AtLeastOnce).await?;
            s2.subscribe(FILTER, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(400)).await;

            let pubr = connect(STICKY_ADDR, "sh-sticky-pub").await?;
            publish_many(&pubr, TOPIC, "a", BATCH).await?;
            tokio::time::sleep(Duration::from_millis(300)).await;

            let s1_first = drain(&mut s1, Duration::from_secs(2)).await;
            let s2_first = drain(&mut s2, Duration::from_secs(2)).await;

            let (owner_first, other_first) =
                if !s1_first.is_empty() { (&s1_first, &s2_first) } else { (&s2_first, &s1_first) };
            if owner_first.len() != BATCH || !other_first.is_empty() {
                return Err(anyhow!(
                    "sticky must send all {BATCH} messages of one publisher to ONE subscriber; \
                     got s1={s1_first:?} s2={s2_first:?}"
                ));
            }
            let owner_is_s1 = !s1_first.is_empty();

            // A third member joins: sticky bindings are per publisher and must
            // survive, unlike round_robin which redistributes.
            let mut s3 = connect(STICKY_ADDR, "sh-sticky-s3").await?;
            s3.subscribe(FILTER, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(400)).await;

            publish_many(&pubr, TOPIC, "b", BATCH).await?;
            tokio::time::sleep(Duration::from_millis(300)).await;

            let s1_second = drain(&mut s1, Duration::from_secs(2)).await;
            let s2_second = drain(&mut s2, Duration::from_secs(2)).await;
            let s3_second = drain(&mut s3, Duration::from_secs(2)).await;

            s1.disconnect().await?;
            s2.disconnect().await?;
            s3.disconnect().await?;
            pubr.disconnect().await?;

            let (owner_second, others_second) = if owner_is_s1 {
                (&s1_second, vec![&s2_second, &s3_second])
            } else {
                (&s2_second, vec![&s1_second, &s3_second])
            };
            if owner_second.len() != BATCH {
                return Err(anyhow!(
                    "after a 3rd member joined, the publisher's original subscriber received {} \
                     of {BATCH} messages (s1={s1_second:?} s2={s2_second:?} s3={s3_second:?}); \
                     `sticky` must keep the binding",
                    owner_second.len()
                ));
            }
            for got in others_second {
                if !got.is_empty() {
                    return Err(anyhow!(
                        "the binding moved after a 3rd member joined: {got:?} landed elsewhere \
                         (s1={s1_second:?} s2={s2_second:?} s3={s3_second:?})"
                    ));
                }
            }
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), format!("{e:#}")),
        }
    }

    fn broker_config(&self) -> Option<PathBuf> {
        Some(fixture_config("plugin-shared-sticky"))
    }

    fn broker_addr(&self) -> Option<&'static str> {
        Some(STICKY_ADDR)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(120)
    }
}

/// `hash_clientid`: one publisher always lands on the same subscriber.
pub struct PluginSharedHashClientIdTest;

impl TestCase for PluginSharedHashClientIdTest {
    fn name(&self) -> &str {
        "plugin_shared_hash_clientid_pins_publisher"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            const TOPIC: &str = "sh/hcid/x";
            const FILTER: &str = "$share/hcid/sh/hcid/x";
            const BATCH: usize = 6;

            let mut s1 = connect(HASH_CLIENTID_ADDR, "sh-hcid-s1").await?;
            let mut s2 = connect(HASH_CLIENTID_ADDR, "sh-hcid-s2").await?;
            s1.subscribe(FILTER, QoS::AtLeastOnce).await?;
            s2.subscribe(FILTER, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(400)).await;

            let p1 = connect(HASH_CLIENTID_ADDR, "sh-hcid-pub-1").await?;
            let p2 = connect(HASH_CLIENTID_ADDR, "sh-hcid-pub-2").await?;

            publish_many(&p1, TOPIC, "p1-", BATCH).await?;
            tokio::time::sleep(Duration::from_millis(300)).await;
            let s1_p1 = drain(&mut s1, Duration::from_secs(2)).await;
            let s2_p1 = drain(&mut s2, Duration::from_secs(2)).await;

            publish_many(&p2, TOPIC, "p2-", BATCH).await?;
            tokio::time::sleep(Duration::from_millis(300)).await;
            let s1_p2 = drain(&mut s1, Duration::from_secs(2)).await;
            let s2_p2 = drain(&mut s2, Duration::from_secs(2)).await;

            s1.disconnect().await?;
            s2.disconnect().await?;
            p1.disconnect().await?;
            p2.disconnect().await?;

            for (who, a, b) in [("publisher 1", &s1_p1, &s2_p1), ("publisher 2", &s1_p2, &s2_p2)] {
                let (owner, other) = if !a.is_empty() { (a, b) } else { (b, a) };
                if owner.len() != BATCH || !other.is_empty() {
                    return Err(anyhow!(
                        "hash_clientid must pin {who} to ONE subscriber for all {BATCH} messages; \
                         got s1={a:?} s2={b:?}"
                    ));
                }
            }
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), format!("{e:#}")),
        }
    }

    fn broker_config(&self) -> Option<PathBuf> {
        Some(fixture_config("plugin-shared-hash-clientid"))
    }

    fn broker_addr(&self) -> Option<&'static str> {
        Some(HASH_CLIENTID_ADDR)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(120)
    }
}

/// `hash_topic`: all messages on one topic land on the same subscriber.
pub struct PluginSharedHashTopicTest;

impl TestCase for PluginSharedHashTopicTest {
    fn name(&self) -> &str {
        "plugin_shared_hash_topic_pins_topic"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().expect("failed to create runtime");
        let result = rt.block_on(async {
            const FILTER: &str = "$share/ht/sh/ht/#";
            const BATCH: usize = 5;

            let mut s1 = connect(HASH_TOPIC_ADDR, "sh-ht-s1").await?;
            let mut s2 = connect(HASH_TOPIC_ADDR, "sh-ht-s2").await?;
            s1.subscribe(FILTER, QoS::AtLeastOnce).await?;
            s2.subscribe(FILTER, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(400)).await;

            let pubr = connect(HASH_TOPIC_ADDR, "sh-ht-pub").await?;
            // Two topics, disjoint payload prefixes, so each delivery can be
            // attributed to its topic after the fact.
            publish_many(&pubr, "sh/ht/one", "one-", BATCH).await?;
            publish_many(&pubr, "sh/ht/two", "two-", BATCH).await?;
            tokio::time::sleep(Duration::from_millis(300)).await;

            let s1_got = drain(&mut s1, Duration::from_secs(3)).await;
            let s2_got = drain(&mut s2, Duration::from_secs(3)).await;

            s1.disconnect().await?;
            s2.disconnect().await?;
            pubr.disconnect().await?;

            for prefix in ["one-", "two-"] {
                let on_s1 = s1_got.iter().filter(|p| p.starts_with(prefix)).count();
                let on_s2 = s2_got.iter().filter(|p| p.starts_with(prefix)).count();
                if on_s1 + on_s2 != BATCH {
                    return Err(anyhow!(
                        "`{prefix}*`: {} deliveries for {BATCH} publishes (s1={on_s1}, s2={on_s2}); \
                         s1={s1_got:?} s2={s2_got:?}",
                        on_s1 + on_s2
                    ));
                }
                if on_s1 != 0 && on_s2 != 0 {
                    return Err(anyhow!(
                        "`{prefix}*` was split across subscribers ({on_s1} on s1, {on_s2} on s2); \
                         `hash_topic` must pin a topic to one subscriber"
                    ));
                }
            }
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), format!("{e:#}")),
        }
    }

    fn broker_config(&self) -> Option<PathBuf> {
        Some(fixture_config("plugin-shared-hash-topic"))
    }

    fn broker_addr(&self) -> Option<&'static str> {
        Some(HASH_TOPIC_ADDR)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(120)
    }
}
