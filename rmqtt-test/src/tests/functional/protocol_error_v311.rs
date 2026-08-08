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

/// Send bytes and check whether the broker closed the connection (EOF /
/// timeout on the following read). Returns true when closed.
fn expect_connection_closed(stream: &mut TcpStream, data: &[u8]) -> bool {
    let _ = stream.write_all(data);
    let _ = stream.flush();
    let mut buf = [0u8; 16];
    matches!(stream.read(&mut buf), Ok(0) | Err(_))
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
