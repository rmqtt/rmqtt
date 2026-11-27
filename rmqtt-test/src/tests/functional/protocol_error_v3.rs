//! MQTT v3.1 protocol error handling tests
//!
//! Covers malformed / illegal packets (spec section 2.1-2.4):
//! - SUBSCRIBE with requested QoS 3 (both QoS bits set) is a protocol error
//! - PUBLISH QoS 1 with packet identifier 0 is invalid
//! - remaining length encoded in more than 4 bytes is invalid
//! - SUBSCRIBE with an empty topic filter is invalid
//! - illegal fixed header reserved bits (packet type 0)
//!
//! These tests craft raw packets (the codec rejects them before they reach
//! the wire) and assert the broker closes the connection or errors out.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Open a raw TCP connection to the broker and perform a valid v3.1 CONNECT
/// (MQIsdp + level 3). Returns the stream with CONNACK already consumed.
fn raw_connect_v3(broker_addr: &str, client_id: &str) -> anyhow::Result<TcpStream> {
    let mut stream = TcpStream::connect(broker_addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;

    // CONNECT: MQIsdp, level 3, clean session
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&[0x00, 0x06]);
    body.extend_from_slice(b"MQIsdp");
    body.push(3); // level
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
    stream.write_all(&pkt)?;
    stream.flush()?;

    // Read CONNACK
    let mut buf = [0u8; 4];
    let n = stream.read(&mut buf)?;
    if n < 4 || buf[0] != 0x20 {
        return Err(anyhow::anyhow!("expected CONNACK, got {:02x?}", &buf[..n]));
    }
    if buf[3] != 0 {
        return Err(anyhow::anyhow!("CONNACK refused: return code {}", buf[3]));
    }
    Ok(stream)
}

/// Send bytes and then try to read a response. Returns true if the broker
/// closed the connection (EOF / timeout on read) — the expected outcome for
/// protocol errors.
fn expect_connection_closed(stream: &mut TcpStream, data: &[u8]) -> bool {
    let _ = stream.write_all(data);
    let _ = stream.flush();
    let mut buf = [0u8; 16];
    matches!(stream.read(&mut buf), Ok(0) | Err(_))
}

/// Negative: SUBSCRIBE with requested QoS 3 (both QoS bits = 1) is a protocol
/// error — the broker must close the connection (MQTT-3.8.3-4).
pub struct ProtocolErrorV3SubscribeQos3Test;

impl TestCase for ProtocolErrorV3SubscribeQos3Test {
    fn name(&self) -> &str {
        "protocol_error_v3_subscribe_qos3"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let result = std::panic::catch_unwind(|| -> anyhow::Result<()> {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let mut stream = raw_connect_v3(&ctx.config.broker_addr, &format!("qos3-{uid}"))?;

            // SUBSCRIBE with a single filter and requested QoS 3 (0x03)
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

            if expect_connection_closed(&mut stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close connection for QoS 3 subscribe"))
            }
        });

        match result {
            Ok(Ok(())) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Ok(Err(e)) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
            Err(_) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), "panic".into()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: PUBLISH with QoS 1 and packet identifier 0 is invalid
/// (MQTT-2.3.1-1: packet id 0 is reserved).
pub struct ProtocolErrorV3PublishPacketIdZeroTest;

impl TestCase for ProtocolErrorV3PublishPacketIdZeroTest {
    fn name(&self) -> &str {
        "protocol_error_v3_publish_packet_id_zero"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let result = std::panic::catch_unwind(|| -> anyhow::Result<()> {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let mut stream = raw_connect_v3(&ctx.config.broker_addr, &format!("pid0-{uid}"))?;

            // PUBLISH QoS 1 with packet id 0: fixed header 0x32 (QoS 1, no retain)
            let topic = b"test/pid0";
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            body.extend_from_slice(topic);
            body.extend_from_slice(&[0x00, 0x00]); // packet id 0 — illegal
            body.extend_from_slice(b"payload");

            let mut pkt = vec![0x32];
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

            if expect_connection_closed(&mut stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close connection for packet id 0"))
            }
        });

        match result {
            Ok(Ok(())) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Ok(Err(e)) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
            Err(_) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), "panic".into()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: remaining length encoded in 5 bytes is invalid (max is 4).
pub struct ProtocolErrorV3BadRemainingLengthTest;

impl TestCase for ProtocolErrorV3BadRemainingLengthTest {
    fn name(&self) -> &str {
        "protocol_error_v3_bad_remaining_length"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let result = std::panic::catch_unwind(|| -> anyhow::Result<()> {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let mut stream = raw_connect_v3(&ctx.config.broker_addr, &format!("remlen-{uid}"))?;

            // PINGREQ with a 5-byte remaining length (0x80 0x80 0x80 0x80 0x01)
            let pkt = [0xC0u8, 0x80, 0x80, 0x80, 0x80, 0x01];

            if expect_connection_closed(&mut stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close connection for 5-byte remaining length"))
            }
        });

        match result {
            Ok(Ok(())) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Ok(Err(e)) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
            Err(_) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), "panic".into()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: SUBSCRIBE with no topic filters (only a packet id) is invalid
/// and must close the connection. [MQTT-3.8.3-1]
///
/// Strict version: the broker must close the connection, not reply SUBACK.
pub struct ProtocolErrorV3EmptyTopicFilterTest;

impl TestCase for ProtocolErrorV3EmptyTopicFilterTest {
    fn name(&self) -> &str {
        "protocol_error_v3_empty_topic_filter"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let result = std::panic::catch_unwind(|| -> anyhow::Result<()> {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let mut stream = raw_connect_v3(&ctx.config.broker_addr, &format!("emptyflt-{uid}"))?;

            // SUBSCRIBE with only a packet id, no topic filters
            let pkt = [0x82u8, 0x02, 0x00, 0x01];

            if expect_connection_closed(&mut stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "broker did not close for SUBSCRIBE with no topic filters [MQTT-3.8.3-1]"
                ))
            }
        });

        match result {
            Ok(Ok(())) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Ok(Err(e)) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
            Err(_) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), "panic".into()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: SUBSCRIBE with an empty-string topic filter is invalid and must
/// close the connection. [MQTT-4.7.3-1]
pub struct ProtocolErrorV3SubscribeEmptyFilterTest;

impl TestCase for ProtocolErrorV3SubscribeEmptyFilterTest {
    fn name(&self) -> &str {
        "protocol_error_v3_subscribe_empty_filter"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let result = std::panic::catch_unwind(|| -> anyhow::Result<()> {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let mut stream = raw_connect_v3(&ctx.config.broker_addr, &format!("emptyflt-{uid}"))?;

            // SUBSCRIBE with one empty-string topic filter (len 0x0000)
            let pkt = [0x82u8, 0x04, 0x00, 0x01, 0x00, 0x00];

            if expect_connection_closed(&mut stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "broker did not close for SUBSCRIBE with an empty topic filter [MQTT-4.7.3-1]"
                ))
            }
        });

        match result {
            Ok(Ok(())) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Ok(Err(e)) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
            Err(_) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), "panic".into()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: UNSUBSCRIBE with no topic filters (only a packet id) is invalid
/// and must close the connection. [MQTT-3.10.3-1]
pub struct ProtocolErrorV3UnsubscribeEmptyTest;

impl TestCase for ProtocolErrorV3UnsubscribeEmptyTest {
    fn name(&self) -> &str {
        "protocol_error_v3_unsubscribe_empty"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let result = std::panic::catch_unwind(|| -> anyhow::Result<()> {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let mut stream = raw_connect_v3(&ctx.config.broker_addr, &format!("emptyunsub-{uid}"))?;

            // UNSUBSCRIBE with only a packet id, no topic filters
            let pkt = [0xA2u8, 0x02, 0x00, 0x01];

            if expect_connection_closed(&mut stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "broker did not close for UNSUBSCRIBE with no topic filters [MQTT-3.10.3-1]"
                ))
            }
        });

        match result {
            Ok(Ok(())) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Ok(Err(e)) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
            Err(_) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), "panic".into()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: UNSUBSCRIBE with an empty-string topic filter is invalid and
/// must close the connection. [MQTT-4.7.3-1]
pub struct ProtocolErrorV3UnsubscribeEmptyFilterTest;

impl TestCase for ProtocolErrorV3UnsubscribeEmptyFilterTest {
    fn name(&self) -> &str {
        "protocol_error_v3_unsubscribe_empty_filter"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let result = std::panic::catch_unwind(|| -> anyhow::Result<()> {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let mut stream = raw_connect_v3(&ctx.config.broker_addr, &format!("emptyunsub-{uid}"))?;

            // UNSUBSCRIBE with one empty-string topic filter (len 0x0000)
            let pkt = [0xA2u8, 0x04, 0x00, 0x01, 0x00, 0x00];

            if expect_connection_closed(&mut stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "broker did not close for UNSUBSCRIBE with an empty topic filter [MQTT-4.7.3-1]"
                ))
            }
        });

        match result {
            Ok(Ok(())) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Ok(Err(e)) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
            Err(_) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), "panic".into()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: a packet with reserved fixed header bits (packet type 0) is
/// invalid and must cause the connection to close.
pub struct ProtocolErrorV3ReservedPacketTypeTest;

impl TestCase for ProtocolErrorV3ReservedPacketTypeTest {
    fn name(&self) -> &str {
        "protocol_error_v3_reserved_packet_type"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let result = std::panic::catch_unwind(|| -> anyhow::Result<()> {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let mut stream = raw_connect_v3(&ctx.config.broker_addr, &format!("restype-{uid}"))?;

            // Packet type 0 (0x00) is reserved
            let pkt = [0x00u8, 0x00];

            if expect_connection_closed(&mut stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close connection for reserved packet type"))
            }
        });

        match result {
            Ok(Ok(())) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Ok(Err(e)) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
            Err(_) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), "panic".into()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: a SUBSCRIBE whose fixed header QoS bits are 0 (not 1) is a
/// protocol error (MQTT-3.8.1-1: SUBSCRIBE must be QoS 1).
pub struct ProtocolErrorV3SubscribeQos0FixedHeaderTest;

impl TestCase for ProtocolErrorV3SubscribeQos0FixedHeaderTest {
    fn name(&self) -> &str {
        "protocol_error_v3_subscribe_qos0_fixed_header"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let result = std::panic::catch_unwind(|| -> anyhow::Result<()> {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let mut stream = raw_connect_v3(&ctx.config.broker_addr, &format!("subqos0-{uid}"))?;

            // SUBSCRIBE with fixed header QoS bits = 0 (first byte 0x80 instead
            // of 0x82) — illegal per MQTT-3.8.1-1.
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

            if expect_connection_closed(&mut stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close connection for SUBSCRIBE with QoS 0 fixed header"))
            }
        });

        match result {
            Ok(Ok(())) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Ok(Err(e)) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
            Err(_) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), "panic".into()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}

/// Negative: a PUBLISH with an empty (zero-length) Topic Name is invalid and
/// must close the connection. [MQTT-4.7.3-1]
pub struct ProtocolErrorV3PublishEmptyTopicTest;

impl TestCase for ProtocolErrorV3PublishEmptyTopicTest {
    fn name(&self) -> &str {
        "protocol_error_v3_publish_empty_topic"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let result = std::panic::catch_unwind(|| -> anyhow::Result<()> {
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let mut stream = raw_connect_v3(&ctx.config.broker_addr, &format!("pubempty-{uid}"))?;

            // PUBLISH QoS 0 with a zero-length Topic Name (len 0x0000)
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

            if expect_connection_closed(&mut stream, &pkt) {
                Ok(())
            } else {
                Err(anyhow::anyhow!("broker did not close for PUBLISH with empty topic [MQTT-4.7.3-1]"))
            }
        });

        match result {
            Ok(Ok(())) => TestResult::passed(self.name(), "functional_v3", start.elapsed()),
            Ok(Err(e)) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), e.to_string()),
            Err(_) => TestResult::failed(self.name(), "functional_v3", start.elapsed(), "panic".into()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }
}
