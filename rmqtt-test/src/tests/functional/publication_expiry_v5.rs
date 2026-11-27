//! MQTT 5.0 Message Expiry Interval test
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

pub struct PublicationExpiryV5Test;
impl TestCase for PublicationExpiryV5Test {
    fn name(&self) -> &str {
        "publication_expiry_v5"
    }
    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "pe-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "pe-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            subscriber.subscribe("test/v5/pubexpiry", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Publish with a long message expiry interval (3600 seconds)
            publisher
                .publish_with_properties(
                    "test/v5/pubexpiry",
                    b"expiry_msg",
                    QoS::AtLeastOnce,
                    false,
                    None,
                    Some(3600),
                    None,
                    None,
                    None,
                    None,
                )
                .await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(3)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"expiry_msg" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected payload: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("message with expiry not received")),
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
// P1 gap-analysis additions (designs/mqtt-5.0-standalone-test-gap-analysis.md)
// ---------------------------------------------------------------------------

/// G17a: a queued QoS 1 message whose Message Expiry Interval elapses while
/// the subscriber is offline MUST NOT be delivered after reconnecting.
/// [MQTT-3.3.2-5 / MQTT-3.3.2-6]
pub struct MessageExpiryV5QueuedDropTest;

impl TestCase for MessageExpiryV5QueuedDropTest {
    fn name(&self) -> &str {
        "message_expiry_v5_queued_drop"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let topic = "test/v5/meq/queue";

            // Persistent subscriber session, then go offline.
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "meq-sub",
                ctx.config.connect_timeout,
                true, // clean start for the first connection
                60,
                None,
                None,
                None,
                Some(300), // session expiry 300s: session survives disconnect
                None,
                None,
            )
            .await?;
            subscriber.subscribe(topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;
            subscriber.disconnect().await?;

            // Publish a QoS 1 message that expires after 2s, while the
            // subscriber is offline, so it queues for the session.
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "meq-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            publisher
                .publish_with_properties(
                    topic,
                    b"dies-in-queue",
                    QoS::AtLeastOnce,
                    false,
                    None,
                    Some(2), // message expiry 2s
                    None,
                    None,
                    None,
                    None,
                )
                .await?;
            publisher.disconnect().await?;

            // Wait until the message has certainly expired in the queue.
            tokio::time::sleep(Duration::from_secs(4)).await;

            // Resume the persistent session: the expired message must NOT be
            // delivered.
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "meq-sub",
                ctx.config.connect_timeout,
                false, // resume existing session
                60,
                None,
                None,
                None,
                Some(300),
                None,
                None,
            )
            .await?;
            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            subscriber.disconnect().await?;

            match msg {
                None => Ok(()),
                Some(m) => Err(anyhow::anyhow!(
                    "expired queued message was delivered on resume (payload: {:?})",
                    m.payload
                )),
            }
        });
        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(45)
    }
}

/// G17b: when a message with a Message Expiry Interval is forwarded, the
/// server MUST send the remaining interval (received value minus the time
/// spent waiting in the server). [MQTT-3.3.2-6]
pub struct MessageExpiryV5ForwardedTest;

impl TestCase for MessageExpiryV5ForwardedTest {
    fn name(&self) -> &str {
        "message_expiry_v5_forwarded"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "mef-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            subscriber.subscribe("test/v5/mef", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "mef-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            publisher
                .publish_with_properties(
                    "test/v5/mef",
                    b"expiry-60",
                    QoS::AtLeastOnce,
                    false,
                    None,
                    Some(60),
                    None,
                    None,
                    None,
                    None,
                )
                .await?;

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"expiry-60" => match m.message_expiry_interval {
                    Some(exp) if exp.get() <= 60 => Ok(()),
                    Some(exp) => Err(anyhow::anyhow!(
                        "forwarded Message Expiry Interval {} exceeds the published value 60",
                        exp.get()
                    )),
                    None => Err(anyhow::anyhow!("Message Expiry Interval was not forwarded [MQTT-3.3.2-6]")),
                },
                Some(m) => Err(anyhow::anyhow!("unexpected payload: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("message not received")),
            }
        });
        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}
