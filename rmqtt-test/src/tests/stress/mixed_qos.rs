//! Mixed-QoS load stress test (P3 G33): the existing fanout / load tests are
//! QoS 0/1 dominated; this test drives QoS 0, 1 and 2 publishes over one
//! session and verifies that every message arrives exactly once.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Publish `message_count` messages with the QoS rotating 0/1/2, then verify
/// the QoS 2 subscriber receives all of them exactly once (no loss, no
/// duplicates — MQTT-4.6.0-2 / test spec §21). [G33]
pub struct MixedQosLoadTest {
    pub message_count: usize,
}

impl Default for MixedQosLoadTest {
    fn default() -> Self {
        Self { message_count: 200 }
    }
}

impl TestCase for MixedQosLoadTest {
    fn name(&self) -> &str {
        "stress_mixed_qos_v311"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let msg_count = self.message_count;

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "mixed-qos-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "mixed-qos-sub",
                ctx.config.connect_timeout,
            )
            .await?;

            let topic = format!("test/stress/mixed-qos/{}", uuid::Uuid::new_v4().simple());
            // QoS 2 subscription: the broker must deliver every QoS 2 message
            // exactly once, and QoS 0/1 messages must not be lost either.
            subscriber.subscribe(&topic, QoS::ExactlyOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Publish with QoS rotating 0 -> 1 -> 2; payload starts with the
            // 2-byte big-endian sequence number.
            for i in 0..msg_count {
                let qos = match i % 3 {
                    0 => QoS::AtMostOnce,
                    1 => QoS::AtLeastOnce,
                    _ => QoS::ExactlyOnce,
                };
                let mut payload = Vec::with_capacity(2 + 16);
                payload.extend_from_slice(&(i as u16).to_be_bytes());
                payload.extend_from_slice(b"mixed-qos-payload");
                publisher.publish(&topic, &payload, qos, false).await?;
            }
            publisher.disconnect().await?;

            // Collect until all arrived or the deadline expires.
            let mut seen: HashSet<u16> = HashSet::new();
            let mut duplicates = 0usize;
            let deadline = Instant::now() + Duration::from_secs(30);
            while seen.len() < msg_count && Instant::now() < deadline {
                match subscriber.recv_message_timeout(Duration::from_secs(5)).await {
                    Some(msg) if msg.payload.len() >= 2 => {
                        let seq = u16::from_be_bytes([msg.payload[0], msg.payload[1]]);
                        if !seen.insert(seq) {
                            duplicates += 1;
                        }
                    }
                    Some(_) => {
                        return Err(anyhow::anyhow!("message with payload shorter than 2 bytes"));
                    }
                    None => break,
                }
            }
            subscriber.disconnect().await?;

            if seen.len() != msg_count {
                return Err(anyhow::anyhow!("mixed QoS: only {}/{} messages arrived", seen.len(), msg_count));
            }
            if duplicates > 0 {
                return Err(anyhow::anyhow!("mixed QoS: {duplicates} duplicate deliveries"));
            }
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "stress", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "stress", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(60)
    }
}
