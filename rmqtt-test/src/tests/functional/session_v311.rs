//! MQTT 3.1.1 Session management tests

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Open a raw v3.1.1 connection with clean_session = false and consume the
/// CONNACK (must be 0x00). Returns the stream.
fn raw_connect_clean0(broker_addr: &str, client_id: &str) -> anyhow::Result<TcpStream> {
    let mut stream = TcpStream::connect(broker_addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;

    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&[0x00, 0x04]);
    body.extend_from_slice(b"MQTT");
    body.push(4);
    body.push(0x00); // clean session = false
    body.extend_from_slice(&[0x00, 0x3C]);
    let cid = client_id.as_bytes();
    body.extend_from_slice(&(cid.len() as u16).to_be_bytes());
    body.extend_from_slice(cid);

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

    let mut buf = [0u8; 4];
    let n = stream.read(&mut buf)?;
    if n < 4 || buf[0] != 0x20 || buf[3] != 0 {
        return Err(anyhow::anyhow!("CONNECT refused: {:02x?}", &buf[..n]));
    }
    Ok(stream)
}

/// Subscribe (QoS 0) over the raw stream and consume the SUBACK.
fn raw_subscribe(stream: &mut TcpStream, topic: &str) -> anyhow::Result<()> {
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&[0x00, 0x01]); // packet id 1
    body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
    body.extend_from_slice(topic.as_bytes());
    body.push(0x00); // requested QoS 0

    let mut pkt = vec![0x82];
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

    let mut buf = [0u8; 8];
    let n = stream.read(&mut buf)?;
    if n < 5 || buf[0] != 0x90 {
        return Err(anyhow::anyhow!("expected SUBACK, got {:02x?}", &buf[..n]));
    }
    Ok(())
}

/// Test session persistence with clean_session=false (v3.1.1)
pub struct CleanSessionFalseTest;

impl TestCase for CleanSessionFalseTest {
    fn name(&self) -> &str {
        "clean_session_false_v311"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let topic = "test/v311/session/persist";
            let payload = b"queued_msg";

            // Phase 1: Connect with clean_session=false and subscribe
            let mut client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "session-v311-client",
                ctx.config.connect_timeout,
                false, // clean_session = false
                60,
                None,
                None,
                None,
            )
            .await?;

            client.subscribe(topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Clean disconnect (session should persist)
            client.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 2: Publish while client is disconnected
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "session-v311-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            publisher.publish(topic, payload, QoS::AtLeastOnce, false).await?;
            publisher.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 3: Reconnect with same client_id + clean_session=false
            let mut reconnected = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "session-v311-client",
                ctx.config.connect_timeout,
                false,
                60,
                None,
                None,
                None,
            )
            .await?;

            // Should receive the queued message
            let msg = reconnected.recv_message_timeout(Duration::from_secs(5)).await;
            reconnected.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == payload => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected queued msg: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("no queued message received after reconnect")),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// Test offline message queue - multiple messages delivered in order (v3.1.1)
pub struct OfflineQueueV311Test;

impl TestCase for OfflineQueueV311Test {
    fn name(&self) -> &str {
        "offline_queue_v311"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let topic = "test/v311/session/queue";

            // Phase 1: Connect with clean_session=false and subscribe
            let mut client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "queue-v311-client",
                ctx.config.connect_timeout,
                false,
                60,
                None,
                None,
                None,
            )
            .await?;

            client.subscribe(topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Clean disconnect
            client.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 2: Publish 10 messages while client is offline
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "queue-v311-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            for i in 0..10 {
                let payload = format!("msg{}", i);
                publisher.publish(topic, payload.as_bytes(), QoS::AtLeastOnce, false).await?;
            }
            publisher.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 3: Reconnect and verify all 10 messages arrive in order
            let mut reconnected = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                "queue-v311-client",
                ctx.config.connect_timeout,
                false,
                60,
                None,
                None,
                None,
            )
            .await?;

            let mut received = Vec::new();
            for _ in 0..10 {
                match reconnected.recv_message_timeout(Duration::from_secs(5)).await {
                    Some(msg) => received.push(msg),
                    None => break,
                }
            }
            reconnected.disconnect().await?;

            if received.len() < 10 {
                return Err(anyhow::anyhow!("expected 10 queued messages, got {}", received.len()));
            }

            // Verify ordering
            for (i, msg) in received.iter().enumerate() {
                let expected = format!("msg{}", i);
                if msg.payload.as_ref() != expected.as_bytes() {
                    return Err(anyhow::anyhow!(
                        "message {} mismatch: expected {:?}, got {:?}",
                        i,
                        expected,
                        msg.payload
                    ));
                }
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

/// Positive: Session Present = 1 when reconnecting with clean_session = 0 and
/// an existing stored session. [MQTT-3.2.2.1]
pub struct SessionV311PresentOnResumeTest;

impl TestCase for SessionV311PresentOnResumeTest {
    fn name(&self) -> &str {
        "session_v311_present_on_resume"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let cid = format!("session-present-{uid}");

            // Phase 1: clean_session = 0, subscribe (creates a stored session)
            let mut client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                &cid,
                ctx.config.connect_timeout,
                false,
                60,
                None,
                None,
                None,
            )
            .await?;
            client.subscribe(&format!("test/v311/present/{uid}"), QoS::AtLeastOnce).await?;
            client.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 2: reconnect with clean_session = 0 — session present = 1
            let resumed = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                &cid,
                ctx.config.connect_timeout,
                false,
                60,
                None,
                None,
                None,
            )
            .await?;
            let session_present = resumed.connack().session_present;
            resumed.disconnect().await?;

            if session_present {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "session present must be 1 when resuming a stored session [MQTT-3.2.2.1]"
                ))
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// Negative: clean_session = 1 discards all stored session state; reconnecting
/// with the same client id and clean_session = 1 must NOT receive messages
/// queued while offline, and session present = 0. [MQTT-3.1.2-6]
pub struct SessionV311CleanDiscardTest;

impl TestCase for SessionV311CleanDiscardTest {
    fn name(&self) -> &str {
        "session_v311_clean_discard"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let cid = format!("session-clean-discard-{uid}");
            let topic = format!("test/v311/session/clean/{uid}");

            // Phase 1: clean_session = false, subscribe
            let mut client = crate::mqtt::v311::MqttV311Client::connect_with_options(
                &ctx.config.broker_addr,
                &cid,
                ctx.config.connect_timeout,
                false,
                60,
                None,
                None,
                None,
            )
            .await?;
            client.subscribe(&topic, QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;
            client.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 2: publish while offline
            let publisher = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &format!("session-clean-pub-{uid}"),
                ctx.config.connect_timeout,
            )
            .await?;
            publisher.publish(&topic, b"should_not_be_queued", QoS::AtLeastOnce, false).await?;
            publisher.disconnect().await?;
            tokio::time::sleep(Duration::from_millis(500)).await;

            // Phase 3: reconnect with clean_session = true — state must be gone
            let mut reconnected = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &cid,
                ctx.config.connect_timeout,
            )
            .await?;
            let session_present = reconnected.connack().session_present;
            let msg = reconnected.recv_message_timeout(Duration::from_secs(2)).await;
            reconnected.disconnect().await?;

            if session_present {
                return Err(anyhow::anyhow!(
                    "session present must be 0 after clean_session = 1 disconnect [MQTT-3.1.2-6]"
                ));
            }
            if msg.is_some() {
                return Err(anyhow::anyhow!(
                    "clean_session = 1 client received a message queued while offline"
                ));
            }
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

// ---------------------------------------------------------------------------
// P1 conformance gap fill (G14): concurrent session takeover
// ---------------------------------------------------------------------------

/// Positive: a second connection using the same Client Identifier takes over
/// the session — the old connection is disconnected by the broker.
/// [MQTT-3.1.4] (mirrors `session_takeover_v5` for v3.1.1)
pub struct SessionV311TakeoverTest;

impl TestCase for SessionV311TakeoverTest {
    fn name(&self) -> &str {
        "session_v311_takeover"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let cid = format!("v311-takeover-{uid}");

            // First connection
            let client1 = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &cid,
                ctx.config.connect_timeout,
            )
            .await?;
            assert!(client1.is_connected());

            // Second connection with SAME client ID should take over
            let client2 = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                &cid,
                ctx.config.connect_timeout,
            )
            .await?;
            assert!(client2.is_connected());

            // First client must have been disconnected by the broker
            tokio::time::sleep(Duration::from_millis(200)).await;
            if client1.is_connected() {
                return Err(anyhow::anyhow!(
                    "client1 should have been taken over by the second connection [MQTT-3.1.4]"
                ));
            }

            let _ = client1.disconnect().await;
            client2.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// An empty ClientId with Clean Session = 1 is accepted, and the broker
/// assigns each connection a distinct, unique client id: two concurrent
/// connections with an empty ClientId must coexist without session takeover.
/// [MQTT-3.1.3-6]
///
/// v3.1.1 CONNACK carries no assigned-client-id field, so uniqueness is
/// verified indirectly: if the broker assigned the same id, the second
/// connection would take over the first (disconnecting it) and the first
/// would not receive a message published by the second.
pub struct ConnectV311AssignedClientIdTest;

impl TestCase for ConnectV311AssignedClientIdTest {
    fn name(&self) -> &str {
        "connect_v311_assigned_client_id"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let topic = format!("test/assigned/{uid}");

            // Both clients connect with an empty client id + clean session.
            let mut subscriber = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "",
                ctx.config.connect_timeout,
            )
            .await?;
            subscriber.subscribe(&topic, QoS::AtMostOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            let client_b = crate::mqtt::v311::MqttV311Client::connect(
                &ctx.config.broker_addr,
                "",
                ctx.config.connect_timeout,
            )
            .await?;

            // A must still be connected (not taken over by B).
            if !subscriber.is_connected() {
                return Err(anyhow::anyhow!(
                    "first empty-client-id connection was taken over by the second — \
                     the broker assigned the same client id [MQTT-3.1.3-6]"
                ));
            }

            // A message published by B must be delivered to A.
            client_b.publish(&topic, b"assigned-ok", QoS::AtMostOnce, false).await?;
            let msg = subscriber.recv_message_timeout(Duration::from_secs(3)).await.ok_or_else(|| {
                anyhow::anyhow!("no message received from the other empty-client-id client")
            })?;
            if msg.payload.as_ref() != b"assigned-ok" {
                return Err(anyhow::anyhow!("unexpected payload: {:?}", msg.payload));
            }

            let _ = subscriber.disconnect().await;
            client_b.disconnect().await?;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// A persistent session (clean_session = false) must survive both TCP
/// disconnect styles: a graceful FIN and an aborted RST, and queued offline
/// messages must be delivered on reconnect. [test spec §12, G32]
///
/// - FIN: the client shuts the socket down without DISCONNECT.
/// - RST: the socket is dropped with SO_LINGER = 0, forcing an RST.
pub struct SessionV311TcpFinRstTest;

impl TestCase for SessionV311TcpFinRstTest {
    fn name(&self) -> &str {
        "session_v311_tcp_fin_rst"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result =
            rt.block_on(async {
                let topic = format!("test/session/finrst/{}", uuid::Uuid::new_v4().simple());
                let fin_payload = b"after-fin";
                let rst_payload = b"after-rst";

                // --- Scenario A: graceful FIN (shutdown without DISCONNECT) ---
                {
                    let fin_cid = "fin-cid";
                    let mut a = crate::mqtt::v311::MqttV311Client::connect_with_options(
                        &ctx.config.broker_addr,
                        fin_cid,
                        ctx.config.connect_timeout,
                        false, // clean_session = false
                        60,
                        None,
                        None,
                        None,
                    )
                    .await?;
                    a.subscribe(&topic, QoS::AtMostOnce).await?;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    // Graceful close: FIN without a DISCONNECT packet.
                    a.abort_connection().await?;
                    tokio::time::sleep(Duration::from_millis(200)).await;

                    let p = crate::mqtt::v311::MqttV311Client::connect(
                        &ctx.config.broker_addr,
                        "fin-pub",
                        ctx.config.connect_timeout,
                    )
                    .await?;
                    p.publish(&topic, fin_payload, QoS::AtMostOnce, false).await?;
                    p.disconnect().await?;

                    // Reconnect with the same id: the session must be restored
                    // and the offline message delivered.
                    let mut a2 = crate::mqtt::v311::MqttV311Client::connect_with_options(
                        &ctx.config.broker_addr,
                        fin_cid,
                        ctx.config.connect_timeout,
                        false,
                        60,
                        None,
                        None,
                        None,
                    )
                    .await?;
                    let msg = a2.recv_message_timeout(Duration::from_secs(3)).await.ok_or_else(|| {
                        anyhow::anyhow!("FIN: offline message not delivered after reconnect")
                    })?;
                    if msg.payload.as_ref() != fin_payload {
                        return Err(anyhow::anyhow!("FIN: payload mismatch: {:?}", msg.payload));
                    }
                    let _ = a2.disconnect().await;
                }

                // --- Scenario B: RST (SO_LINGER = 0, dropped without DISCONNECT) ---
                {
                    let rst_cid = "rst-cid";
                    // Local 127.0.0.1 connection: blocking briefly is fine.
                    let mut raw = raw_connect_clean0(&ctx.config.broker_addr, rst_cid)?;
                    raw_subscribe(&mut raw, &topic)?;
                    // Force an RST instead of FIN (socket2's set_linger is
                    // stable, unlike std's which is still feature-gated).
                    let sock = socket2::SockRef::from(&raw);
                    sock.set_linger(Some(Duration::ZERO))?;
                    drop(raw); // RST sent
                    tokio::time::sleep(Duration::from_millis(200)).await;

                    let p = crate::mqtt::v311::MqttV311Client::connect(
                        &ctx.config.broker_addr,
                        "rst-pub",
                        ctx.config.connect_timeout,
                    )
                    .await?;
                    p.publish(&topic, rst_payload, QoS::AtMostOnce, false).await?;
                    p.disconnect().await?;

                    let mut a2 = crate::mqtt::v311::MqttV311Client::connect_with_options(
                        &ctx.config.broker_addr,
                        rst_cid,
                        ctx.config.connect_timeout,
                        false,
                        60,
                        None,
                        None,
                        None,
                    )
                    .await?;
                    let msg = a2.recv_message_timeout(Duration::from_secs(3)).await.ok_or_else(|| {
                        anyhow::anyhow!("RST: offline message not delivered after reconnect")
                    })?;
                    if msg.payload.as_ref() != rst_payload {
                        return Err(anyhow::anyhow!("RST: payload mismatch: {:?}", msg.payload));
                    }
                    let _ = a2.disconnect().await;
                }

                Ok::<(), anyhow::Error>(())
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
