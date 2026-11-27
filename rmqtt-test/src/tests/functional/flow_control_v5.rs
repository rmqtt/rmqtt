//! MQTT 5.0 Flow Control (receive_max) tests

use std::num::NonZeroU16;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Test V5 flow control with receive_max=2 - rapid publishes should not overflow
pub struct FlowControlV5Test;

impl TestCase for FlowControlV5Test {
    fn name(&self) -> &str {
        "flow_control_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            // Connect with receive_max=2
            let publisher = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "fc-pub-v5",
                ctx.config.connect_timeout,
                true,
                60,
                None,
                None,
                None,
                None,
                NonZeroU16::new(2),
                None,
            )
            .await?;
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "fc-sub-v5",
                ctx.config.connect_timeout,
                true,
                60,
                None,
                None,
                None,
                None,
                NonZeroU16::new(2),
                None,
            )
            .await?;

            subscriber.subscribe("test/flow/control", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Publish rapidly - should not overflow despite receive_max=2
            for i in 0..10 {
                publisher
                    .publish("test/flow/control", format!("msg{}", i).as_bytes(), QoS::AtLeastOnce, false)
                    .await?;
            }

            // Verify at least some messages were delivered
            let mut received = 0;
            for _ in 0..10 {
                if subscriber.recv_message_timeout(Duration::from_secs(3)).await.is_some() {
                    received += 1;
                }
            }

            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            if received >= 1 {
                Ok(())
            } else {
                Err(anyhow::anyhow!("no messages received with flow control"))
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

// ---------------------------------------------------------------------------
// P0 gap-analysis additions (designs/mqtt-5.0-standalone-test-gap-analysis.md)
// ---------------------------------------------------------------------------

use std::io::{Read, Write};
use std::net::TcpStream;

/// Open a raw TCP connection with a successful v5 CONNECT handshake and
/// return (stream, connack bytes).
fn raw_connect_with_connack(broker_addr: &str, client_id: &str) -> anyhow::Result<(TcpStream, Vec<u8>)> {
    let mut stream = TcpStream::connect(broker_addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;

    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&[0x00, 0x04]);
    body.extend_from_slice(b"MQTT");
    body.push(5);
    body.push(0x02); // clean start
    body.extend_from_slice(&[0x00, 0x3C]); // keep alive 60
    body.push(0x00); // property length 0
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
    let connack = match read_full_packet_fc(&mut stream)? {
        ReadOutcome::Packet(p) => p,
        ReadOutcome::Eof | ReadOutcome::Timeout => {
            return Err(anyhow::anyhow!("no CONNACK: connection closed or timed out"))
        }
    };
    if connack.len() < 4 || connack[0] != 0x20 || connack[3] != 0 {
        return Err(anyhow::anyhow!("no CONNACK: {:02x?}", &connack[..connack.len().min(8)]));
    }
    Ok((stream, connack))
}

/// Outcome of a raw packet read attempt.
enum ReadOutcome {
    Packet(Vec<u8>),
    Eof,
    Timeout,
}

/// Read one full MQTT packet (fixed header + remaining length) from a raw
/// stream, distinguishing EOF from a read timeout.
fn read_full_packet_fc(stream: &mut TcpStream) -> anyhow::Result<ReadOutcome> {
    fn is_timeout(e: &std::io::Error) -> bool {
        matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)
    }
    // WSAECONNABORTED / WSAECONNRESET: the peer closed the connection while
    // unread data remained in our receive buffer (e.g. after a burst without
    // reading PUBACKs) — treat it as "connection closed".
    fn is_aborted(e: &std::io::Error) -> bool {
        matches!(e.kind(), std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::ConnectionReset)
    }

    let mut buf = Vec::new();
    let mut b = [0u8; 1];
    match stream.read(&mut b) {
        Ok(0) => return Ok(ReadOutcome::Eof),
        Err(e) if is_timeout(&e) => return Ok(ReadOutcome::Timeout),
        Err(e) if is_aborted(&e) => return Ok(ReadOutcome::Eof),
        Err(e) => return Err(e.into()),
        Ok(_) => {}
    }
    buf.push(b[0]);

    let mut remaining: u32 = 0;
    let mut shift = 0u32;
    loop {
        match stream.read(&mut b) {
            Ok(0) => return Ok(ReadOutcome::Eof),
            Err(e) if is_timeout(&e) => return Ok(ReadOutcome::Timeout),
            Err(e) if is_aborted(&e) => return Ok(ReadOutcome::Eof),
            Err(e) => return Err(e.into()),
            Ok(_) => {}
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
    match stream.read_exact(&mut rest) {
        Ok(()) => {}
        Err(e) if is_timeout(&e) => return Ok(ReadOutcome::Timeout),
        Err(e) if is_aborted(&e) => return Ok(ReadOutcome::Eof),
        Err(e) => return Err(e.into()),
    }
    buf.extend_from_slice(&rest);
    Ok(ReadOutcome::Packet(buf))
}

/// Parse the Receive Maximum (property 0x21) advertised in the CONNACK bytes.
/// Returns `None` when the property is absent or the CONNACK cannot be
/// scanned; the caller then derives a burst size from the spec default
/// (65535, effectively unlimited).
fn connack_receive_max(connack: &[u8]) -> Option<u16> {
    if connack.len() < 4 || connack[0] != 0x20 {
        return None;
    }
    let mut i = 1usize;
    while i < connack.len() && connack[i] & 0x80 != 0 {
        i += 1;
    }
    i += 1; // consume terminating varint byte
    i += 2; // ack flags + reason code
    if i >= connack.len() {
        return None;
    }
    let mut plen: usize = 0;
    let mut shift = 0u32;
    loop {
        if i >= connack.len() {
            return None;
        }
        let v = connack[i];
        i += 1;
        plen |= ((v & 0x7F) as usize) << shift;
        if v & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    let props_end = (i + plen).min(connack.len());

    while i < props_end {
        let id = connack[i];
        i += 1;
        match id {
            0x21 => {
                if i + 2 > props_end {
                    return None;
                }
                return Some(u16::from_be_bytes([connack[i], connack[i + 1]]));
            }
            0x24 | 0x25 => i += 1,
            0x22 => i += 2,
            0x11 | 0x27 => i += 4,
            0x12 | 0x1F | 0x31 => {
                if i + 2 > props_end {
                    return None;
                }
                let slen = u16::from_be_bytes([connack[i], connack[i + 1]]) as usize;
                i += 2 + slen;
            }
            0x26 => {
                for _ in 0..2 {
                    if i + 2 > props_end {
                        return None;
                    }
                    let slen = u16::from_be_bytes([connack[i], connack[i + 1]]) as usize;
                    i += 2 + slen;
                }
            }
            _ => return None,
        }
    }
    None
}

/// Negative: a client that exceeds the broker's advertised Receive Maximum
/// (more in-flight QoS 1 PUBLISHes than the broker allows without waiting for
/// PUBACK) must be disconnected with reason 0x93. [MQTT-4.9.0-1 / MQTT-4.9.0-2]
///
/// Known broker defect (registered, see conformance-gap log): the broker
/// accepts unbounded in-flight QoS 1 PUBLISHes without sending DISCONNECT
/// 0x93 or closing the connection. Expected-fail until enforced; a pass
/// surfaces as UNEXPECTED-PASS and should then be promoted.
pub struct FlowControlV5ReceiveMaxViolationTest;

impl TestCase for FlowControlV5ReceiveMaxViolationTest {
    fn name(&self) -> &str {
        "flow_control_v5_receive_max_violation"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let result = std::panic::catch_unwind(|| -> anyhow::Result<()> {
            let (mut stream, connack) = raw_connect_with_connack(&ctx.config.broker_addr, "fc-violation")?;
            let recv_max = connack_receive_max(&connack)
                .ok_or_else(|| anyhow::anyhow!("could not parse Receive Maximum from CONNACK"))?;

            // Burst of QoS 1 PUBLISHes far beyond the advertised Receive
            // Maximum, with unique packet ids, sent without reading PUBACKs.
            let burst: u16 = recv_max.saturating_mul(4).saturating_add(20);
            let topic = b"test/fc/violation";
            for pid in 1..=burst {
                let mut body: Vec<u8> = Vec::new();
                body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
                body.extend_from_slice(topic);
                // Wire order for QoS > 0: topic, THEN packet id, THEN
                // properties. (A property byte before the packet id makes the
                // packet malformed -> Malformed Packet close, which would let
                // this test pass vacuously without exercising Receive Maximum.)
                body.extend_from_slice(&pid.to_be_bytes()); // packet id
                body.push(0x00); // property length 0
                body.extend_from_slice(b"v"); // payload

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
                // A write may fail (WSAECONNABORTED / EPIPE / ECONNRESET) once
                // the broker has detected the violation and aborted the
                // connection mid-burst — that is itself evidence of rejection,
                // so stop writing and fall through to the read phase to
                // collect the DISCONNECT/EOF evidence.
                if stream.write_all(&pkt).is_err() {
                    break;
                }
            }
            let _ = stream.flush();

            // Read responses until DISCONNECT / EOF / deadline.
            let deadline = Instant::now() + Duration::from_secs(5);
            stream.set_read_timeout(Some(Duration::from_millis(500)))?;
            loop {
                if Instant::now() > deadline {
                    return Err(anyhow::anyhow!(
                        "broker did not disconnect after {} in-flight QoS1 PUBLISHes \
                         (Receive Maximum advertised: {})",
                        burst,
                        recv_max
                    ));
                }
                match read_full_packet_fc(&mut stream)? {
                    ReadOutcome::Eof => return Ok(()), // closed without DISCONNECT
                    ReadOutcome::Timeout => continue,  // keep waiting until deadline
                    ReadOutcome::Packet(pkt) => {
                        let ptype = pkt[0] & 0xF0;
                        if ptype == 0xE0 {
                            // DISCONNECT: reason code is the first remaining byte
                            if pkt.len() >= 3 && pkt[2] != 0x93 {
                                return Err(anyhow::anyhow!(
                                    "DISCONNECT with unexpected reason 0x{:02X}, expected 0x93",
                                    pkt[2]
                                ));
                            }
                            return Ok(());
                        }
                        // PUBACK (0x40) etc. — keep reading
                    }
                }
            }
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

    fn expectation(&self) -> crate::framework::testcase::Expectation {
        crate::framework::testcase::Expectation::ExpectedFail
    }
}

// ---------------------------------------------------------------------------
// P2 gap-analysis additions (designs/mqtt-5.0-standalone-test-gap-analysis.md)
// ---------------------------------------------------------------------------

/// G27: broker -> client direction flow control. The client announces
/// Receive Maximum = 1 and does NOT acknowledge the first delivery; the
/// broker must not push a second unacknowledged QoS 1 delivery beyond the
/// quota (MQTT-4.9.0-1/2). Only after a manual PUBACK may the next message
/// arrive.
pub struct FlowControlV5InflightCapStrictTest;

impl TestCase for FlowControlV5InflightCapStrictTest {
    fn name(&self) -> &str {
        "flow_control_v5_inflight_cap_strict"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result: anyhow::Result<()> = rt.block_on(async {
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect_with_options(
                &ctx.config.broker_addr,
                "fc-strict-sub",
                ctx.config.connect_timeout,
                true,
                60,
                None,
                None,
                None,
                None,
                NonZeroU16::new(1),
                None,
            )
            .await?;
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "fc-strict-pub",
                ctx.config.connect_timeout,
            )
            .await?;

            subscriber.subscribe("test/v5/fc/strict", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(200)).await;

            // Stall acknowledgements: broker may keep at most 1 unacked QoS 1
            // delivery in flight against this client.
            subscriber.set_auto_puback(false);

            for i in 0..3 {
                publisher
                    .publish("test/v5/fc/strict", format!("fc{i}").as_bytes(), QoS::AtLeastOnce, false)
                    .await?;
            }

            // First delivery must arrive.
            let m1 = subscriber
                .recv_message_timeout(Duration::from_secs(3))
                .await
                .ok_or_else(|| anyhow::anyhow!("no delivery within timeout (broker stalled entirely)"))?;
            let pid1 = m1.packet_id.ok_or_else(|| anyhow::anyhow!("QoS 1 delivery without packet id"))?;

            // While unacknowledged, no further delivery may be pushed
            // (in-flight quota = 1). Allow a short grace window for the
            // broker to make its decision.
            let extra = subscriber.recv_message_timeout(Duration::from_millis(700)).await;
            if let Some(msg) = extra {
                return Err(anyhow::anyhow!(
                    "broker pushed a second delivery while the first was unacknowledged \
                     (Receive Maximum=1 violated, MQTT-4.9.0-1): topic={:?} pid={:?}",
                    msg.topic,
                    msg.packet_id
                ));
            }

            // Acknowledge the first delivery: the next one must now arrive.
            subscriber.send_puback(pid1).await?;
            let m2 = subscriber
                .recv_message_timeout(Duration::from_secs(3))
                .await
                .ok_or_else(|| anyhow::anyhow!("no second delivery after PUBACK"))?;
            let pid2 =
                m2.packet_id.ok_or_else(|| anyhow::anyhow!("second QoS 1 delivery without packet id"))?;
            if pid2 == pid1 {
                return Err(anyhow::anyhow!(
                    "broker redelivered the same packet id instead of the next message"
                ));
            }

            subscriber.send_puback(pid2).await?;
            let m3 = subscriber
                .recv_message_timeout(Duration::from_secs(3))
                .await
                .ok_or_else(|| anyhow::anyhow!("no third delivery after second PUBACK"))?;
            let pid3 =
                m3.packet_id.ok_or_else(|| anyhow::anyhow!("third QoS 1 delivery without packet id"))?;
            if pid3 == pid2 || pid3 == pid1 {
                return Err(anyhow::anyhow!("unexpected packet id reuse: {pid3}"));
            }
            subscriber.send_puback(pid3).await?;

            subscriber.set_auto_puback(true);
            publisher.disconnect().await?;
            subscriber.disconnect().await?;
            Ok(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}
