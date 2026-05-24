//! MQTT 5.0 Session management tests

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Test session persistence with session_expiry_interval (v5)
pub struct SessionExpiryV5Test;

impl TestCase for SessionExpiryV5Test {
    fn name(&self) -> &str {
        "session_expiry_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let topic = "test/v5/session/persist";
            let payload = b"persisted_msg";

            // Phase 1: Connect with clean_start=false + session_expiry
            let mut client = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "session-v5-client",
                ctx.config.connect_timeout,
                false, // clean_start = false
                60,
                None,
                None,
                None,
                Some(3600), // session_expiry_interval
                None,
            )
            .await?;

            // Subscribe
            client.subscribe(topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Disconnect (session should persist)
            client.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 2: Publish while client is disconnected
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "session-v5-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            publisher.publish(topic, payload, QoS::AtLeastOnce, false).await?;
            publisher.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 3: Reconnect with same client_id + clean_start=false
            let mut reconnected = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "session-v5-client",
                ctx.config.connect_timeout,
                false,
                60,
                None,
                None,
                None,
                Some(3600),
                None,
            )
            .await?;

            // Should receive the queued message
            let msg = reconnected.recv_message_timeout(Duration::from_secs(5)).await;
            reconnected.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == payload => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected persisted msg: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("no queued message received after reconnect")),
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

/// Test session takeover - new connection with same Client ID takes over existing session (v5)
pub struct SessionTakeoverV5Test;

impl TestCase for SessionTakeoverV5Test {
    fn name(&self) -> &str {
        "session_takeover_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            // First connection
            let client1 = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "takeover-v5",
                ctx.config.connect_timeout,
            )
            .await?;
            assert!(client1.is_connected());

            // Second connection with SAME client ID should take over
            let client2 = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "takeover-v5",
                ctx.config.connect_timeout,
            )
            .await?;
            assert!(client2.is_connected());

            // First client should be disconnected now
            tokio::time::sleep(Duration::from_millis(200)).await;
            if client1.is_connected() {
                return Err(anyhow::anyhow!("client1 should have been taken over"));
            }

            let _ = client1.disconnect().await;
            client2.disconnect().await?;
            Ok::<(), anyhow::Error>(())
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

/// Test clean_start=true clears previous session (v5)
pub struct SessionCleanStartV5Test;

impl TestCase for SessionCleanStartV5Test {
    fn name(&self) -> &str {
        "session_clean_start_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let topic = "test/v5/session/cleanstart";

            // Connect with persistent session and subscribe
            let mut client = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "clean-start-client",
                ctx.config.connect_timeout,
                false,
                60,
                None,
                None,
                None,
                Some(3600),
                None,
            )
            .await?;
            client.subscribe(topic, QoS::AtLeastOnce).await?;
            client.disconnect().await?;

            tokio::time::sleep(Duration::from_millis(200)).await;

            // Publish while disconnected
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "clean-start-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            publisher.publish(topic, b"queued_message", QoS::AtLeastOnce, false).await?;
            publisher.disconnect().await?;

            tokio::time::sleep(Duration::from_millis(200)).await;

            // Reconnect with clean_start=true - old session cleared, no queued msg
            let mut reconnected = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "clean-start-client",
                ctx.config.connect_timeout,
                true, // clean_start = true
                60,
                None,
                None,
                None,
                Some(3600),
                None,
            )
            .await?;

            // Should NOT receive the queued message
            let msg = reconnected.recv_message_timeout(Duration::from_secs(3)).await;
            reconnected.disconnect().await?;

            if msg.is_some() {
                Err(anyhow::anyhow!("received queued message despite clean_start=true"))
            } else {
                Ok(())
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
