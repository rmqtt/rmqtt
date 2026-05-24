//! Fan-out stress tests using v311 client

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Fan-out test: one publisher, N subscribers (using v311 client)
pub struct FanOutTest {
    pub subscriber_count: usize,
    pub message_count: usize,
}

impl Default for FanOutTest {
    fn default() -> Self { Self { subscriber_count: 10, message_count: 5 } }
}

impl TestCase for FanOutTest {
    fn name(&self) -> &str { "fan_out" }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let sub_count = self.subscriber_count;
        let msg_count = self.message_count;

        let result = rt.block_on(async {
            // Start subscribers
            let mut subscribers = Vec::new();
            for i in 0..sub_count {
                let sub = crate::mqtt::v311::MqttV311Client::connect(
                    &ctx.config.broker_addr,
                    &format!("v311-fanout-sub-{}", i),
                    ctx.config.connect_timeout,
                ).await?;
                subscribers.push(sub);
            }

            // Subscribe
            let topic = "test/stress/fanout";
            for sub in &mut subscribers {
                sub.subscribe(topic, QoS::AtLeastOnce).await?;
            }

            tokio::time::sleep(Duration::from_millis(200)).await;

            // Publish messages
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "v311-fanout-pub",
                ctx.config.connect_timeout,
            ).await?;

            for i in 0..msg_count {
                let payload = format!("fanout-msg-{}", i);
                publisher.publish(topic, payload.as_bytes(), QoS::AtLeastOnce, false).await?;
            }

            // Each subscriber should receive all messages
            let total_expected = sub_count * msg_count;
            let total_received = Arc::new(AtomicU64::new(0));

            let mut handles = Vec::new();
            for mut sub in subscribers {
                let tr = total_received.clone();
                let handle = tokio::spawn(async move {
                    for _ in 0..msg_count {
                        if let Some(_msg) = sub.recv_message_timeout(Duration::from_secs(10)).await {
                            tr.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    let _ = sub.disconnect().await;
                });
                handles.push(handle);
            }

            for handle in handles {
                let _ = handle.await;
            }

            publisher.disconnect().await?;

            let received = total_received.load(Ordering::Relaxed);
            if received as usize >= total_expected * 90 / 100 {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "fan-out: received {}/{} messages ({:.0}%)",
                    received, total_expected,
                    (received as f64 / total_expected as f64) * 100.0
                ))
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "stress", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "stress", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration { Duration::from_secs(120) }
}
