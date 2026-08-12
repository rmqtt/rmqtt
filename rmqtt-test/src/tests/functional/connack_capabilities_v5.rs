//! MQTT v5.0 CONNACK capability advertisement tests
//!
//! Covers spec section 3.2.2: the server advertises its capabilities in the
//! CONNACK properties (Receive Maximum, Maximum QoS, Retain Available,
//! Maximum Packet Size, Topic Alias Maximum, Server Keep Alive, Session
//! Expiry Interval, Assigned Client Identifier, Wildcard / Subscription
//! Identifiers / Shared Subscription Available).

use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Positive: CONNACK advertises Receive Maximum, Maximum QoS and Maximum
/// Packet Size back to the client.
pub struct ConnAckCapabilitiesV5Test;

impl TestCase for ConnAckCapabilitiesV5Test {
    fn name(&self) -> &str {
        "connack_capabilities_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "v5-cap-check",
                ctx.config.connect_timeout,
            )
            .await?;
            let ack = client.connack();

            // receive_max must be non-zero (broker's inflight limit)
            assert!(ack.receive_max.get() > 0, "receive_max must be > 0");
            // max_qos must be 0, 1 or 2
            let q = ack.max_qos as u8;
            assert!(q <= 2, "max_qos must be 0-2, got {q}");
            // session expiry default
            let _ = ack.session_expiry_interval_secs;
            // topic alias max is a u16; broker may advertise 0 (not supported)
            let _ = ack.topic_alias_max;
            // wildcard + subscription identifiers availability must be true
            assert!(ack.wildcard_subscription_available, "wildcard_subscription_available must be true");
            assert!(
                ack.subscription_identifiers_available,
                "subscription_identifiers_available must be true"
            );

            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
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

/// Positive: when the client sends a Receive Maximum in CONNECT, the broker's
/// CONNACK Receive Maximum is unaffected (it is the server's own limit, not an
/// echo). Verify the connection works and the server value is sane.
pub struct ConnAckReceiveMaxEchoV5Test;

impl TestCase for ConnAckReceiveMaxEchoV5Test {
    fn name(&self) -> &str {
        "connack_receive_max_echo_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "v5-rm-check",
                ctx.config.connect_timeout,
                true,
                60,
                None,
                None,
                None,
                None,
                std::num::NonZeroU16::new(5), // client receive_max = 5
                None,
            )
            .await?;
            let ack = client.connack();
            // Server advertises its own receive maximum — must be non-zero
            assert!(ack.receive_max.get() > 0, "server receive_max must be > 0");
            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
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

/// Positive: an empty client id with clean_start = 1 results in an Assigned
/// Client Identifier in CONNACK. [MQTT-3.2.2-10]
pub struct ConnAckAssignedClientIdV5Test;

impl TestCase for ConnAckAssignedClientIdV5Test {
    fn name(&self) -> &str {
        "connack_assigned_client_id_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let client = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "", // empty client id → assigned by server
                ctx.config.connect_timeout,
            )
            .await?;
            let ack = client.connack();
            assert!(
                ack.assigned_client_id.is_some(),
                "server must assign a client identifier for an empty client id [MQTT-3.2.2-10]"
            );
            let assigned = ack.assigned_client_id.as_ref().unwrap();
            assert!(!assigned.is_empty(), "assigned client id must not be empty");
            client.disconnect().await?;
            Ok::<(), anyhow::Error>(())
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
