//! Load stress tests using v311 client

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Multi-client connection load test using v311 client
pub struct ConnectionLoadTest {
    pub client_count: usize,
}

impl Default for ConnectionLoadTest {
    fn default() -> Self { Self { client_count: 100 } }
}

impl TestCase for ConnectionLoadTest {
    fn name(&self) -> &str { "connection_load" }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let count = self.client_count;

        let result = rt.block_on(async {
            let mut handles = Vec::new();
            let success_count = Arc::new(AtomicU64::new(0));
            let error_count = Arc::new(AtomicU64::new(0));

            for i in 0..count {
                let addr = ctx.config.broker_addr.clone();
                let sc = success_count.clone();
                let ec = error_count.clone();

                let handle = tokio::spawn(async move {
                    match crate::mqtt::v311::MqttV311Client::connect(
                        &addr,
                        &format!("load-conn-{}", i),
                        Duration::from_secs(15),
                    ).await {
                        Ok(client) => {
                            sc.fetch_add(1, Ordering::Relaxed);
                            tokio::time::sleep(Duration::from_millis(100)).await;
                            let _ = client.disconnect().await;
                        }
                        Err(_) => {
                            ec.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
                handles.push(handle);
            }

            for handle in handles {
                let _ = handle.await;
            }

            let _successes = success_count.load(Ordering::Relaxed);
            let errors = error_count.load(Ordering::Relaxed);

            if errors == 0 {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "{} of {} connections failed",
                    errors, count
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

/// Sustained publish load test using v311 client
pub struct PublishLoadTest {
    pub message_count: usize,
    pub payload_size: usize,
}

impl Default for PublishLoadTest {
    fn default() -> Self { Self { message_count: 1000, payload_size: 256 } }
}

impl TestCase for PublishLoadTest {
    fn name(&self) -> &str { "publish_load" }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let msg_count = self.message_count;
        let payload_size = self.payload_size;

        let result = rt.block_on(async {
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "v311-load-pub",
                ctx.config.connect_timeout,
            ).await?;
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "v311-load-sub",
                ctx.config.connect_timeout,
            ).await?;

            let topic = "test/load/publish";
            subscriber.subscribe(topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let payload = vec![0xABu8; payload_size];

            let pub_start = Instant::now();
            for _ in 0..msg_count {
                publisher.publish(topic, &payload, QoS::AtLeastOnce, false).await?;
            }
            let pub_elapsed = pub_start.elapsed();

            // Receive messages
            let mut received = 0usize;
            let recv_deadline = Instant::now() + Duration::from_secs(30);
            while received < msg_count && Instant::now() < recv_deadline {
                if let Some(_msg) = subscriber.recv_message_timeout(Duration::from_secs(5)).await {
                    received += 1;
                } else {
                    break;
                }
            }

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            let qps = msg_count as f64 / pub_elapsed.as_secs_f64();
            tracing::info!(
                "Publish load: {} msgs in {:.2}s ({:.0} msg/s), received {}/{}",
                msg_count, pub_elapsed.as_secs_f64(), qps, received, msg_count
            );

            if received >= msg_count * 90 / 100 {
                Ok(())
            } else {
                Err(anyhow::anyhow!("only received {}/{} messages", received, msg_count))
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "stress", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "stress", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration { Duration::from_secs(120) }
}
