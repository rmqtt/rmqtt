//! Broker restart chaos test (using v311 client)

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Test that clients can reconnect after broker restart
pub struct BrokerRestartTest;

impl TestCase for BrokerRestartTest {
    fn name(&self) -> &str {
        "chaos_broker_restart"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        // Broker-restart chaos tests require this harness to manage the broker
        // process; in `--no-broker` mode there is nothing to restart → skip.
        if !ctx.has_broker() {
            return TestResult::skipped(
                self.name(),
                "chaos",
                start.elapsed(),
                "no broker managed by this context (--no-broker mode)",
            );
        }
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            // Connect a client
            let client = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "chaos-restart",
                ctx.config.connect_timeout,
            )
            .await?;
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
            )
            .await?;
            client2.disconnect().await?;

            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "chaos", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "chaos", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(60)
    }
}

/// Test publish/subscriber recovery after broker restart
pub struct BrokerRestartPubSubTest;

impl TestCase for BrokerRestartPubSubTest {
    fn name(&self) -> &str {
        "chaos_broker_restart_pubsub"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        // Broker-restart chaos tests require this harness to manage the broker
        // process; in `--no-broker` mode there is nothing to restart → skip.
        if !ctx.has_broker() {
            return TestResult::skipped(
                self.name(),
                "chaos",
                start.elapsed(),
                "no broker managed by this context (--no-broker mode)",
            );
        }
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            // Initial publish
            let pub1 = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "chaos-pubsub-pub1",
                ctx.config.connect_timeout,
            )
            .await?;
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
            )
            .await?;
            let mut sub = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "chaos-pubsub-sub",
                ctx.config.connect_timeout,
            )
            .await?;

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

    fn timeout(&self) -> Duration {
        Duration::from_secs(60)
    }
}

/// GitHub issue #475 reproduction: after a broker restart, a persistent
/// session restored from sled storage is placed back into `peers` with its
/// subscriptions, but **no route is ever added to the router**
/// (`rebuild_offline_sessions` never calls `router().add(..)`; the only two
/// `router().add` call sites are SUBSCRIBE in `shared.rs` and
/// `transfer_session_state` in `session.rs`, neither of which runs at
/// rebuild time). Until the client reconnects and its session transfer is
/// consumed, any publish matching its subscription is PUBACKed and silently
/// dropped — while the broker still answers the reconnect with
/// `session_present = 1` (violating MQTT-3.2.2-2).
///
/// Reproduction sequence (deterministic per the issue report):
///   1. Subscriber A connects with clean_start = false, subscribes to T and
///      disconnects; the session is persisted to sled.
///   2. The broker is restarted on the same sled path; A's offline session
///      is rebuilt from storage (`rebuild_offline_sessions`).
///   3. Client B publishes QoS 1 to T while A is still offline; B receives a
///      PUBACK.
///   4. A reconnects (session_present must be 1) and does **not** re-subscribe.
///      With the bug the first publish had no router entry and was dropped, so
///      A receives nothing → the test FAILS, reproducing #475.
///   5. Control: a further publish by B is delivered to A without any
///      re-subscribe, proving the subscription WAS restored and that the first
///      message was silently dropped rather than never subscribed.
///
/// Requires the sled-backed `rmqtt-session-storage` plugin, enabled by the
/// `session-sled` test config declared via `broker_config()`.
pub struct BrokerRestartSessionRoutingTest;

impl TestCase for BrokerRestartSessionRoutingTest {
    fn name(&self) -> &str {
        "chaos_broker_restart_session_routing"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        Some(crate::tests::config_path("session-sled"))
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        // Broker-restart tests require this harness to manage the broker
        // process; in `--no-broker` mode there is nothing to restart → skip.
        if !ctx.has_broker() {
            return TestResult::skipped(
                self.name(),
                "chaos",
                start.elapsed(),
                "no broker managed by this context (--no-broker mode)",
            );
        }
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let sub_id = format!("issue475-sub-{uid}");
            let pub_id = format!("issue475-pub-{uid}");
            let topic = format!("test/issue475/{uid}");
            let payload_before = b"msg-before-reconnect".as_slice();
            let payload_control = b"msg-after-transfer".as_slice();

            // Phase 1: A connects as a persistent subscriber and disconnects.
            let mut a = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                &sub_id,
                ctx.config.connect_timeout,
                false, // clean_start = false -> persistent session
                60,
                None,
                None,
                None,
                Some(3600), // session expiry interval keeps the session alive
                None,
                None,
            )
            .await?;
            a.subscribe(&topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(200)).await;
            a.disconnect().await?;
            // Give the broker time to persist the session (OfflineMessage hook).
            tokio::time::sleep(Duration::from_secs(1)).await;

            // Phase 2: restart the broker on the same sled path.
            ctx.restart_broker()?;
            if !ctx.broker_healthy() {
                tokio::time::sleep(Duration::from_secs(2)).await;
                if !ctx.broker_healthy() {
                    return Err(anyhow::anyhow!("broker not healthy after restart"));
                }
            }
            // Let the rebuild (BeforeStartup) settle before publishing.
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 3: B publishes QoS 1 to T while A is still offline.
            let b = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                &pub_id,
                ctx.config.connect_timeout,
            )
            .await?;
            b.publish(&topic, payload_before, QoS::AtLeastOnce, false).await?;
            tokio::time::sleep(Duration::from_millis(200)).await;
            b.disconnect().await?;

            // Phase 4: A reconnects WITHOUT re-subscribing.
            let mut a2 = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                &sub_id,
                ctx.config.connect_timeout,
                false,
                60,
                None,
                None,
                None,
                Some(3600),
                None,
                None,
            )
            .await?;
            if !a2.connack().session_present {
                let _ = a2.disconnect().await;
                return Err(anyhow::anyhow!(
                    "session_present = 0 after restart: session was NOT restored from sled \
                     (precondition of issue #475 violated)"
                ));
            }

            let first = a2.recv_message_timeout(Duration::from_secs(5)).await;
            match first {
                Some(m) if m.payload.as_ref() == payload_before => {
                    // The offline publish was delivered -> subscription routing
                    // works across restart (issue fixed).
                    let _ = a2.disconnect().await;
                    Ok(())
                }
                Some(m) => {
                    let _ = a2.disconnect().await;
                    Err(anyhow::anyhow!("unexpected payload: {:?}", m.payload))
                }
                None => {
                    // No message: run the control probe. A further publish must
                    // now be delivered (the session transfer consumed on
                    // reconnect registers the router entry) — proving the
                    // subscription was restored and the first message was
                    // silently dropped.
                    let ctl_id = format!("{pub_id}-ctl");
                    let b2 = crate::mqtt::v5::MqttV5Client::connect(
                        &ctx.config.broker_addr,
                        &ctl_id,
                        ctx.config.connect_timeout,
                    )
                    .await?;
                    b2.publish(&topic, payload_control, QoS::AtLeastOnce, false).await?;
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    b2.disconnect().await?;

                    let control = a2.recv_message_timeout(Duration::from_secs(5)).await;
                    let _ = a2.disconnect().await;
                    match control {
                        Some(m) if m.payload.as_ref() == payload_control => Err(anyhow::anyhow!(
                            "issue #475 reproduced: first publish was PUBACKed but silently \
                             dropped (no router entry for the restored subscription); the \
                             control publish is only delivered after the reconnect's session \
                             transfer registered the route"
                        )),
                        Some(m) => Err(anyhow::anyhow!(
                            "first publish lost, control publish got unexpected payload: {:?}",
                            m.payload
                        )),
                        None => Err(anyhow::anyhow!(
                            "no message delivered even after session transfer: subscription \
                             was not restored at all (different failure mode than issue #475)"
                        )),
                    }
                }
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "chaos", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "chaos", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(90)
    }
}
