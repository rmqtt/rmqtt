//! Shared subscription functional tests for MQTT v3.1.1 and v5
//!
//! Tests `$share/{group}/{topic}` shared subscription behavior,
//! where messages are load-balanced among subscribers in the same group.

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Shared subscription test with MQTT v3.1.1 clients.
///
/// Two subscribers join `$share/grp/test/shared` at QoS 1.
/// A publisher sends 4 messages to `test/shared`.
/// Verifies messages are distributed across subscribers (total >= 2, each >= 1).
pub struct SharedSubV311Test;

impl TestCase for SharedSubV311Test {
    fn name(&self) -> &str {
        "shared_sub_v311"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let mut subscriber1 = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "v311-shared-sub1",
                ctx.config.connect_timeout,
                true,
                60,
                None,
                None,
                None,
            )
            .await?;
            let mut subscriber2 = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "v311-shared-sub2",
                ctx.config.connect_timeout,
                true,
                60,
                None,
                None,
                None,
            )
            .await?;
            let publisher = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "v311-shared-pub",
                ctx.config.connect_timeout,
                true,
                60,
                None,
                None,
                None,
            )
            .await?;

            let shared_topic = "$share/grp/test/shared";
            subscriber1.subscribe(shared_topic, QoS::AtLeastOnce).await?;
            subscriber2.subscribe(shared_topic, QoS::AtLeastOnce).await?;

            tokio::time::sleep(Duration::from_millis(300)).await;

            let topic = "test/shared";
            let msg_count = 10;
            for i in 0..msg_count {
                let payload = format!("msg{}", i);
                publisher.publish(topic, payload.as_bytes(), QoS::AtLeastOnce, false).await?;
            }

            let mut sub1_count = 0u32;
            while subscriber1.recv_message_timeout(Duration::from_secs(3)).await.is_some() {
                sub1_count += 1;
            }

            let mut sub2_count = 0u32;
            while subscriber2.recv_message_timeout(Duration::from_secs(3)).await.is_some() {
                sub2_count += 1;
            }

            publisher.disconnect().await?;
            subscriber1.disconnect().await?;
            subscriber2.disconnect().await?;

            let total = sub1_count + sub2_count;
            if total < 5 {
                return Err(anyhow::anyhow!(
                    "expected at least 5 shared sub messages total, got {} (sub1={}, sub2={})",
                    total,
                    sub1_count,
                    sub2_count
                ));
            }
            if sub1_count < 1 {
                return Err(anyhow::anyhow!("subscriber1 should get at least 1 message, got 0"));
            }
            if sub2_count < 1 {
                return Err(anyhow::anyhow!("subscriber2 should get at least 1 message, got 0"));
            }
            Ok(())
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

/// Shared subscription test with MQTT v5.0 clients.
///
/// Two subscribers join `$share/grp/test/shared_v5` at QoS 1.
/// A publisher sends 4 messages to `test/shared_v5`.
/// Verifies messages are distributed across subscribers (total >= 2, each >= 1).
pub struct SharedSubV5Test;

impl TestCase for SharedSubV5Test {
    fn name(&self) -> &str {
        "shared_sub_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let mut subscriber1 = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "v5-shared-sub1",
                ctx.config.connect_timeout,
                true,
                60,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await?;
            let mut subscriber2 = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "v5-shared-sub2",
                ctx.config.connect_timeout,
                true,
                60,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await?;
            let publisher = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "v5-shared-pub",
                ctx.config.connect_timeout,
                true,
                60,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await?;

            let shared_topic = "$share/grp/test/shared_v5";
            subscriber1.subscribe(shared_topic, QoS::AtLeastOnce).await?;
            subscriber2.subscribe(shared_topic, QoS::AtLeastOnce).await?;

            tokio::time::sleep(Duration::from_millis(300)).await;

            let topic = "test/shared_v5";
            let msg_count = 10;
            for i in 0..msg_count {
                let payload = format!("msg{}", i);
                publisher.publish(topic, payload.as_bytes(), QoS::AtLeastOnce, false).await?;
            }

            let mut sub1_count = 0u32;
            while subscriber1.recv_message_timeout(Duration::from_secs(3)).await.is_some() {
                sub1_count += 1;
            }

            let mut sub2_count = 0u32;
            while subscriber2.recv_message_timeout(Duration::from_secs(3)).await.is_some() {
                sub2_count += 1;
            }

            publisher.disconnect().await?;
            subscriber1.disconnect().await?;
            subscriber2.disconnect().await?;

            let total = sub1_count + sub2_count;
            if total < 5 {
                return Err(anyhow::anyhow!(
                    "expected at least 5 shared sub messages total, got {} (sub1={}, sub2={})",
                    total,
                    sub1_count,
                    sub2_count
                ));
            }
            if sub1_count < 1 {
                return Err(anyhow::anyhow!("subscriber1 should get at least 1 message, got 0"));
            }
            if sub2_count < 1 {
                return Err(anyhow::anyhow!("subscriber2 should get at least 1 message, got 0"));
            }
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}
