//! MQTT 5.0 ACK reason-code semantics (G14/G15)
//!
//! designs/mqtt-5.0-standalone-test-gap-analysis.md
//!
//! - 0x10 "No matching subscribers" on PUBACK/PUBREC is a MAY-level choice
//!   ("If the Server knows that there are no matching subscribers, it MAY use
//!   this Reason Code instead of 0x00"), so the tests assert that the returned
//!   reason code is *legal* and record the actual value in the note.
//! - 0x11 "No subscription existed" on UNSUBACK follows the same approach.
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// G14: QoS 1 publish to a topic with no subscribers must be answered with a
/// PUBACK carrying a legal reason code (0x00 Success or 0x10 No matching
/// subscribers). Records which one the broker chose.
pub struct ReasonCodeV5PubackNoMatchingTest;

impl TestCase for ReasonCodeV5PubackNoMatchingTest {
    fn name(&self) -> &str {
        "reason_code_v5_puback_no_matching_subscribers"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<String> = rt.block_on(async {
            let mut publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "rc-puback",
                ctx.config.connect_timeout,
            )
            .await?;

            // Unique topic: guarantee there are no matching subscribers.
            let topic = format!("test/v5/nomatch/{}", uuid_v4_suffix());
            publisher.publish(&topic, b"nobody-listens", QoS::AtLeastOnce, false).await?;

            let ack = publisher.recv_puback_reason(Duration::from_secs(5)).await;
            publisher.disconnect().await?;

            match ack {
                Some((_pid, code)) if code == 0x00 || code == 0x10 => {
                    Ok(format!("PUBACK reason code 0x{code:02X}"))
                }
                Some((_pid, code)) => Err(anyhow::anyhow!(
                    "PUBACK with illegal reason code 0x{code:02X} (expected 0x00 or 0x10)"
                )),
                None => Err(anyhow::anyhow!("no PUBACK received within timeout")),
            }
        });

        match result {
            Ok(observation) => {
                TestResult::passed_with_note(self.name(), "functional_v5", start.elapsed(), &observation)
            }
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// G15: UNSUBSCRIBE for a topic filter with no existing subscription must be
/// answered with an UNSUBACK whose reason codes are legal (0x00 Success or
/// 0x11 No Subscription Existed). Records which one the broker chose.
pub struct ReasonCodeV5UnsubackNoSubscriptionTest;

impl TestCase for ReasonCodeV5UnsubackNoSubscriptionTest {
    fn name(&self) -> &str {
        "reason_code_v5_unsuback_no_subscription"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<String> = rt.block_on(async {
            let mut client = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "rc-unsuback",
                ctx.config.connect_timeout,
            )
            .await?;

            // Never subscribed on this session.
            let topic = format!("test/v5/never-subscribed/{}", uuid_v4_suffix());
            let status = client.unsubscribe_with_ack(&topic).await?;
            client.disconnect().await?;

            if status.is_empty() {
                return Err(anyhow::anyhow!("UNSUBACK carried zero return codes [MQTT-3.11.3 violation]"));
            }

            let all_legal = status.iter().all(|&c| c as u8 == 0x00 || c as u8 == 0x11);
            let summary = status.iter().map(|c| format!("0x{:02X}", *c as u8)).collect::<Vec<_>>().join(",");

            if all_legal {
                Ok(format!("UNSUBACK reason code(s) {summary}"))
            } else {
                Err(anyhow::anyhow!("UNSUBACK with illegal reason code(s) {summary} (expected 0x00 or 0x11)"))
            }
        });

        match result {
            Ok(observation) => {
                TestResult::passed_with_note(self.name(), "functional_v5", start.elapsed(), &observation)
            }
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// Short unique suffix so tests never collide with state left by other runs.
fn uuid_v4_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    format!("{}{:04}", std::process::id(), nanos % 10000)
}
