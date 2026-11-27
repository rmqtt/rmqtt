//! MQTT v3.1.1 protocol error handling tests
//!
//! Covers malformed / illegal packets (spec section 2.2-2.3, 3.8, 3.10):
//! - SUBSCRIBE with requested QoS 3 (both QoS bits set) [MQTT-3.8.3-4]
//! - SUBSCRIBE with fixed header QoS != 1 [MQTT-3.8.1-1]
//! - UNSUBSCRIBE with fixed header QoS != 1 [MQTT-3.10.1-1]
//! - PUBLISH with QoS = 3 (illegal QoS encoding)
//! - PUBLISH with QoS 1 and packet id 0 [MQTT-2.3.1-1]
//! - remaining length encoded in more than 4 bytes
//! - reserved packet type 0x00
//!
//! These tests craft raw packets (the codec rejects them before they reach
//! the wire) and assert the broker closes the connection or errors out.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Build a raw MQTT v3.1.1 CONNECT ("MQTT" / level 4) and return the bytes.
fn raw_connect_packet(client_id: &str) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&[0x00, 0x04]);
    body.extend_from_slice(b"MQTT");
    body.push(4); // level
    body.push(0x02); // clean session
    body.extend_from_slice(&[0x00, 0x3C]); // keep alive 60
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
    pkt
}

/// Open a raw TCP connection, send a valid v3.1.1 CONNECT, consume the
/// CONNACK. Returns the stream.
fn raw_connect(broker_addr: &str, client_id: &str) -> anyhow::Result<TcpStream> {
    let mut stream = TcpStream::connect(broker_addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let pkt = raw_connect_packet(client_id);
    stream.write_all(&pkt)?;
    stream.flush()?;
    let mut buf = [0u8; 8];
    let n = stream.read(&mut buf)?;
    if n < 4 || buf[0] != 0x20 || buf[3] != 0 {
        return Err(anyhow::anyhow!("CONNECT refused: {:02x?}", &buf[..n]));
    }
    Ok(stream)
}

/// Send bytes and check whether the broker closed the connection.
///
/// Returns `true` only on a real close (EOF or connection reset). A read
/// timeout (`WouldBlock` / `TimedOut`) means the connection is still open —
/// that is NOT treated as closed, so a lenient broker cannot produce a false
/// positive.
fn expect_connection_closed(stream: &mut TcpStream, data: &[u8]) -> bool {
    let _ = stream.write_all(data);
    let _ = stream.flush();
    let mut buf = [0u8; 16];
    match stream.read(&mut buf) {
        Ok(0) => true,                                                 // EOF: closed
        Ok(_) => false,                                                // data received: still open
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => false, // timeout: still open
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => false,   // timeout: still open
        Err(_) => true, // connection reset / other error: closed
    }
}

/// Generic protocol-error test body: connect, send a malformed packet,
/// assert the broker closes the connection.
fn run_protocol_error(
    name: &str,
    ctx: &TestContext,
    start: Instant,
    malformed: impl Fn(&mut TcpStream) -> anyhow::Result<()>,
) -> TestResult {
    let uid = uuid::Uuid::new_v4().simple().to_string();
    let result = raw_connect(&ctx.config.broker_addr, &format!("perr-{uid}"))
        .and_then(|mut stream| malformed(&mut stream));

    match result {
        Ok(()) => TestResult::passed(name, "functional_v311", start.elapsed()),
        Err(e) => TestResult::failed(name, "functional_v311", start.elapsed(), e.to_string()),
    }
}

/// Negative: SUBSCRIBE with requested QoS 3 is a protocol error. [MQTT-3.8.3-4]
pub struct ProtocolErrorV311SubscribeQos3Test;

impl TestCase for ProtocolErrorV311SubscribeQos3Test {
    fn name(&self) -> &str {
        "protocol_error_v311_subscribe_qos3"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/qos3";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x01]); // packet id 1
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);
            body.push(0x03); // requested QoS 3 — illegal

            let mut pkt = vec![0x82]; // SUBSCRIBE, QoS 1 fixed header
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

            if expect_connection_closed(stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close for QoS 3 subscribe [MQTT-3.8.3-4]"))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: SUBSCRIBE with fixed header QoS bits = 0 is a protocol error.
/// [MQTT-3.8.1-1]
pub struct ProtocolErrorV311SubscribeQos0FixedHeaderTest;

impl TestCase for ProtocolErrorV311SubscribeQos0FixedHeaderTest {
    fn name(&self) -> &str {
        "protocol_error_v311_subscribe_qos0_fixed_header"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/subqos0";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x01]); // packet id
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);
            body.push(0x00); // requested QoS 0

            let mut pkt = vec![0x80]; // SUBSCRIBE with QoS bits = 0 — illegal
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

            if expect_connection_closed(stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close for SUBSCRIBE QoS 0 fixed header [MQTT-3.8.1-1]"))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: UNSUBSCRIBE with fixed header QoS bits = 0 is a protocol error.
/// [MQTT-3.10.1-1]
pub struct ProtocolErrorV311UnsubscribeQos0FixedHeaderTest;

impl TestCase for ProtocolErrorV311UnsubscribeQos0FixedHeaderTest {
    fn name(&self) -> &str {
        "protocol_error_v311_unsubscribe_qos0_fixed_header"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/unsubqos0";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x01]); // packet id
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);

            let mut pkt = vec![0xA0]; // UNSUBSCRIBE with QoS bits = 0 — illegal
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

            if expect_connection_closed(stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "broker did not close for UNSUBSCRIBE QoS 0 fixed header [MQTT-3.10.1-1]"
                ))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: PUBLISH with QoS bits = 3 (illegal QoS value) must close the
/// connection. [MQTT-2.2.2-2]
pub struct ProtocolErrorV311PublishQos3Test;

impl TestCase for ProtocolErrorV311PublishQos3Test {
    fn name(&self) -> &str {
        "protocol_error_v311_publish_qos3"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/qos3pub";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);
            body.extend_from_slice(&[0x00, 0x01]); // packet id
            body.extend_from_slice(b"payload");

            // fixed header 0x36: PUBLISH, QoS bits = 3 (0b11 << 1)
            let mut pkt = vec![0x36];
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

            if expect_connection_closed(stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close for PUBLISH QoS 3"))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: PUBLISH QoS 1 with packet identifier 0 is invalid. [MQTT-2.3.1-1]
pub struct ProtocolErrorV311PublishPacketIdZeroTest;

impl TestCase for ProtocolErrorV311PublishPacketIdZeroTest {
    fn name(&self) -> &str {
        "protocol_error_v311_publish_packet_id_zero"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/pid0";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);
            body.extend_from_slice(&[0x00, 0x00]); // packet id 0 — illegal
            body.extend_from_slice(b"payload");

            let mut pkt = vec![0x32]; // PUBLISH QoS 1
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

            if expect_connection_closed(stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close for PUBLISH packet id 0"))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: a remaining length encoded in 5 bytes is invalid (max 4).
pub struct ProtocolErrorV311BadRemainingLengthTest;

impl TestCase for ProtocolErrorV311BadRemainingLengthTest {
    fn name(&self) -> &str {
        "protocol_error_v311_bad_remaining_length"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            // PINGREQ with a 5-byte remaining length
            let pkt = [0xC0u8, 0x80, 0x80, 0x80, 0x80, 0x01];
            if expect_connection_closed(stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close for 5-byte remaining length"))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: a packet with reserved packet type 0x00 must close the connection.
pub struct ProtocolErrorV311ReservedPacketTypeTest;

impl TestCase for ProtocolErrorV311ReservedPacketTypeTest {
    fn name(&self) -> &str {
        "protocol_error_v311_reserved_packet_type"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let pkt = [0x00u8, 0x00];
            if expect_connection_closed(stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close for reserved packet type 0x00"))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

// ---------------------------------------------------------------------------
// P0 conformance gap fill (G1 / G7 / G8 / G9)
// ---------------------------------------------------------------------------

/// Negative: a PUBLISH whose Topic Name contains invalid UTF-8 must close the
/// connection. [MQTT-1.5.3-1]
pub struct ProtocolErrorV311InvalidUtf8TopicTest;

impl TestCase for ProtocolErrorV311InvalidUtf8TopicTest {
    fn name(&self) -> &str {
        "protocol_error_v311_invalid_utf8_topic"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            // overlong, surrogate half, truncated multi-byte, embedded U+0000
            let bad_topics: [&[u8]; 4] = [&[0xC0, 0x80], &[0xED, 0xA0, 0x80], &[0xE2, 0x82], &[b'a', 0x00]];
            for bad in bad_topics {
                let mut body: Vec<u8> = Vec::new();
                body.extend_from_slice(&(bad.len() as u16).to_be_bytes());
                body.extend_from_slice(bad);
                body.extend_from_slice(b"payload");

                let mut pkt = vec![0x30]; // PUBLISH QoS 0
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

                if !expect_connection_closed(stream, &pkt) {
                    return Err(anyhow::anyhow!(
                        "broker did not close for PUBLISH with invalid UTF-8 topic {:02x?}",
                        bad
                    ));
                }
            }
            Ok(())
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: PUBREL with fixed header flags != 0010 must close the
/// connection. [MQTT-3.6.1-1]
pub struct ProtocolErrorV311PubrelWrongFlagsTest;

impl TestCase for ProtocolErrorV311PubrelWrongFlagsTest {
    fn name(&self) -> &str {
        "protocol_error_v311_pubrel_wrong_flags"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            // PUBREL type = 6, correct fixed header is 0x62 (flags 0010);
            // 0x60 (flags 0000) is a protocol violation [MQTT-3.6.1-1].
            let pkt = [0x60u8, 0x02, 0x00, 0x01];
            if expect_connection_closed(stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close for PUBREL flags != 0010 [MQTT-3.6.1-1]"))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: PUBREC / PUBCOMP with non-zero fixed header flags must close
/// the connection. [MQTT-3.5.1-1] [MQTT-3.7.1-1]
pub struct ProtocolErrorV311PubrecPubcompWrongFlagsTest;

impl TestCase for ProtocolErrorV311PubrecPubcompWrongFlagsTest {
    fn name(&self) -> &str {
        "protocol_error_v311_pubrec_pubcomp_wrong_flags"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            // PUBREC type = 5: correct header is 0x50 (flags 0000); 0x52 illegal.
            let pubrec = [0x52u8, 0x02, 0x00, 0x01];
            if !expect_connection_closed(stream, &pubrec) {
                return Err(anyhow::anyhow!("broker did not close for PUBREC flags != 0000 [MQTT-3.5.1-1]"));
            }

            // Reconnect for the second sub-case (the connection is now closed).
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let mut s2 = raw_connect(&ctx.config.broker_addr, &format!("perr-flags-{uid}"))?;
            // PUBCOMP type = 7: correct header is 0x70 (flags 0000); 0x72 illegal.
            let pubcomp = [0x72u8, 0x02, 0x00, 0x01];
            if expect_connection_closed(&mut s2, &pubcomp) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close for PUBCOMP flags != 0000 [MQTT-3.7.1-1]"))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// QoS 0 PUBLISH must not carry a packet identifier: the bytes following the
/// topic name belong to the payload. [MQTT-2.3.1-5]
///
/// Sends a raw QoS 0 PUBLISH whose body looks like "topic + packet id +
/// payload" and asserts the broker treats the 2 extra bytes as payload data
/// rather than stripping them as a packet identifier.
pub struct ProtocolErrorV311Qos0PublishWithPacketIdTest;

impl TestCase for ProtocolErrorV311Qos0PublishWithPacketIdTest {
    fn name(&self) -> &str {
        "protocol_error_v311_qos0_publish_with_packet_id"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            use crate::mqtt::common::QoS;
            use crate::mqtt::v311::MqttV311Client;

            let mut subscriber =
                MqttV311Client::connect(&ctx.config.broker_addr, "qos0-pid-sub", ctx.config.connect_timeout)
                    .await?;
            subscriber.subscribe("qos0/pid", QoS::AtMostOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Raw publisher: connect, then send a QoS 0 PUBLISH whose body is
            // topic + 2-byte "packet id" + payload.
            let mut stream = TcpStream::connect(&ctx.config.broker_addr)?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            let connect_pkt = raw_connect_packet("qos0-pid-pub");
            stream.write_all(&connect_pkt)?;
            stream.flush()?;
            let mut buf = [0u8; 8];
            let n = stream.read(&mut buf)?;
            if n < 4 || buf[0] != 0x20 || buf[3] != 0 {
                return Err(anyhow::anyhow!("raw publisher CONNECT failed: {:02x?}", &buf[..n]));
            }

            let topic = b"qos0/pid";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);
            body.extend_from_slice(&[0x00, 0x01]); // would-be packet id
            body.extend_from_slice(b"hello");

            let mut pkt = vec![0x30]; // PUBLISH QoS 0
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

            let msg = subscriber.recv_message_timeout(Duration::from_secs(5)).await;
            let _ = stream.shutdown(std::net::Shutdown::Both);
            let _ = subscriber.disconnect().await;

            let full: &[u8] = &[0x00, 0x01, b'h', b'e', b'l', b'l', b'o'];
            match msg {
                Some(m) if m.payload.as_ref() == full => Ok(()),
                Some(m) if m.payload.as_ref() == b"hello" => Err(anyhow::anyhow!(
                    "broker stripped the 2 trailing bytes as a QoS 0 packet id [MQTT-2.3.1-5]"
                )),
                Some(m) => Err(anyhow::anyhow!("unexpected payload: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("no QoS 0 PUBLISH delivered")),
            }
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

/// Negative: SUBSCRIBE with packet identifier 0 must close the connection.
/// [MQTT-2.3.1-1]
pub struct ProtocolErrorV311SubscribePacketIdZeroTest;

impl TestCase for ProtocolErrorV311SubscribePacketIdZeroTest {
    fn name(&self) -> &str {
        "protocol_error_v311_subscribe_packet_id_zero"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/pid0sub";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x00]); // packet id 0 — illegal
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);
            body.push(0x00); // requested QoS 0

            let mut pkt = vec![0x82]; // SUBSCRIBE
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

            if expect_connection_closed(stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close for SUBSCRIBE packet id 0 [MQTT-2.3.1-1]"))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: UNSUBSCRIBE with packet identifier 0 must close the connection.
/// [MQTT-2.3.1-1]
pub struct ProtocolErrorV311UnsubscribePacketIdZeroTest;

impl TestCase for ProtocolErrorV311UnsubscribePacketIdZeroTest {
    fn name(&self) -> &str {
        "protocol_error_v311_unsubscribe_packet_id_zero"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/pid0unsub";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x00]); // packet id 0 — illegal
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);

            let mut pkt = vec![0xA2]; // UNSUBSCRIBE
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

            if expect_connection_closed(stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close for UNSUBSCRIBE packet id 0 [MQTT-2.3.1-1]"))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

// ---------------------------------------------------------------------------
// P1 conformance gap fill (G11 / G12 / G17 / G18 / G21)
// ---------------------------------------------------------------------------

/// Negative: a packet whose declared remaining length exceeds the bytes
/// actually sent (a truncated / half-sent packet) must not cause the broker
/// to mishandle the connection. The broker keeps waiting for the missing
/// bytes — it must NOT crash or close the connection prematurely. [MQTT-2.2.3]
pub struct ProtocolErrorV311TruncatedPacketTest;

impl TestCase for ProtocolErrorV311TruncatedPacketTest {
    fn name(&self) -> &str {
        "protocol_error_v311_truncated_packet"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            // PUBLISH QoS 0 declaring 20 remaining bytes, but send only 8
            // (a 2-byte topic + 6-byte partial payload).
            let topic = b"t";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);
            body.extend_from_slice(&[0x41; 6]); // only 6 of the 18 payload bytes
            let mut pkt = vec![0x30];
            // remaining length claims 2 + 1 + 17 = 20 bytes
            let declared = 20u8;
            pkt.push(declared);
            pkt.extend_from_slice(&body);
            // body is 2+1+6 = 9 bytes < declared 20 → truncated
            debug_assert!(body.len() < declared as usize);

            stream.write_all(&pkt)?;
            stream.flush()?;

            // The broker must keep the connection open waiting for the
            // remaining bytes (read timeout), not close it.
            let mut buf = [0u8; 16];
            match stream.read(&mut buf) {
                Ok(0) => Err(anyhow::anyhow!(
                    "broker closed the connection on a truncated packet (declared > sent) [MQTT-2.2.3]"
                )),
                Ok(n) => Err(anyhow::anyhow!(
                    "unexpected data while waiting for the truncated packet body: {:02x?}",
                    &buf[..n]
                )),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(()),
                Err(e) => Err(anyhow::anyhow!("read error: {e}")),
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Negative: a PUBLISH whose declared remaining length is *less* than the
/// bytes actually sent — the surplus bytes are parsed as a separate (garbage)
/// packet and must lead to a protocol error / connection close, never a crash.
/// [MQTT-2.2.3] [MQTT-4.8.0-1]
pub struct ProtocolErrorV311DeclaredLengthMismatchTest;

impl TestCase for ProtocolErrorV311DeclaredLengthMismatchTest {
    fn name(&self) -> &str {
        "protocol_error_v311_declared_length_mismatch"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            // PUBLISH QoS 0 declaring remaining length 5, but send 8 bytes:
            // the 3 surplus bytes form a reserved packet type 0x00 packet,
            // which the broker must reject by closing the connection.
            let topic = b"t";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);
            body.extend_from_slice(b"ab"); // 2 bytes of "payload"

            let mut pkt = vec![0x30];
            let declared = 5u8; // 2 (topic len) + 1 (topic) + 2 (payload) = 5, but…
            pkt.push(declared);
            pkt.extend_from_slice(&body);
            // …we append a surplus garbage packet (reserved type 0x00)
            pkt.extend_from_slice(&[0x00, 0x00]);
            // total sent = 5 declared + 2 surplus = 7 bytes > declared

            if expect_connection_closed(stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "broker did not close for surplus bytes after the declared length [MQTT-2.2.3]"
                ))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: a packet with reserved packet type 0x0F (15) must close the
/// connection. [MQTT-2.2.1] (only type 0x00 was covered before)
pub struct ProtocolErrorV311PacketType15Test;

impl TestCase for ProtocolErrorV311PacketType15Test {
    fn name(&self) -> &str {
        "protocol_error_v311_packet_type_15"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let pkt = [0xF0u8, 0x00];
            if expect_connection_closed(stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close for reserved packet type 0x0F"))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: a PUBLISH with an empty (zero-length) Topic Name must close the
/// connection. [MQTT-4.7.3-1]
///
/// The codec rejected this case with `DecodeError::InvalidTopicName`
/// (2026-08-23); previously the broker accepted the empty topic and kept
/// the connection open.
pub struct ProtocolErrorV311PublishEmptyTopicTest;

impl TestCase for ProtocolErrorV311PublishEmptyTopicTest {
    fn name(&self) -> &str {
        "protocol_error_v311_publish_empty_topic"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x00]); // topic name length 0
            body.extend_from_slice(b"payload");

            let mut pkt = vec![0x30]; // PUBLISH QoS 0
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

            if expect_connection_closed(stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close for PUBLISH with empty topic [MQTT-4.7.3-1]"))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: an UNSUBSCRIBE with an empty payload (no Topic Filters) is a
/// protocol error and must close the connection. [MQTT-3.10.3-1]
///
/// The codec rejected this case with `DecodeError::InvalidTopicFilter`
/// (2026-08-23); previously the broker kept the connection open.
pub struct ProtocolErrorV311UnsubscribeEmptyTest;

impl TestCase for ProtocolErrorV311UnsubscribeEmptyTest {
    fn name(&self) -> &str {
        "protocol_error_v311_unsubscribe_empty"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            // UNSUBSCRIBE with a packet id but zero topic filters
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x01]); // packet id

            let mut pkt = vec![0xA2]; // UNSUBSCRIBE
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

            if expect_connection_closed(stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "broker did not close for UNSUBSCRIBE with no topic filters [MQTT-3.10.3-1]"
                ))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// QoS 2 out-of-order: a PUBREL for a packet identifier with no in-progress
/// QoS 2 exchange must not crash the broker. Acceptable conformant outcomes
/// (MQTT-4.3.3 method B): complete the exchange with a PUBCOMP, ignore and
/// stay open, or close the connection.
pub struct ProtocolErrorV311UnsolicitedPubrelTest;

impl TestCase for ProtocolErrorV311UnsolicitedPubrelTest {
    fn name(&self) -> &str {
        "protocol_error_v311_unsolicited_pubrel"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            // PUBREL (0x62) with packet id 1 — no PUBLISH/PUBREC preceded it
            let pkt = [0x62u8, 0x02, 0x00, 0x01];
            stream.write_all(&pkt)?;
            stream.flush()?;

            let mut buf = [0u8; 16];
            match stream.read(&mut buf) {
                Ok(0) => Ok(()), // broker closed the connection — acceptable
                // PUBCOMP (0x70) for the same packet id: the broker completed
                // the (unknown) exchange — acceptable per MQTT-4.3.3 method B
                Ok(n) if n >= 1 && buf[0] == 0x70 => Ok(()),
                Ok(n) => Err(anyhow::anyhow!(
                    "unexpected data in reply to an unsolicited PUBREL: {:02x?}",
                    &buf[..n]
                )),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(()), // ignored, still open
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(()),
                Err(e) => Err(anyhow::anyhow!("read error: {e}")),
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

// ---------------------------------------------------------------------------
// Empty-topic-filter gap fill (2026-08-23): SUBSCRIBE/UNSUBSCRIBE with no
// topic filters or an empty-string filter must close the connection
// (MQTT-3.8.3-1 / MQTT-3.10.3-1 / MQTT-4.7.3-1). The codec now rejects all
// four variants with DecodeError::InvalidTopicFilter.
// ---------------------------------------------------------------------------

/// Negative: SUBSCRIBE with no topic filters (only a packet id) is a protocol
/// error and must close the connection. [MQTT-3.8.3-1]
///
/// Strict version of the former lenient `empty_topic_filter` test, which
/// accepted SUBACK and could never FAIL.
pub struct ProtocolErrorV311SubscribeEmptyTest;

impl TestCase for ProtocolErrorV311SubscribeEmptyTest {
    fn name(&self) -> &str {
        "protocol_error_v311_subscribe_empty"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            // SUBSCRIBE with a packet id but zero topic filters
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x01]); // packet id

            let mut pkt = vec![0x82]; // SUBSCRIBE
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

            if expect_connection_closed(stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "broker did not close for SUBSCRIBE with no topic filters [MQTT-3.8.3-1]"
                ))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: SUBSCRIBE with an empty-string topic filter is a protocol error
/// and must close the connection. [MQTT-4.7.3-1]
pub struct ProtocolErrorV311SubscribeEmptyFilterTest;

impl TestCase for ProtocolErrorV311SubscribeEmptyFilterTest {
    fn name(&self) -> &str {
        "protocol_error_v311_subscribe_empty_filter"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            // SUBSCRIBE with one empty-string topic filter (len 0x0000)
            let pkt = [0x82u8, 0x04, 0x00, 0x01, 0x00, 0x00];

            if expect_connection_closed(stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "broker did not close for SUBSCRIBE with an empty topic filter [MQTT-4.7.3-1]"
                ))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: UNSUBSCRIBE with an empty-string topic filter is a protocol error
/// and must close the connection. [MQTT-4.7.3-1]
pub struct ProtocolErrorV311UnsubscribeEmptyFilterTest;

impl TestCase for ProtocolErrorV311UnsubscribeEmptyFilterTest {
    fn name(&self) -> &str {
        "protocol_error_v311_unsubscribe_empty_filter"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            // UNSUBSCRIBE with one empty-string topic filter (len 0x0000)
            let pkt = [0xA2u8, 0x04, 0x00, 0x01, 0x00, 0x00];

            if expect_connection_closed(stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "broker did not close for UNSUBSCRIBE with an empty topic filter [MQTT-4.7.3-1]"
                ))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}
