//! MQTT v3.1 Topic wildcard functional tests
//!
//! Covers spec section 4.7 (Topic Names and Topic Filters):
//! - single-level wildcard `+` matches exactly one level
//! - multi-level wildcard `#` matches any number of levels (including 0)
//! - overlapping subscriptions (concrete + wildcard) receive multiple copies
//! - `$`-prefixed topics are not matched by wildcards starting with `$`
//! - topic names are case-sensitive
//! - leading `/` creates a distinct topic level

use std::time::{Duration, Instant};

use rmqtt_codec::v3::QoS;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Positive: `+` (single-level wildcard) matches exactly one level.
pub struct WildcardV3PlusTest;

impl TestCase for WildcardV3PlusTest {
    fn name(&self) -> &str {
        "wildcard_v3_plus"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let publisher = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-wc-plus-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-wc-plus-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;

            let filter = "test/v3/wildcard/+/level".to_string();
            subscriber.subscribe(&filter, QoS::AtMostOnce).await?;
            tokio::time::sleep(Duration::from_millis(1000)).await;

            // `+` matches exactly one level: "one" should match
            publisher.publish("test/v3/wildcard/one/level", b"match", QoS::AtMostOnce, false).await?;
            // "one/two" has two levels between: should NOT match `+`
            publisher.publish("test/v3/wildcard/one/two/level", b"nomatch", QoS::AtMostOnce, false).await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            let extra = subscriber.recv_message_timeout(Duration::from_millis(500)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"match" => {
                    if extra.is_some() {
                        Err(anyhow::anyhow!(
                            "single-level wildcard matched more than one level: {:?}",
                            extra.map(|e| e.payload)
                        ))
                    } else {
                        Ok(())
                    }
                }
                Some(m) => Err(anyhow::anyhow!("unexpected payload: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("`+` wildcard did not match a single-level topic")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Positive: `#` (multi-level wildcard) matches any number of levels.
pub struct WildcardV3HashTest;

impl TestCase for WildcardV3HashTest {
    fn name(&self) -> &str {
        "wildcard_v3_hash"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let publisher = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-wc-hash-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-wc-hash-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;

            let filter = "test/v3/wildcard/#".to_string();
            subscriber.subscribe(&filter, QoS::AtMostOnce).await?;
            tokio::time::sleep(Duration::from_millis(1000)).await;

            // `#` matches zero or more levels
            publisher.publish("test/v3/wildcard/a", b"lvl1", QoS::AtMostOnce, false).await?;
            publisher.publish("test/v3/wildcard/a/b/c", b"lvl3", QoS::AtMostOnce, false).await?;
            publisher.publish("test/v3/wildcard", b"zero", QoS::AtMostOnce, false).await?;

            let m1 = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            let m2 = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            let m3 = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            let mut payloads = vec![];
            for m in [m1, m2, m3] {
                match m {
                    Some(m) => payloads.push(m.payload.to_vec()),
                    None => return Err(anyhow::anyhow!("`#` wildcard missed a message")),
                }
            }
            payloads.sort();
            let expected = vec![b"lvl1".to_vec(), b"lvl3".to_vec(), b"zero".to_vec()];
            let mut exp = expected.clone();
            exp.sort();
            if payloads == exp {
                Ok(())
            } else {
                Err(anyhow::anyhow!("`#` wildcard delivered wrong set: {:?}", payloads))
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Positive: overlapping subscriptions (concrete topic + wildcard) both
/// receive a copy of the message.
pub struct WildcardV3OverlapTest;

impl TestCase for WildcardV3OverlapTest {
    fn name(&self) -> &str {
        "wildcard_v3_overlap"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let publisher = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-ov-sub-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-ov-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;

            let concrete = format!("test/v3/overlap/{uid}/data");
            let wildcard = format!("test/v3/overlap/{uid}/#");
            subscriber.subscribe(&concrete, QoS::AtMostOnce).await?;
            subscriber.subscribe(&wildcard, QoS::AtMostOnce).await?;
            tokio::time::sleep(Duration::from_millis(1000)).await;

            publisher.publish(&concrete, b"dup", QoS::AtMostOnce, false).await?;

            let m1 = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            let m2 = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

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
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Positive: `$`-prefixed topics are NOT matched by wildcard filters that do
/// not also start with `$` (spec 4.7.2).
pub struct WildcardV3DollarTopicsTest;

impl TestCase for WildcardV3DollarTopicsTest {
    fn name(&self) -> &str {
        "wildcard_v3_dollar_topics"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let publisher = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-dollar-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-dollar-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;

            // `#` at root must not match `$SYS/...`
            subscriber.subscribe("#", QoS::AtMostOnce).await?;
            tokio::time::sleep(Duration::from_millis(1000)).await;

            // Drain any stale retained messages from previous tests — a `#`
            // subscription receives ALL matching retained messages on
            // subscribe, so messages left over by retain tests would otherwise
            // be misread as a $SYS match.
            while subscriber.recv_message_timeout(Duration::from_millis(100)).await.is_some() {}

            publisher.publish("$SYS/v3/test", b"sys", QoS::AtMostOnce, false).await?;
            publisher.publish("normal/topic", b"normal", QoS::AtMostOnce, false).await?;

            // Read messages until we find "normal"; assert none has a $SYS
            // (metadata) topic. Messages from other concurrent tests may also
            // arrive, so we filter rather than fail on the first one.
            let mut saw_normal = false;
            let mut saw_dollar = false;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while tokio::time::Instant::now() < deadline {
                match subscriber.recv_message_timeout(Duration::from_millis(200)).await {
                    Some(m) => {
                        if m.payload.as_ref() == b"normal" {
                            saw_normal = true;
                            break;
                        } else if m.topic.starts_with('$') {
                            saw_dollar = true;
                            break;
                        }
                        // ignore unrelated messages from other tests
                    }
                    None => break,
                }
            }

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            if saw_dollar {
                Err(anyhow::anyhow!("$-prefixed topic was matched by a root wildcard (4.7.2)"))
            } else if saw_normal {
                Ok(())
            } else {
                Err(anyhow::anyhow!("normal topic not delivered"))
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Negative: topic matching is case-sensitive — `A` and `a` are distinct.
pub struct WildcardV3CaseSensitiveTest;

impl TestCase for WildcardV3CaseSensitiveTest {
    fn name(&self) -> &str {
        "wildcard_v3_case_sensitive"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let publisher = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-case-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-case-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = format!("test/v3/CaseSensitive/{uid}");
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
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Positive: a leading `/` creates a distinct topic (e.g. `/a` != `a`).
pub struct WildcardV3LeadingSlashTest;

impl TestCase for WildcardV3LeadingSlashTest {
    fn name(&self) -> &str {
        "wildcard_v3_leading_slash"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let publisher = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-slash-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v3::MqttV3Client::connect(
                &ctx.config.broker_addr,
                &format!("v3-slash-sub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;

            // Subscribe to the slash-prefixed topic only
            let slashed = format!("/test/v3/slash/{uid}");
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
            Ok(()) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}
