//! MQTT 5.0 Session management tests

use std::io::{Read, Write};
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

/// Positive: a DISCONNECT carrying Session Expiry Interval = 0 ends the
/// session immediately, so a subsequent reconnect with clean_start = 0 does
/// NOT resume it (session present = 0). [MQTT-3.14.2-1/2]
///
/// Uses a raw socket throughout: (1) CONNECT with SEI=3600 creates the
/// session, (2) DISCONNECT with SEI=0 must delete it, (3) reconnect must see
/// session present = 0.
pub struct SessionV5DisconnectExpiryZeroTest;

impl TestCase for SessionV5DisconnectExpiryZeroTest {
    fn name(&self) -> &str {
        "session_v5_disconnect_expiry_zero"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let result = std::panic::catch_unwind(|| -> anyhow::Result<()> {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let cid = format!("v5-sei0-{uid}");

            // ---- raw CONNECT helper: returns (stream, session_present) ----
            fn raw_connect(
                addr: &str,
                cid: &str,
                clean_start: bool,
                sei: u32,
            ) -> anyhow::Result<(std::net::TcpStream, bool)> {
                let mut stream = std::net::TcpStream::connect(addr)?;
                stream.set_read_timeout(Some(Duration::from_secs(5)))?;
                let mut body: Vec<u8> = Vec::new();
                body.extend_from_slice(&[0x00, 0x04]);
                body.extend_from_slice(b"MQTT");
                body.push(5);
                body.push(if clean_start { 0x02 } else { 0x00 });
                body.extend_from_slice(&[0x00, 0x3C]);
                // properties: Session Expiry Interval (0x11) + 4 bytes
                body.push(0x05);
                body.push(0x11);
                body.extend_from_slice(&sei.to_be_bytes());
                let cb = cid.as_bytes();
                body.extend_from_slice(&(cb.len() as u16).to_be_bytes());
                body.extend_from_slice(cb);
                let mut pkt = vec![0x10];
                let mut len = body.len();
                loop {
                    let mut b = (len % 128) as u8;
                    len /= 128;
                    if len > 0 {
                        b |= 0x80;
                    }
                    pkt.push(b);
                    if len == 0 {
                        break;
                    }
                }
                pkt.extend_from_slice(&body);
                stream.write_all(&pkt)?;
                stream.flush()?;
                // read full CONNACK
                let mut full = Vec::new();
                let mut b = [0u8; 1];
                let n = stream.read(&mut b)?;
                if n == 0 || b[0] != 0x20 {
                    return Err(anyhow::anyhow!("no CONNACK"));
                }
                full.push(b[0]);
                let mut remaining: u32 = 0;
                let mut shift = 0u32;
                loop {
                    let n = stream.read(&mut b)?;
                    if n == 0 {
                        return Err(anyhow::anyhow!("truncated CONNACK"));
                    }
                    full.push(b[0]);
                    remaining |= ((b[0] & 0x7F) as u32) << shift;
                    if b[0] & 0x80 == 0 {
                        break;
                    }
                    shift += 7;
                    if shift > 21 {
                        return Err(anyhow::anyhow!("malformed remaining length"));
                    }
                }
                let mut rest = vec![0u8; remaining as usize];
                stream.read_exact(&mut rest)?;
                full.extend_from_slice(&rest);
                if rest.len() < 2 || rest[1] != 0 {
                    return Err(anyhow::anyhow!(
                        "CONNECT refused, reason {:02x?}",
                        &rest[..rest.len().min(4)]
                    ));
                }
                let session_present = rest[0] & 0x01 != 0;
                Ok((stream, session_present))
            }

            // Phase 1: create the persistent session
            let (mut s1, sp1) = raw_connect(&ctx.config.broker_addr, &cid, false, 3600)?;
            if sp1 {
                return Err(anyhow::anyhow!("fresh session must have session present = 0"));
            }

            // Phase 2: DISCONNECT with Session Expiry Interval = 0
            // [0xE0, len=7, 0x00 (reason), 0x05 (prop len), 0x11, 0,0,0,0]
            let disc: [u8; 9] = [0xE0, 0x07, 0x00, 0x05, 0x11, 0x00, 0x00, 0x00, 0x00];
            s1.write_all(&disc)?;
            s1.flush()?;
            // The broker processes the DISCONNECT when the connection closes;
            // shut down the socket so the session cleanup runs.
            let _ = s1.shutdown(std::net::Shutdown::Both);
            // Give the broker time to process the disconnect + delete the session
            std::thread::sleep(Duration::from_millis(500));

            // Phase 3: reconnect with clean_start = 0 — session must be gone
            let (mut s2, sp2) = raw_connect(&ctx.config.broker_addr, &cid, false, 3600)?;
            if sp2 {
                return Err(anyhow::anyhow!(
                    "session present = 1 after DISCONNECT with SEI = 0 [MQTT-3.14.2-2]"
                ));
            }
            // clean close s2
            s2.write_all(&[0xE0, 0x00])?;
            s2.flush()?;
            Ok(())
        });

        match result {
            Ok(Ok(())) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Ok(Err(e)) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
            Err(_) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), "panic".into()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// Positive: a session with a short Session Expiry Interval is cleaned up by
/// the server after the interval elapses (session present = 0 on reconnect).
pub struct SessionV5ExpiryCleanupTest;

impl TestCase for SessionV5ExpiryCleanupTest {
    fn name(&self) -> &str {
        "session_v5_expiry_cleanup"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let cid = format!("v5-exp-{uid}");

            // Connect with a 1-second session expiry, then disconnect cleanly.
            let client = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                &cid,
                ctx.config.connect_timeout,
                false,
                60,
                None,
                None,
                None,
                Some(1), // 1-second session expiry
                None,
                None,
            )
            .await?;
            client.disconnect().await?;

            // Wait longer than the expiry
            tokio::time::sleep(Duration::from_secs(3)).await;

            // Reconnect with clean_start = 0 — session should be expired
            let resumed = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                &cid,
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
            let session_present = resumed.connack().session_present;
            resumed.disconnect().await?;

            if session_present {
                Err(anyhow::anyhow!("session present = 1 after session expiry interval elapsed"))
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
