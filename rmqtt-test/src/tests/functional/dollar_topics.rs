//! Dollar topics test ($-prefixed topic behavior)
//!
//! Per MQTT spec, topics starting with '$' are reserved and should
//! NOT be matched by '#' or '+' wildcard subscriptions. However,
//! some brokers may allow this. This test verifies dollar topic
//! behavior and explicit $SYS subscription.
//!
//! Only a topic name that *begins with* `$` is reserved ([MQTT-4.7.2-1]); a
//! `$`-prefixed level deeper in the name is an ordinary level, which is the
//! shape the Homie convention publishes device state in (GitHub issue #537).

use std::time::{Duration, Instant};

use bytestring::ByteString;
use rmqtt_codec::v3::LastWill;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v311::client::IncomingMessage;
use crate::mqtt::v311::MqttV311Client;

/// Test dollar-prefixed topic behavior with # wildcard and explicit subscription
pub struct DollarTopicsTest;

impl TestCase for DollarTopicsTest {
    fn name(&self) -> &str {
        "dollar_topics"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "dollar-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "dollar-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            // Subscribe to # wildcard
            subscriber.subscribe("#", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Drain any stale retained messages from previous tests
            while subscriber.recv_message_timeout(Duration::from_millis(100)).await.is_some() {}

            // Publish to normal topic - should always be received
            publisher.publish("test/dollar/normal", b"normal_msg", QoS::AtLeastOnce, false).await?;
            let mut msg = subscriber.recv_message_timeout(Duration::from_secs(3)).await;
            // Other concurrent tests may also publish to topics matched by `#`;
            // filter until we see our own "normal_msg" (or timeout).
            let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
            while msg.as_ref().is_none_or(|m| m.payload.as_ref() != b"normal_msg")
                && tokio::time::Instant::now() < deadline
            {
                msg = subscriber.recv_message_timeout(Duration::from_millis(200)).await;
            }
            let got_normal = msg.as_ref().is_some_and(|m| m.payload.as_ref() == b"normal_msg");
            if !got_normal {
                return Err(anyhow::anyhow!("# wildcard did not match normal topic"));
            }

            // Publish to $SYS topic - behavior is broker-dependent
            publisher.publish("$SYS/broker/version", b"dollar_sys", QoS::AtLeastOnce, false).await?;
            let dollar_via_hash = subscriber.recv_message_timeout(Duration::from_secs(2)).await;

            // Some brokers exclude $-topics from # matches (spec-compliant),
            // others include them. Either is acceptable for this test.
            if dollar_via_hash.as_ref().is_some_and(|m| m.payload.as_ref() == b"dollar_sys") {
                eprintln!("Note: broker delivers $SYS via # wildcard (non-standard)");
            }

            // Now subscribe explicitly to $SYS/#
            subscriber.subscribe("$SYS/#", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Publish to $SYS topic - should be received via explicit subscription
            publisher.publish("$SYS/broker/version", b"dollar_sys_explicit", QoS::AtLeastOnce, false).await?;
            let mut msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            // Filter out unrelated messages from other concurrent tests.
            let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
            while msg.as_ref().is_none_or(|m| m.payload.as_ref() != b"dollar_sys_explicit")
                && tokio::time::Instant::now() < deadline
            {
                msg = subscriber.recv_message_timeout(Duration::from_millis(200)).await;
            }

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"dollar_sys_explicit" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected message: {:?}", m.payload)),
                None => Err(anyhow::anyhow!(
                    "explicit $SYS/# did not match $SYS topic - broker may not support $ topics"
                )),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(25)
    }
}

/// Drain whatever is already queued (retained deliveries left by earlier cases,
/// other cases' traffic) so the assertions below only see their own messages.
/// Bounded by `max`, so a chatty neighbour cannot stall the case.
async fn drain(client: &mut MqttV311Client, quiet: Duration, max: Duration) {
    let deadline = tokio::time::Instant::now() + max;
    while tokio::time::Instant::now() < deadline && client.recv_message_timeout(quiet).await.is_some() {}
}

/// Read until `payload` arrives or `timeout` elapses, skipping the traffic of
/// the cases sharing this broker.
async fn wait_for_payload(
    client: &mut MqttV311Client,
    payload: &[u8],
    timeout: Duration,
) -> Option<IncomingMessage> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let Some(m) = client.recv_message_timeout(remaining.min(Duration::from_millis(200))).await else {
            continue;
        };
        if m.payload.as_ref() == payload {
            return Some(m);
        }
    }
}

/// A `$`-prefixed level is reserved only in the first position (GitHub issue #537)
///
/// MQTT 3.1.1 reserves topic names *beginning with* `$` ([MQTT-4.7.2-1]), so
/// `homie/dev/dollar/$state` — the shape the Homie convention puts a device's
/// state in — is an ordinary topic name. `Topic::is_valid()` rejected any
/// `$`-prefixed level that was not the first: every PUBLISH to such a topic was
/// dropped and a Last Will set on one made the broker log `InvalidTopic` and
/// discard it.
///
/// The isolation is deliberately *not* part of what changed: a `#` filter must
/// still fail to match a topic name whose first level starts with `$`.
pub struct DollarTopicsNonLeadingV311Test;

impl TestCase for DollarTopicsNonLeadingV311Test {
    fn name(&self) -> &str {
        "dollar_topics_non_leading_v311"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let state_topic = "homie/dev/dollar/$state";
            let sys_probe = "$SYS/dollar/non_leading_probe";

            let publisher = MqttV311Client::connect(
                &ctx.config.broker_addr,
                "dollar-nonleading-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut hash_sub = MqttV311Client::connect(
                &ctx.config.broker_addr,
                "dollar-nonleading-hash",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut exact_sub = MqttV311Client::connect(
                &ctx.config.broker_addr,
                "dollar-nonleading-exact",
                ctx.config.connect_timeout,
            )
            .await?;

            // `#` reaches every name that does not *begin* with `$`. The exact
            // subscription is the control that keeps the isolation check below
            // from passing vacuously: it proves the `$SYS` probe was routed.
            hash_sub.subscribe("#", QoS::AtLeastOnce).await?;
            exact_sub.subscribe(sys_probe, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;
            drain(&mut hash_sub, Duration::from_millis(100), Duration::from_secs(2)).await;
            drain(&mut exact_sub, Duration::from_millis(100), Duration::from_secs(2)).await;

            // 1. A PUBLISH whose `$` level is not the first must be accepted
            //    and delivered.
            publisher.publish(state_topic, b"non_leading_state", QoS::AtLeastOnce, false).await?;
            match wait_for_payload(&mut hash_sub, b"non_leading_state", Duration::from_secs(3)).await {
                Some(m) if m.topic == state_topic => {}
                Some(m) => {
                    return Err(anyhow::anyhow!(
                        "unexpected topic for the non-leading $ publish: {}",
                        m.topic
                    ))
                }
                None => {
                    return Err(anyhow::anyhow!(
                        "PUBLISH to `{state_topic}` was not delivered - a `$` level that is not \
                         the first must be accepted"
                    ))
                }
            }

            // 2. The isolation is intact: `#` must not reach a name starting
            //    with `$`, while the same PUBLISH still reaches its exact
            //    subscriber.
            publisher.publish(sys_probe, b"dollar_sys_probe", QoS::AtLeastOnce, false).await?;
            if wait_for_payload(&mut exact_sub, b"dollar_sys_probe", Duration::from_secs(3)).await.is_none() {
                return Err(anyhow::anyhow!(
                    "`{sys_probe}` was not routed at all - the isolation check would be vacuous"
                ));
            }
            if let Some(m) =
                wait_for_payload(&mut hash_sub, b"dollar_sys_probe", Duration::from_secs(2)).await
            {
                return Err(anyhow::anyhow!(
                    "`#` matched `{}`, a topic name starting with `$` [MQTT-4.7.2-1]",
                    m.topic
                ));
            }

            // 3. A Last Will on such a topic must be published when the client
            //    dies without a DISCONNECT.
            let will = LastWill {
                qos: QoS::AtLeastOnce,
                retain: false,
                topic: ByteString::from(state_topic),
                message: bytes::Bytes::from("non_leading_will"),
            };
            let will_client = MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "dollar-nonleading-will",
                ctx.config.connect_timeout,
                true,
                60,
                Some(will),
                None,
                None,
            )
            .await?;
            will_client.abort_connection().await?;

            let will_msg = wait_for_payload(&mut hash_sub, b"non_leading_will", Duration::from_secs(5)).await;

            publisher.disconnect().await?;
            hash_sub.disconnect().await?;
            exact_sub.disconnect().await?;

            match will_msg {
                Some(m) if m.topic == state_topic => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected topic for the Last Will: {}", m.topic)),
                None => Err(anyhow::anyhow!(
                    "Last Will on `{state_topic}` was not published - a `$` level that is not \
                     the first must be accepted"
                )),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// Test V5 wildcard_subscription_available flag in CONNACK
pub struct WildcardAvailableV5Test;

impl TestCase for WildcardAvailableV5Test {
    fn name(&self) -> &str {
        "wildcard_available_v5"
    }
    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let client = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "wcavail-v5",
                ctx.config.connect_timeout,
            )
            .await?;
            let ack = client.connack();
            let _ = ack.wildcard_subscription_available;
            client.disconnect().await?;
            Ok(())
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
