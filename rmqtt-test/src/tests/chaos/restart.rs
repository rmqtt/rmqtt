//! Broker restart chaos test (using v311 client)

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Test that clients can reconnect after broker restart
pub struct BrokerRestartTest;

impl TestCase for BrokerRestartTest {
    fn name(&self) -> &str { "chaos_broker_restart" }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            // Connect a client
            let client = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "chaos-restart",
                ctx.config.connect_timeout,
            ).await?;
            assert!(client.is_connected());

            // Disconnect client
            client.disconnect().await?;

            // Restart broker (sync - uses dedicated runtime internally)
            ctx.restart_broker()?;

            // Wait for broker to be healthy
            let healthy = ctx.broker_healthy();
            if !healthy {
                // Give it more time
                tokio::time::sleep(Duration::from_secs(2)).await;
                let healthy = ctx.broker_healthy();
                if !healthy {
                    return Err(anyhow::anyhow!("broker not healthy after restart"));
                }
            }

            // Reconnect
            let client2 = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "chaos-restart-2",
                ctx.config.connect_timeout,
            ).await?;
            client2.disconnect().await?;

            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "chaos", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "chaos", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration { Duration::from_secs(60) }
}

/// Test publish/subscriber recovery after broker restart
pub struct BrokerRestartPubSubTest;

impl TestCase for BrokerRestartPubSubTest {
    fn name(&self) -> &str { "chaos_broker_restart_pubsub" }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            // Initial publish
            let pub1 = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "chaos-pubsub-pub1",
                ctx.config.connect_timeout,
            ).await?;
            pub1.publish("test/chaos/restart", b"before", QoS::AtLeastOnce, false).await?;
            pub1.disconnect().await?;

            // Restart broker (sync)
            ctx.restart_broker()?;
            tokio::time::sleep(Duration::from_secs(2)).await;

            // Reconnect and verify pub/sub still works
            let pub2 = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "chaos-pubsub-pub2",
                ctx.config.connect_timeout,
            ).await?;
            let mut sub = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "chaos-pubsub-sub",
                ctx.config.connect_timeout,
            ).await?;

            sub.subscribe("test/chaos/restart", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            pub2.publish("test/chaos/restart", b"after", QoS::AtLeastOnce, false).await?;

            let msg = sub.recv_message_timeout(Duration::from_secs(5)).await;
            pub2.disconnect().await?;
            sub.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"after" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected payload: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("no message after broker restart")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "chaos", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "chaos", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration { Duration::from_secs(60) }
}
