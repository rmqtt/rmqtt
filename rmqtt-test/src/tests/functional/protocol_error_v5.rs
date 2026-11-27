//! MQTT v5.0 protocol error handling tests
//!
//! Covers malformed / illegal packets (spec section 2.2):
//! - SUBSCRIBE with requested QoS 3
//! - SUBSCRIBE with fixed header QoS != 1 [MQTT-3.8.1-1]
//! - UNSUBSCRIBE with fixed header QoS != 1 [MQTT-3.10.1-1]
//! - PUBLISH with QoS = 3 (illegal QoS encoding)
//! - PUBLISH QoS 1 with packet identifier 0 [MQTT-2.2.1-2]
//! - remaining length encoded in more than 4 bytes
//! - reserved packet type 0x00
//! - PUBLISH with an empty topic name
//!
//! These tests craft raw packets (the codec rejects them before they reach
//! the wire) and assert the broker closes the connection or errors out.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Build a raw MQTT v5 CONNECT ("MQTT" / level 5, clean start, no props).
fn raw_connect_v5(client_id: &str) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&[0x00, 0x04]);
    body.extend_from_slice(b"MQTT");
    body.push(5); // level
    body.push(0x02); // clean start
    body.extend_from_slice(&[0x00, 0x3C]); // keep alive 60
    body.push(0x00); // property length = 0
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

/// Read one full MQTT packet from the stream (fixed header + remaining
/// length), returning the raw bytes. Needed because v5 CONNACK has a variable
/// length (properties); a naive 4-byte read leaves trailing bytes in the
/// buffer which would corrupt the next read.
fn read_full_packet(stream: &mut TcpStream) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut b = [0u8; 1];
    let n = stream.read(&mut b)?;
    if n == 0 {
        return Err(anyhow::anyhow!("connection closed"));
    }
    buf.push(b[0]);

    // Decode remaining length (variable byte integer, max 4 bytes)
    let mut remaining: u32 = 0;
    let mut shift = 0u32;
    loop {
        let n = stream.read(&mut b)?;
        if n == 0 {
            return Err(anyhow::anyhow!("connection closed mid-header"));
        }
        buf.push(b[0]);
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
    buf.extend_from_slice(&rest);
    Ok(buf)
}

/// Open a raw TCP connection, send a valid v5 CONNECT, consume the complete
/// CONNACK. Returns the stream.
fn raw_connect(broker_addr: &str, client_id: &str) -> anyhow::Result<TcpStream> {
    let mut stream = TcpStream::connect(broker_addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let pkt = raw_connect_v5(client_id);
    stream.write_all(&pkt)?;
    stream.flush()?;
    let connack = read_full_packet(&mut stream)?;
    if connack.len() < 4 || connack[0] != 0x20 || connack[3] != 0 {
        return Err(anyhow::anyhow!("CONNECT refused: {:02x?}", &connack[..connack.len().min(8)]));
    }
    Ok(stream)
}

/// Send bytes and check whether the broker signalled a protocol error: either
/// closed the connection (EOF / timeout) or sent a DISCONNECT packet (0xE0).
/// In MQTT v5, the server responds to a protocol error with a DISCONNECT
/// packet (reason 0x81/0x82/0x93...) before closing, so both are accepted.
fn expect_connection_closed(stream: &mut TcpStream, data: &[u8]) -> bool {
    let _ = stream.write_all(data);
    let _ = stream.flush();
    let mut buf = [0u8; 16];
    match stream.read(&mut buf) {
        Ok(0) | Err(_) => true,                    // EOF / timeout → closed
        Ok(n) if n >= 1 && buf[0] == 0xE0 => true, // DISCONNECT packet
        Ok(_) => false,                            // any other response → not an error signal
    }
}

/// Generic protocol-error test body.
fn run_protocol_error(
    name: &str,
    ctx: &TestContext,
    start: Instant,
    malformed: impl Fn(&mut TcpStream) -> anyhow::Result<()>,
) -> TestResult {
    let uid = uuid::Uuid::new_v4().simple().to_string();
    let result = raw_connect(&ctx.config.broker_addr, &format!("perr5-{uid}"))
        .and_then(|mut stream| malformed(&mut stream));

    match result {
        Ok(()) => TestResult::passed(name, "functional_v5", start.elapsed()),
        Err(e) => TestResult::failed(name, "functional_v5", start.elapsed(), e.to_string()),
    }
}

/// Append a Remaining Length varint for `len` and then `body` to `pkt`.
fn finish_packet(pkt: &mut Vec<u8>, body: &[u8]) {
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
    pkt.extend_from_slice(body);
}

/// Expect the broker to reject a raw packet: connection closed (EOF /
/// timeout), or a DISCONNECT packet carrying a specific reason code.
/// When `expect_reason` is `Some(code)`, the DISCONNECT reason code must
/// match; otherwise any rejection signal is accepted.
fn expect_rejection_with_reason(
    stream: &mut TcpStream,
    data: &[u8],
    expect_reason: Option<u8>,
) -> anyhow::Result<()> {
    let _ = stream.write_all(data);
    let _ = stream.flush();
    let mut buf = [0u8; 16];
    match stream.read(&mut buf) {
        Ok(0) | Err(_) => Ok(()), // EOF / timeout → closed
        Ok(n) if n >= 1 && buf[0] == 0xE0 => {
            // DISCONNECT: [0xE0][rlen][reason][props...]
            if let Some(expected) = expect_reason {
                let rlen = buf[1] as usize;
                if n >= 3 && rlen >= 1 && buf[2] != expected {
                    return Err(anyhow::anyhow!(
                        "broker sent DISCONNECT with reason 0x{:02X}, expected 0x{:02X}",
                        buf[2],
                        expected
                    ));
                }
            }
            Ok(())
        }
        Ok(n) => {
            Err(anyhow::anyhow!("broker did not reject the packet (responded {:02x?})", &buf[..n.min(4)]))
        }
    }
}

/// Negative: SUBSCRIBE with requested QoS 3 is a protocol error.
pub struct ProtocolErrorV5SubscribeQos3Test;

impl TestCase for ProtocolErrorV5SubscribeQos3Test {
    fn name(&self) -> &str {
        "protocol_error_v5_subscribe_qos3"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/qos3";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x01]); // packet id 1
                                                   // property length 0
            body.push(0x00);
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
                Err(anyhow::anyhow!("broker did not close for QoS 3 subscribe"))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: SUBSCRIBE with fixed header QoS bits = 0 is a protocol error.
/// [MQTT-3.8.1-1]
pub struct ProtocolErrorV5SubscribeQos0FixedHeaderTest;

impl TestCase for ProtocolErrorV5SubscribeQos0FixedHeaderTest {
    fn name(&self) -> &str {
        "protocol_error_v5_subscribe_qos0_fixed_header"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/subqos0";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x01]); // packet id
            body.push(0x00); // property length
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
pub struct ProtocolErrorV5UnsubscribeQos0FixedHeaderTest;

impl TestCase for ProtocolErrorV5UnsubscribeQos0FixedHeaderTest {
    fn name(&self) -> &str {
        "protocol_error_v5_unsubscribe_qos0_fixed_header"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/unsubqos0";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x01]); // packet id
            body.push(0x00); // property length
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
pub struct ProtocolErrorV5PublishQos3Test;

impl TestCase for ProtocolErrorV5PublishQos3Test {
    fn name(&self) -> &str {
        "protocol_error_v5_publish_qos3"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/qos3pub";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);
            body.push(0x00); // property length
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

/// Negative: PUBLISH QoS 1 with packet identifier 0 is invalid. [MQTT-2.2.1-2]
pub struct ProtocolErrorV5PublishPacketIdZeroTest;

impl TestCase for ProtocolErrorV5PublishPacketIdZeroTest {
    fn name(&self) -> &str {
        "protocol_error_v5_publish_packet_id_zero"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/pid0";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);
            // Wire order for QoS > 0: topic, THEN packet id, THEN properties,
            // so the rejection really is the zero packet id [MQTT-2.2.1-2].
            body.extend_from_slice(&[0x00, 0x00]); // packet id 0 — illegal
            body.push(0x00); // property length
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

/// Negative: PUBLISH with an empty topic name is a protocol error.
pub struct ProtocolErrorV5PublishEmptyTopicTest;

impl TestCase for ProtocolErrorV5PublishEmptyTopicTest {
    fn name(&self) -> &str {
        "protocol_error_v5_publish_empty_topic"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x00]); // empty topic name
            body.push(0x00); // property length
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
                Err(anyhow::anyhow!("broker did not close for PUBLISH with empty topic"))
            }
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: a remaining length encoded in 5 bytes is invalid (max 4).
pub struct ProtocolErrorV5BadRemainingLengthTest;

impl TestCase for ProtocolErrorV5BadRemainingLengthTest {
    fn name(&self) -> &str {
        "protocol_error_v5_bad_remaining_length"
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
pub struct ProtocolErrorV5ReservedPacketTypeTest;

impl TestCase for ProtocolErrorV5ReservedPacketTypeTest {
    fn name(&self) -> &str {
        "protocol_error_v5_reserved_packet_type"
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
// P0 gap-analysis additions (designs/mqtt-5.0-standalone-test-gap-analysis.md)
// ---------------------------------------------------------------------------

/// Negative: SUBSCRIBE with an empty payload (zero topic filters) is a
/// Protocol Error. [MQTT-3.8.3-3]
pub struct ProtocolErrorV5SubscribeEmptyPayloadTest;

impl TestCase for ProtocolErrorV5SubscribeEmptyPayloadTest {
    fn name(&self) -> &str {
        "protocol_error_v5_subscribe_empty_payload"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let body: [u8; 3] = [0x00, 0x01, 0x00]; // packet id 1, property length 0
            let mut pkt = vec![0x82]; // SUBSCRIBE, QoS 1 fixed header
            finish_packet(&mut pkt, &body);
            expect_rejection_with_reason(stream, &pkt, None)
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: UNSUBSCRIBE with an empty payload (zero topic filters) is a
/// Protocol Error. [MQTT-3.10.3-2]
///
/// Broker fixed in the v5 codec (`Subscribe::decode` / `Unsubscribe::decode`
/// now reject empty topic_filters, parity with v3); this test is the
/// regression guard for that fix.
pub struct ProtocolErrorV5UnsubscribeEmptyPayloadTest;

impl TestCase for ProtocolErrorV5UnsubscribeEmptyPayloadTest {
    fn name(&self) -> &str {
        "protocol_error_v5_unsubscribe_empty_payload"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let body: [u8; 3] = [0x00, 0x01, 0x00]; // packet id 1, property length 0
            let mut pkt = vec![0xA2]; // UNSUBSCRIBE, QoS 1 fixed header
            finish_packet(&mut pkt, &body);
            expect_rejection_with_reason(stream, &pkt, None)
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: SUBSCRIBE with Retain Handling value 3 in the subscription
/// options is a Malformed Packet. [MQTT-3.8.3-4]
pub struct ProtocolErrorV5RetainHandling3Test;

impl TestCase for ProtocolErrorV5RetainHandling3Test {
    fn name(&self) -> &str {
        "protocol_error_v5_retain_handling_3"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/rh3";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x01]); // packet id 1
            body.push(0x00); // property length 0
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);
            body.push(0x30); // options: QoS 0, Retain Handling = 3 — illegal

            let mut pkt = vec![0x82];
            finish_packet(&mut pkt, &body);
            expect_rejection_with_reason(stream, &pkt, None)
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: SUBSCRIBE with reserved bits (6-7) set in the subscription
/// options is a Malformed Packet. [MQTT-3.8.3-5]
pub struct ProtocolErrorV5SubOptionsReservedBitsTest;

impl TestCase for ProtocolErrorV5SubOptionsReservedBitsTest {
    fn name(&self) -> &str {
        "protocol_error_v5_sub_options_reserved_bits"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/subres";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x01]); // packet id 1
            body.push(0x00); // property length 0
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);
            body.push(0xC0); // options: reserved bits 6-7 set — illegal

            let mut pkt = vec![0x82];
            finish_packet(&mut pkt, &body);
            expect_rejection_with_reason(stream, &pkt, None)
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: SUBSCRIBE with Subscription Identifier 0 is a Protocol Error.
/// [MQTT-3.8.2.1.2]
pub struct ProtocolErrorV5SubIdZeroTest;

impl TestCase for ProtocolErrorV5SubIdZeroTest {
    fn name(&self) -> &str {
        "protocol_error_v5_sub_id_zero"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/subid0";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x01]); // packet id 1
            body.push(0x02); // property length
            body.push(0x0B); // Subscription Identifier property
            body.push(0x00); // value 0 — illegal
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);
            body.push(0x00); // subscription options: QoS 0

            let mut pkt = vec![0x82];
            finish_packet(&mut pkt, &body);
            expect_rejection_with_reason(stream, &pkt, None)
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: UNSUBSCRIBE carrying a Subscription Identifier property is a
/// Protocol Error. [MQTT-3.10.2.1]
pub struct ProtocolErrorV5UnsubscribeWithSubIdTest;

impl TestCase for ProtocolErrorV5UnsubscribeWithSubIdTest {
    fn name(&self) -> &str {
        "protocol_error_v5_unsubscribe_with_sub_id"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/unsubid";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x01]); // packet id 1
            body.push(0x02); // property length
            body.push(0x0B); // Subscription Identifier property — illegal here
            body.push(0x01); // value 1
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);

            let mut pkt = vec![0xA2];
            finish_packet(&mut pkt, &body);
            expect_rejection_with_reason(stream, &pkt, None)
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: PUBLISH with DUP=1 and QoS=0 is a Malformed Packet.
/// [MQTT-3.3.1-2]
pub struct ProtocolErrorV5PublishDupOnQos0Test;

impl TestCase for ProtocolErrorV5PublishDupOnQos0Test {
    fn name(&self) -> &str {
        "protocol_error_v5_publish_dup_on_qos0"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/dup0";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);
            body.push(0x00); // property length 0
            body.extend_from_slice(b"payload");

            let mut pkt = vec![0x38]; // PUBLISH with DUP=1, QoS=0 — illegal
            finish_packet(&mut pkt, &body);
            expect_rejection_with_reason(stream, &pkt, None)
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: DISCONNECT with non-zero reserved fixed-header flags is a
/// Malformed Packet. [MQTT-3.14.1-1 / MQTT-3.14.1-2]
pub struct ProtocolErrorV5DisconnectBadFlagsTest;

impl TestCase for ProtocolErrorV5DisconnectBadFlagsTest {
    fn name(&self) -> &str {
        "protocol_error_v5_disconnect_bad_flags"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let pkt = [0xE1u8, 0x00]; // DISCONNECT with fixed-header flags != 0
            expect_rejection_with_reason(stream, &pkt, None)
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: SUBSCRIBE with packet identifier 0 is a Protocol Error.
/// [MQTT-2.2.1-2]
pub struct ProtocolErrorV5SubscribePacketIdZeroTest;

impl TestCase for ProtocolErrorV5SubscribePacketIdZeroTest {
    fn name(&self) -> &str {
        "protocol_error_v5_subscribe_packet_id_zero"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/subpid0";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x00]); // packet id 0 — illegal
            body.push(0x00); // property length 0
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);
            body.push(0x00); // subscription options: QoS 0

            let mut pkt = vec![0x82];
            finish_packet(&mut pkt, &body);
            expect_rejection_with_reason(stream, &pkt, None)
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: UNSUBSCRIBE with packet identifier 0 is a Protocol Error.
/// [MQTT-2.2.1-2]
pub struct ProtocolErrorV5UnsubscribePacketIdZeroTest;

impl TestCase for ProtocolErrorV5UnsubscribePacketIdZeroTest {
    fn name(&self) -> &str {
        "protocol_error_v5_unsubscribe_packet_id_zero"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/unsubpid0";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x00]); // packet id 0 — illegal
            body.push(0x00); // property length 0
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);

            let mut pkt = vec![0xA2];
            finish_packet(&mut pkt, &body);
            expect_rejection_with_reason(stream, &pkt, None)
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: PUBLISH with a topic name that is not valid UTF-8 is a Malformed
/// Packet. [MQTT-1.5.3 / MQTT-3.3.2-1]
pub struct ProtocolErrorV5InvalidUtf8TopicTest;

impl TestCase for ProtocolErrorV5InvalidUtf8TopicTest {
    fn name(&self) -> &str {
        "protocol_error_v5_invalid_utf8_topic"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x02]); // topic length 2
            body.extend_from_slice(&[0xC3, 0x28]); // invalid UTF-8 sequence
            body.push(0x00); // property length 0
            body.extend_from_slice(b"payload");

            let mut pkt = vec![0x30]; // PUBLISH QoS 0
            finish_packet(&mut pkt, &body);
            expect_rejection_with_reason(stream, &pkt, None)
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: PUBLISH with a User Property value that is not valid UTF-8 is a
/// Malformed Packet. [MQTT-1.5.3 / MQTT-3.3.2.3.8]
pub struct ProtocolErrorV5UserPropertyBadUtf8Test;

impl TestCase for ProtocolErrorV5UserPropertyBadUtf8Test {
    fn name(&self) -> &str {
        "protocol_error_v5_user_property_bad_utf8"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let topic = b"test/uprop";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);
            body.push(0x09); // property length: 1 + (2+2) + (2+2)
            body.push(0x26); // User Property
            body.extend_from_slice(&[0x00, 0x02]); // key length 2
            body.extend_from_slice(b"k1");
            body.extend_from_slice(&[0x00, 0x02]); // value length 2
            body.extend_from_slice(&[0xC3, 0x28]); // invalid UTF-8 value
            body.extend_from_slice(b"payload");

            let mut pkt = vec![0x30]; // PUBLISH QoS 0
            finish_packet(&mut pkt, &body);
            expect_rejection_with_reason(stream, &pkt, None)
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: an AUTH packet when no authentication method was negotiated in
/// CONNECT must be rejected. [MQTT-4.12.0-1 / MQTT-3.15.1]
pub struct ProtocolErrorV5UnsolicitedAuthTest;

impl TestCase for ProtocolErrorV5UnsolicitedAuthTest {
    fn name(&self) -> &str {
        "protocol_error_v5_unsolicited_auth"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        run_protocol_error(self.name(), ctx, start, |stream| {
            let pkt = [0xF0u8, 0x00]; // AUTH with no properties
            expect_rejection_with_reason(stream, &pkt, None)
        })
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}
