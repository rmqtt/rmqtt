//! Wildcard subscription tests (using v311 client)

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Test single-level wildcard (+)
pub struct WildcardPlusTest;

impl TestCase for WildcardPlusTest {
    fn name(&self) -> &str {
        "wildcard_plus"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "wc-plus-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "wc-plus-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            let sub_topic = "test/wildcard/+/message";
            subscriber.subscribe(sub_topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Should match
            publisher.publish("test/wildcard/foo/message", b"match1", QoS::AtLeastOnce, false).await?;
            publisher.publish("test/wildcard/bar/message", b"match2", QoS::AtLeastOnce, false).await?;

            // Should NOT match
            publisher.publish("test/wildcard/foo/bar/message", b"no_match", QoS::AtLeastOnce, false).await?;

            let msg1 = subscriber.recv_message_timeout(Duration::from_secs(3)).await;
            let msg2 = subscriber.recv_message_timeout(Duration::from_secs(3)).await;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            if msg1.is_some() && msg2.is_some() {
                Ok(())
            } else {
                Err(anyhow::anyhow!("wildcard + did not match expected messages"))
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Test multi-level wildcard (#)
pub struct WildcardHashTest;

impl TestCase for WildcardHashTest {
    fn name(&self) -> &str {
        "wildcard_hash"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "wc-hash-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "wc-hash-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            let sub_topic = "test/wildcard/#";
            subscriber.subscribe(sub_topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // All of these should match
            publisher.publish("test/wildcard/a", b"match1", QoS::AtLeastOnce, false).await?;
            publisher.publish("test/wildcard/a/b/c", b"match2", QoS::AtLeastOnce, false).await?;

            let msg1 = subscriber.recv_message_timeout(Duration::from_secs(3)).await;
            let msg2 = subscriber.recv_message_timeout(Duration::from_secs(3)).await;

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            if msg1.is_some() && msg2.is_some() {
                Ok(())
            } else {
                Err(anyhow::anyhow!("wildcard # did not match expected messages"))
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Positive: topic matching is case-sensitive — `A` and `a` are distinct.
pub struct WildcardV311CaseSensitiveTest;

impl TestCase for WildcardV311CaseSensitiveTest {
    fn name(&self) -> &str {
        "wildcard_v311_case_sensitive"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &format!("v311-case-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &format!("v311-case-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = format!("test/v311/CaseSensitive/{uid}");
            subscriber.subscribe(&topic, QoS::AtMostOnce).await?;
            tokio::time::sleep(Duration::from_millis(1000)).await;

            // Publish to a differently-cased topic; must NOT be delivered
            publisher.publish(&topic.to_lowercase(), b"wrong-case", QoS::AtMostOnce, false).await?;
            let msg = subscriber.recv_message_timeout(Duration::from_millis(800)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            if msg.is_some() {
                Err(anyhow::anyhow!("case-insensitive match occurred; topics must be case-sensitive"))
            } else {
                Ok(())
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Positive: a leading `/` creates a distinct topic (e.g. `/a` != `a`).
pub struct WildcardV311LeadingSlashTest;

impl TestCase for WildcardV311LeadingSlashTest {
    fn name(&self) -> &str {
        "wildcard_v311_leading_slash"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &format!("v311-slash-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &format!("v311-slash-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;

            let slashed = format!("/test/v311/slash/{uid}");
            subscriber.subscribe(&slashed, QoS::AtMostOnce).await?;
            tokio::time::sleep(Duration::from_millis(1000)).await;

            // Publish to the non-slashed variant; must NOT be delivered
            publisher.publish(slashed.trim_start_matches('/'), b"no-slash", QoS::AtMostOnce, false).await?;
            // Publish to the slashed variant; must be delivered
            publisher.publish(&slashed, b"with-slash", QoS::AtMostOnce, false).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            let extra = subscriber.recv_message_timeout(Duration::from_millis(500)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"with-slash" => {
                    if extra.is_some() {
                        Err(anyhow::anyhow!("leading-slash topic matched without the slash"))
                    } else {
                        Ok(())
                    }
                }
                Some(m) => Err(anyhow::anyhow!("unexpected payload: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("leading-slash topic not delivered")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Negative: a topic filter with `#` not in the final position is invalid
/// (spec 4.7.1); the broker must reject the SUBSCRIBE (SUBACK failure) or
/// close the connection.
pub struct WildcardV311HashNotLastTest;

impl TestCase for WildcardV311HashNotLastTest {
    fn name(&self) -> &str {
        "wildcard_v311_hash_not_last"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let mut client = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &format!("v311-hashpos-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;

            // "sport/#/x" — '#' is not the last level → invalid filter
            let res = client.subscribe("sport/#/x", QoS::AtMostOnce).await;
            let still_connected = client.is_connected();
            let _ = client.disconnect().await;

            // The broker must not accept the invalid filter as a valid
            // subscription. Accepted outcomes: SUBACK with failure status,
            // subscribe error, or connection closed.
            let rejected = match res {
                Err(_) => true,
                Ok(ack) => {
                    ack.status.iter().any(|s| matches!(s, rmqtt_codec::v3::SubscribeReturnCode::Failure))
                }
            };

            if !rejected && still_connected {
                Err(anyhow::anyhow!("broker accepted an invalid `#` position filter (4.7.1)"))
            } else {
                Ok(())
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Positive: topic matching is case-sensitive — `A` and `a` are distinct (v5).
pub struct WildcardV5CaseSensitiveTest;

impl TestCase for WildcardV5CaseSensitiveTest {
    fn name(&self) -> &str {
        "wildcard_v5_case_sensitive"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                &format!("v5-case-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                &format!("v5-case-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = format!("test/v5/CaseSensitive/{uid}");
            subscriber.subscribe(&topic, QoS::AtMostOnce).await?;
            tokio::time::sleep(Duration::from_millis(1000)).await;

            // Publish to a differently-cased topic; must NOT be delivered
            publisher.publish(&topic.to_lowercase(), b"wrong-case", QoS::AtMostOnce, false).await?;
            let msg = subscriber.recv_message_timeout(Duration::from_millis(800)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            if msg.is_some() {
                Err(anyhow::anyhow!("case-insensitive match occurred; topics must be case-sensitive"))
            } else {
                Ok(())
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Positive: a leading `/` creates a distinct topic (e.g. `/a` != `a`) (v5).
pub struct WildcardV5LeadingSlashTest;

impl TestCase for WildcardV5LeadingSlashTest {
    fn name(&self) -> &str {
        "wildcard_v5_leading_slash"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                &format!("v5-slash-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                &format!("v5-slash-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;

            let slashed = format!("/test/v5/slash/{uid}");
            subscriber.subscribe(&slashed, QoS::AtMostOnce).await?;
            tokio::time::sleep(Duration::from_millis(1000)).await;

            // Publish to the non-slashed variant; must NOT be delivered
            publisher.publish(slashed.trim_start_matches('/'), b"no-slash", QoS::AtMostOnce, false).await?;
            // Publish to the slashed variant; must be delivered
            publisher.publish(&slashed, b"with-slash", QoS::AtMostOnce, false).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            let extra = subscriber.recv_message_timeout(Duration::from_millis(500)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"with-slash" => {
                    if extra.is_some() {
                        Err(anyhow::anyhow!("leading-slash topic matched without the slash"))
                    } else {
                        Ok(())
                    }
                }
                Some(m) => Err(anyhow::anyhow!("unexpected payload: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("leading-slash topic not delivered")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

// ---------------------------------------------------------------------------
// P1 conformance gap fill (G15 empty levels / G16 overlapping wildcards)
// ---------------------------------------------------------------------------

/// Positive: topic matching treats empty levels correctly — `a//b` has an
/// empty middle level, a trailing slash adds a final empty level, and `+`
/// can match an empty level. [MQTT-4.7.1-2/3]
pub struct WildcardEmptyLevelsV311Test;

impl TestCase for WildcardEmptyLevelsV311Test {
    fn name(&self) -> &str {
        "wildcard_empty_levels_v311"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &format!("v311-emptylvl-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &format!("v311-emptylvl-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;

            let base = format!("test/v311/emptylvl/{uid}");
            // + must match the empty middle level of "<base>//leaf"
            let plus = format!("{base}/+/leaf");
            // # must span empty levels and a trailing slash
            let hash = format!("{base}/#");
            subscriber.subscribe(&plus, QoS::AtMostOnce).await?;
            subscriber.subscribe(&hash, QoS::AtMostOnce).await?;
            tokio::time::sleep(Duration::from_millis(1000)).await;

            // 1) "<base>//leaf": matches BOTH + (empty level) and #
            publisher.publish(&format!("{base}//leaf"), b"empty-middle", QoS::AtMostOnce, false).await?;
            // 2) "<base>/x/leaf": matches + and #  → also 2 deliveries
            publisher.publish(&format!("{base}/x/leaf"), b"normal", QoS::AtMostOnce, false).await?;
            // 3) "<base>/": trailing slash — matches # (parent + empty level)
            publisher.publish(&format!("{base}/"), b"trailing", QoS::AtMostOnce, false).await?;

            let mut seen: Vec<String> = Vec::new();
            for _ in 0..5 {
                if let Some(m) = subscriber.recv_message_timeout(Duration::from_secs(3)).await {
                    seen.push(String::from_utf8_lossy(&m.payload).into_owned());
                }
            }
            let _ = publisher.disconnect().await;
            let _ = subscriber.disconnect().await;

            let mut counts = std::collections::HashMap::new();
            for p in &seen {
                *counts.entry(p.clone()).or_insert(0usize) += 1;
            }
            // each published message must have been delivered (once per
            // matching subscription): empty-middle ×2, normal ×2, trailing ×1
            let ok = counts.get("empty-middle").copied().unwrap_or(0) == 2
                && counts.get("normal").copied().unwrap_or(0) == 2
                && counts.get("trailing").copied().unwrap_or(0) == 1;
            if ok {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "empty-level matching mismatch: {:?} (expected empty-middle×2, normal×2, trailing×1)",
                    counts
                ))
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// Positive: when a topic matches multiple overlapping subscriptions of the
/// same client, each subscription receives its own copy of the message.
/// [MQTT-4.7.0-1] (mirrors `wildcard_v3_overlap` for v3.1.1)
pub struct WildcardV311OverlapTest;

impl TestCase for WildcardV311OverlapTest {
    fn name(&self) -> &str {
        "wildcard_v311_overlap"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &format!("v311-ov-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &format!("v311-ov-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;

            let concrete = format!("test/v311/overlap/{uid}/data");
            let wildcard = format!("test/v311/overlap/{uid}/#");
            subscriber.subscribe(&concrete, QoS::AtMostOnce).await?;
            subscriber.subscribe(&wildcard, QoS::AtMostOnce).await?;
            tokio::time::sleep(Duration::from_millis(1000)).await;

            publisher.publish(&concrete, b"dup", QoS::AtMostOnce, false).await?;

            let m1 = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            let m2 = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            let _ = publisher.disconnect().await;
            let _ = subscriber.disconnect().await;

            // Both subscriptions must have been matched → two deliveries
            match (m1, m2) {
                (Some(a), Some(b)) if a.payload.as_ref() == b"dup" && b.payload.as_ref() == b"dup" => Ok(()),
                (Some(_), None) | (None, Some(_)) => {
                    Err(anyhow::anyhow!("overlapping subscriptions delivered only one copy"))
                }
                _ => Err(anyhow::anyhow!("overlapping subscriptions did not deliver both copies")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}
