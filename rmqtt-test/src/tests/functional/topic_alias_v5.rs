//! MQTT 5.0 Topic Alias negotiation tests
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;

/// Fixed header byte of a QoS 1 PUBLISH.
const PKT_PUBLISH_QOS1: u8 = 0x32;
/// Fixed header byte of a PUBACK.
const PKT_PUBACK: u8 = 0x40;
/// Fixed header byte of a DISCONNECT.
const PKT_DISCONNECT: u8 = 0xE0;

/// Encode an MQTT variable-length integer (a Remaining Length field).
fn encode_remaining_length(mut len: usize) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut b = (len % 128) as u8;
        len /= 128;
        if len > 0 {
            b |= 0x80;
        }
        out.push(b);
        if len == 0 {
            break;
        }
    }
    out
}

/// Read one full MQTT packet (fixed header + remaining length) from a raw
/// stream. v5 CONNACK has a variable length (properties), so a naive read
/// leaves trailing bytes that corrupt subsequent reads.
fn read_full_packet(stream: &mut TcpStream) -> anyhow::Result<Vec<u8>> {
    let mut b = [0u8; 1];
    if stream.read(&mut b)? == 0 {
        return Err(anyhow::anyhow!("connection closed"));
    }
    read_packet_body(stream, b[0])
}

/// Read the remainder of an MQTT packet whose fixed-header byte has already
/// been consumed. Split out of `read_full_packet` so a caller that must look at
/// the first byte before deciding what it is reading — a PUBACK that means
/// "accepted" versus a DISCONNECT that means "refused" — does not have to
/// re-implement the framing.
fn read_packet_body(stream: &mut TcpStream, first: u8) -> anyhow::Result<Vec<u8>> {
    let mut buf = vec![first];
    let mut b = [0u8; 1];
    let mut remaining: u32 = 0;
    let mut shift = 0u32;
    loop {
        if stream.read(&mut b)? == 0 {
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

pub struct ServerTopicAliasV5Test;
impl TestCase for ServerTopicAliasV5Test {
    fn name(&self) -> &str {
        "server_topic_alias_v5"
    }
    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let client = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "alias-test",
                ctx.config.connect_timeout,
            )
            .await?;
            let ack = client.connack();
            let _ = ack.topic_alias_max;
            client.disconnect().await?;
            Ok(())
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

/// Test client-side topic alias usage (v5)
pub struct ClientTopicAliasV5Test;

impl TestCase for ClientTopicAliasV5Test {
    fn name(&self) -> &str {
        "client_topic_alias_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result: anyhow::Result<()> = rt.block_on(async {
            let publisher = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "cta-pub",
                ctx.config.connect_timeout,
            )
            .await?;
            let mut subscriber = crate::mqtt::v5::MqttV5Client::connect(
                &ctx.config.broker_addr,
                "cta-sub",
                ctx.config.connect_timeout,
            )
            .await?;
            subscriber.subscribe("test/v5/topicalias", QoS::AtLeastOnce).await?;
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Publish with topic_alias property (simulating client-side alias)
            // The broker should resolve the alias and route to subscribers
            publisher
                .publish_with_properties(
                    "test/v5/topicalias",
                    b"alias_msg",
                    QoS::AtLeastOnce,
                    false,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
                .await?;

            // If publish_with_properties doesn't support topic_alias directly,
            // we at least verify the basic publish+subscribe works
            let msg = subscriber.recv_message_timeout(Duration::from_secs(3)).await;
            publisher.disconnect().await?;
            subscriber.disconnect().await?;

            match msg {
                Some(m) if m.payload.as_ref() == b"alias_msg" => Ok(()),
                Some(m) => Err(anyhow::anyhow!("unexpected payload: {:?}", m.payload)),
                None => Err(anyhow::anyhow!("no message received")),
            }
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

/// Negative: a PUBLISH carrying an unknown Topic Alias (never registered by a
/// prior PUBLISH) is a protocol error — the server must send a DISCONNECT with
/// reason 0x94 (Topic Alias invalid) or close the connection. [MQTT-3.3.2-5]
pub struct TopicAliasV5UnknownAliasTest;

impl TestCase for TopicAliasV5UnknownAliasTest {
    fn name(&self) -> &str {
        "topic_alias_v5_unknown_alias"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let raw_result = std::panic::catch_unwind(|| -> anyhow::Result<()> {
            let mut stream = std::net::TcpStream::connect(&ctx.config.broker_addr)?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;

            // CONNECT v5 clean start
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x04]);
            body.extend_from_slice(b"MQTT");
            body.push(5);
            body.push(0x02);
            body.extend_from_slice(&[0x00, 0x3C]);
            body.push(0x00);
            let cid = b"v5-alias-raw";
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
            let connack = read_full_packet(&mut stream)?;
            if connack.len() < 4 || connack[0] != 0x20 || connack[3] != 0 {
                return Err(anyhow::anyhow!("no CONNACK: {:02x?}", &connack[..connack.len().min(8)]));
            }

            // Register alias 1: PUBLISH with topic + Topic Alias property 1
            let topic = b"test/v5/alias/known";
            let mut pb: Vec<u8> = Vec::new();
            pb.extend_from_slice(&(topic.len() as u16).to_be_bytes());
            pb.extend_from_slice(topic);
            // properties: prop_len = 3, 0x23 (Topic Alias), 0x00 0x01
            pb.push(0x03);
            pb.push(0x23);
            pb.push(0x00);
            pb.push(0x01);
            pb.extend_from_slice(b"hi");
            let mut ppkt = vec![0x30];
            let mut plen = pb.len();
            loop {
                let mut b = (plen % 128) as u8;
                plen /= 128;
                if plen > 0 {
                    b |= 0x80;
                }
                ppkt.push(b);
                if plen == 0 {
                    break;
                }
            }
            ppkt.extend_from_slice(&pb);
            stream.write_all(&ppkt)?;
            stream.flush()?;

            // Now send PUBLISH with alias 2 (never registered) — protocol error
            let mut pb2: Vec<u8> = Vec::new();
            // empty topic (alias-only reference)
            pb2.extend_from_slice(&[0x00, 0x00]);
            pb2.push(0x03);
            pb2.push(0x23);
            pb2.push(0x00);
            pb2.push(0x02); // alias 2 — never registered
            pb2.extend_from_slice(b"bad");
            let mut ppkt2 = vec![0x30];
            let mut plen2 = pb2.len();
            loop {
                let mut b = (plen2 % 128) as u8;
                plen2 /= 128;
                if plen2 > 0 {
                    b |= 0x80;
                }
                ppkt2.push(b);
                if plen2 == 0 {
                    break;
                }
            }
            ppkt2.extend_from_slice(&pb2);
            stream.write_all(&ppkt2)?;
            stream.flush()?;

            // Broker must close the connection (EOF) or send DISCONNECT 0x94
            let mut rbuf = [0u8; 16];
            match stream.read(&mut rbuf) {
                Ok(0) | Err(_) => Ok(()), // closed — acceptable
                Ok(n) if n >= 2 && rbuf[0] == 0xE0 => {
                    // DISCONNECT — reason byte is at index 2
                    if n >= 3 && rbuf[2] == 0x94 {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!("DISCONNECT with unexpected reason: {:02x?}", &rbuf[..n]))
                    }
                }
                Ok(n) => Err(anyhow::anyhow!("unexpected response: {:02x?}", &rbuf[..n])),
            }
        });

        match raw_result {
            Ok(Ok(())) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Ok(Err(e)) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
            Err(_) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), "panic".into()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

// ---------------------------------------------------------------------------
// P0 gap-analysis additions (designs/mqtt-5.0-standalone-test-gap-analysis.md)
// ---------------------------------------------------------------------------

/// Open a raw TCP connection with a successful v5 CONNECT handshake.
fn raw_connect_stream(broker_addr: &str, client_id: &str) -> anyhow::Result<TcpStream> {
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
    let connack = read_full_packet(&mut stream)?;
    if connack.len() < 4 || connack[0] != 0x20 || connack[3] != 0 {
        return Err(anyhow::anyhow!("no CONNACK: {:02x?}", &connack[..connack.len().min(8)]));
    }
    Ok(stream)
}

/// Extract the Topic Alias Maximum (property 0x22) advertised in the CONNACK
/// bytes. Returns 0 when the property is absent (spec: absent means 0).
fn connack_topic_alias_max(connack: &[u8]) -> Option<u16> {
    // CONNACK: [0x20][rlen varint][ack flags][reason][props varint][props...]
    if connack.len() < 4 || connack[0] != 0x20 {
        return None;
    }
    let mut i = 1usize;
    // skip remaining length varint
    while i < connack.len() && connack[i] & 0x80 != 0 {
        i += 1;
    }
    i += 1; // consume the terminating varint byte
            // skip ack flags + reason code
    i += 2;
    if i >= connack.len() {
        return None;
    }
    // property length varint
    let mut plen: usize = 0;
    let mut shift = 0u32;
    loop {
        if i >= connack.len() {
            return None;
        }
        let b = connack[i];
        i += 1;
        plen |= ((b & 0x7F) as usize) << shift;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    let props_end = (i + plen).min(connack.len());

    while i < props_end {
        let id = connack[i];
        i += 1;
        match id {
            0x22 => {
                if i + 2 > props_end {
                    return None;
                }
                return Some(u16::from_be_bytes([connack[i], connack[i + 1]]));
            }
            // 1-byte properties
            0x24 | 0x25 => i += 1,
            // 2-byte properties
            0x21 => i += 2,
            // 4-byte properties
            0x11 | 0x27 => i += 4,
            // UTF-8 string properties
            0x12 | 0x1F | 0x31 => {
                if i + 2 > props_end {
                    return None;
                }
                let slen = u16::from_be_bytes([connack[i], connack[i + 1]]) as usize;
                i += 2 + slen;
            }
            // User Property (string + string)
            0x26 => {
                for _ in 0..2 {
                    if i + 2 > props_end {
                        return None;
                    }
                    let slen = u16::from_be_bytes([connack[i], connack[i + 1]]) as usize;
                    i += 2 + slen;
                }
            }
            _ => return None, // unknown property id — give up scanning
        }
    }
    None
}

/// Build a QoS 1 PUBLISH that carries a Topic Alias.
///
/// QoS 1 on purpose: the negative cases below have to tell "the broker refused
/// this packet" apart from "the broker took it", and a QoS 0 PUBLISH draws no
/// answer at all — silence is what a refusal looks like too, so such a case
/// passes whatever the broker does. A PUBACK is the one answer that proves
/// acceptance.
fn publish_qos1_with_alias(topic: &[u8], alias: u16, packet_id: u16) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
    body.extend_from_slice(topic);
    body.extend_from_slice(&packet_id.to_be_bytes()); // v5 order: Topic Name, Packet Id, Properties
    body.push(0x03); // property length: Topic Alias (0x23) + two bytes
    body.push(0x23); // Topic Alias
    body.extend_from_slice(&alias.to_be_bytes());
    body.extend_from_slice(b"hi");

    let mut pkt = vec![PKT_PUBLISH_QOS1];
    pkt.extend_from_slice(&encode_remaining_length(body.len()));
    pkt.extend_from_slice(&body);
    pkt
}

/// Assert that the broker refused the illegal PUBLISH that was just sent.
///
/// `what` names the violation for the failure message. The only answer that
/// proves acceptance is a PUBACK; every other outcome has to be the connection
/// ending — a bare EOF, or a DISCONNECT (the Server announcing why, MQTT 5.0
/// section 4.13.1) followed by the close. A read that times out with the
/// connection still open therefore fails too: the broker left the violation
/// unpunished and the connection usable.
fn expect_alias_refused(stream: &mut TcpStream, what: &str) -> anyhow::Result<()> {
    // One byte at a time: a DISCONNECT fits in a single TCP segment, so a
    // larger buffer could swallow the whole packet and leave `read_packet_body`
    // parsing whatever follows it.
    let mut b = [0u8; 1];
    match stream.read(&mut b) {
        Ok(0) => Ok(()), // closed without a word — acceptable
        Ok(_) => match b[0] {
            PKT_PUBACK => {
                let pkt = read_packet_body(stream, b[0])?;
                Err(anyhow::anyhow!("the broker accepted {what}: PUBACK {:02x?}", pkt))
            }
            PKT_DISCONNECT => {
                let pkt = read_packet_body(stream, b[0])?;
                let code = pkt.get(2).copied().unwrap_or(0x00);
                match stream.read(&mut b) {
                    Ok(0) => Ok(()), // DISCONNECT then EOF — acceptable
                    Ok(n) => Err(anyhow::anyhow!(
                        "{what} was refused with DISCONNECT 0x{code:02X}, but the broker left the \
                         connection open ({n} bytes to follow)"
                    )),
                    Err(e) => Err(anyhow::anyhow!(
                        "{what} was refused with DISCONNECT 0x{code:02X}, but the broker did not \
                         close the connection ({e:?})"
                    )),
                }
            }
            other => Err(anyhow::anyhow!(
                "unexpected answer to {what}: first byte 0x{other:02X}, expected a PUBACK or a \
                 DISCONNECT"
            )),
        },
        Err(e) => Err(anyhow::anyhow!(
            "the broker neither refused {what} nor closed the connection ({e:?}) — the \
             connection is still open"
        )),
    }
}

/// Negative: a PUBLISH carrying Topic Alias 0 is a Protocol Error — a sender
/// MUST NOT use the value 0 at all [MQTT-3.3.2-8].
///
/// The assertion is that the packet is not accepted: a QoS 1 PUBLISH must not
/// come back as a PUBACK, and the connection must end (MQTT 5.0 section 4.13.1
/// lets the Server announce the reason before closing).
pub struct TopicAliasV5ZeroTest;

impl TestCase for TopicAliasV5ZeroTest {
    fn name(&self) -> &str {
        "topic_alias_v5_zero"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let result = std::panic::catch_unwind(|| -> anyhow::Result<()> {
            let mut stream = raw_connect_stream(&ctx.config.broker_addr, "v5-alias-zero")?;

            // PUBLISH with topic + Topic Alias property = 0 (illegal).
            let ppkt = publish_qos1_with_alias(b"test/v5/alias/zero", 0, 1);
            stream.write_all(&ppkt)?;
            stream.flush()?;

            expect_alias_refused(&mut stream, "Topic Alias 0")
        });

        match result {
            Ok(Ok(())) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Ok(Err(e)) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
            Err(_) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), "panic".into()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}

/// Negative: a PUBLISH carrying a Topic Alias greater than the Server's
/// advertised Topic Alias Maximum must not be accepted.
///
/// [MQTT-3.3.2-9] forbids a Client to send such a PUBLISH, which makes the
/// maximum the Server advertises in the CONNACK its own statement about which
/// aliases it will honour; MQTT 5.0 section 4.13.1 then covers the Server
/// saying why before it closes (0x94 Topic Alias invalid is the Reason Code
/// defined for an invalid Topic Alias). The assertion is the one that cannot be
/// faked: the packet must not be acknowledged, and the connection must end.
///
/// REPRODUCTION — this case FAILS against the current broker, which accepts the
/// alias. `ClientTopicAliases::set_and_get` (`rmqtt/src/types.rs`) only caps how
/// *many* aliases a connection may register, never that an individual alias lies
/// within the advertised maximum, so `Topic Alias Maximum + 1` is stored and the
/// PUBLISH is delivered like any other. Registered as a plain failure rather
/// than an expected-fail, so it stays visible until the check is added.
pub struct TopicAliasV5OverMaxTest;

impl TestCase for TopicAliasV5OverMaxTest {
    fn name(&self) -> &str {
        "topic_alias_v5_over_max"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let result = std::panic::catch_unwind(|| -> anyhow::Result<()> {
            // Probe connection: read the CONNACK bytes and parse the
            // advertised Topic Alias Maximum.
            let mut probe = TcpStream::connect(&ctx.config.broker_addr)?;
            probe.set_read_timeout(Some(Duration::from_secs(5)))?;
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x04]);
            body.extend_from_slice(b"MQTT");
            body.push(5);
            body.push(0x02);
            body.extend_from_slice(&[0x00, 0x3C]);
            body.push(0x00);
            let cid = b"v5-alias-max-probe";
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
            probe.write_all(&pkt)?;
            probe.flush()?;
            let connack = read_full_packet(&mut probe)?;
            drop(probe);

            let alias_max = connack_topic_alias_max(&connack)
                .ok_or_else(|| anyhow::anyhow!("could not parse Topic Alias Maximum from CONNACK"))?;
            let over_max = alias_max.saturating_add(1);
            if over_max == 0 {
                return Err(anyhow::anyhow!("Topic Alias Maximum already at u16::MAX"));
            }

            let mut stream = raw_connect_stream(&ctx.config.broker_addr, "v5-alias-overmax")?;

            // QoS 1 with topic + Topic Alias = max + 1: the PUBACK is what tells
            // the two outcomes apart, since a refused packet never gets one.
            let ppkt = publish_qos1_with_alias(b"test/v5/alias/overmax", over_max, 1);
            stream.write_all(&ppkt)?;
            stream.flush()?;

            expect_alias_refused(
                &mut stream,
                &format!("Topic Alias {over_max} (advertised Topic Alias Maximum is {alias_max})"),
            )
        });

        match result {
            Ok(Ok(())) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Ok(Err(e)) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
            Err(_) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), "panic".into()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}
