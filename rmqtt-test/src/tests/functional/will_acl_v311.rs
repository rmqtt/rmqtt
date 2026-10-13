//! Will Message publish ACL regression tests (v3.1.1)
//!
//! A Will Message is a PUBLISH the broker sends on a client's behalf when the
//! connection drops without a DISCONNECT. It has to pass the same publish ACL
//! as a PUBLISH the client sends itself. Before the fix, the Will path in
//! `Session::process_last_will` skipped `message_publish_check_acl`, so a
//! client that was denied publishing to a topic could still get a message onto
//! it: register the topic as its Will, then close the socket.
//!
//! Fixture: `configs/will-acl` enables `rmqtt-acl` with one rule that denies
//! publishing to `test/will-acl/denied` for everyone. Everything else is
//! allowed. Both cases run against that broker (`functional_v311@will-acl`).
//!
//! The denied case also sends an *allowed* Will right after the denied one and
//! checks that the allowed one is the first (and only) message to arrive. That
//! way a broker that stopped sending Wills altogether cannot pass by accident.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bytestring::ByteString;
use rmqtt_codec::v3::LastWill;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v311::MqttV311Client;

const SUITE: &str = "functional_v311";
/// The fixture pins its own MQTT port (see configs/will-acl/rmqtt.toml).
const BROKER_ADDR: &str = "127.0.0.1:1905";
const DENIED_TOPIC: &str = "test/will-acl/denied";
const ALLOWED_TOPIC: &str = "test/will-acl/allowed";

fn will_acl_config() -> Option<PathBuf> {
    Some(crate::tests::config_path("will-acl"))
}

fn will(topic: &str, payload: &'static str) -> LastWill {
    LastWill {
        qos: QoS::AtLeastOnce,
        retain: false,
        topic: ByteString::from(topic),
        message: bytes::Bytes::from(payload),
    }
}

/// Connect a client whose only job is to carry a Will, then drop its socket
/// without a DISCONNECT so the broker publishes the Will.
async fn connect_and_abort_with_will(ctx: &TestContext, client_id: &str, lw: LastWill) -> anyhow::Result<()> {
    let client = MqttV311Client::connect_with_options(
        &ctx.config.broker_addr,
        client_id,
        ctx.config.connect_timeout,
        true,
        60,
        Some(lw),
        None,
        None,
    )
    .await?;
    client.abort_connection().await?;
    Ok(())
}

/// Control case: a Will on a topic the client may publish to is delivered.
pub struct WillAclAllowedTopicPublishedV311Test;

impl TestCase for WillAclAllowedTopicPublishedV311Test {
    fn name(&self) -> &str {
        "will_acl_allowed_topic_published_v311"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        will_acl_config()
    }

    fn broker_addr(&self) -> Option<&'static str> {
        Some(BROKER_ADDR)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let mut sub = MqttV311Client::connect(
                &ctx.config.broker_addr,
                "will-acl-allowed-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            sub.subscribe(ALLOWED_TOPIC, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            connect_and_abort_with_will(ctx, "will-acl-allowed-client", will(ALLOWED_TOPIC, "allowed-will"))
                .await?;

            let msg = sub.recv_message_timeout(Duration::from_secs(5)).await;
            sub.disconnect().await?;

            match msg {
                Some(m) if m.topic == ALLOWED_TOPIC && m.payload.as_ref() == b"allowed-will" => Ok(()),
                Some(m) => {
                    Err(anyhow::anyhow!("unexpected message: topic={}, payload={:?}", m.topic, m.payload))
                }
                None => Err(anyhow::anyhow!("allowed Will was not delivered")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), SUITE, start.elapsed()),
            Err(e) => TestResult::failed(self.name(), SUITE, start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// A Will on a topic the client may NOT publish to is dropped by the ACL.
pub struct WillAclDeniedTopicDroppedV311Test;

impl TestCase for WillAclDeniedTopicDroppedV311Test {
    fn name(&self) -> &str {
        "will_acl_denied_topic_dropped_v311"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        will_acl_config()
    }

    fn broker_addr(&self) -> Option<&'static str> {
        Some(BROKER_ADDR)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let mut sub = MqttV311Client::connect(
                &ctx.config.broker_addr,
                "will-acl-denied-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            sub.subscribe(DENIED_TOPIC, QoS::AtLeastOnce).await?;
            sub.subscribe(ALLOWED_TOPIC, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // 1) The Will under test: its topic is denied to every publisher.
            connect_and_abort_with_will(ctx, "will-acl-denied-client", will(DENIED_TOPIC, "denied-will"))
                .await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // 2) A marker Will on an allowed topic. If the denied Will had been
            //    published it would have arrived before this one.
            connect_and_abort_with_will(ctx, "will-acl-marker-client", will(ALLOWED_TOPIC, "marker-will"))
                .await?;

            let first = sub.recv_message_timeout(Duration::from_secs(5)).await;
            let second = sub.recv_message_timeout(Duration::from_secs(1)).await;
            sub.disconnect().await?;

            match first {
                None => return Err(anyhow::anyhow!("no Will arrived at all; the marker Will should have")),
                Some(m) if m.topic == DENIED_TOPIC => {
                    return Err(anyhow::anyhow!(
                        "Will on the denied topic was published despite the ACL (payload={:?})",
                        m.payload
                    ));
                }
                Some(m) if m.topic == ALLOWED_TOPIC && m.payload.as_ref() == b"marker-will" => {}
                Some(m) => {
                    return Err(anyhow::anyhow!(
                        "unexpected message: topic={}, payload={:?}",
                        m.topic,
                        m.payload
                    ));
                }
            }
            if let Some(m) = second {
                return Err(anyhow::anyhow!(
                    "a second message arrived after the marker: topic={}, payload={:?}",
                    m.topic,
                    m.payload
                ));
            }
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), SUITE, start.elapsed()),
            Err(e) => TestResult::failed(self.name(), SUITE, start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}
