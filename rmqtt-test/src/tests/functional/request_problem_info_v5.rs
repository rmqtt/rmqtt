//! G23 / G24 (P2): Request Problem Information & Request Response Information.
//!
//! - G23 [MQTT-3.1.2-29]: with Request Problem Information = 0, the broker
//!   MUST NOT send Reason String (0x1F) or User Property (0x26) on any
//!   packet other than PUBLISH, CONNACK or DISCONNECT. Observable on
//!   UNSUBACK and PUBACK here.
//! - G24 [MQTT-3.2.2.3.5]: with Request Response Information = 1, the
//!   CONNACK MAY carry Response Information (0x1A). Record-type: the
//!   broker's actual behavior is reported, not asserted.
//!
//! Both tests use raw sockets so the exact property bytes are observable.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{Expectation, TestCase, TestResult};

/// Read one full MQTT packet (fixed header + remaining length).
fn read_full_packet(stream: &mut TcpStream) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut b = [0u8; 1];
    let n = stream.read(&mut b)?;
    if n == 0 {
        return Err(anyhow::anyhow!("connection closed"));
    }
    buf.push(b[0]);
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

/// Build a raw v5 CONNECT with optional Request Problem Information (0x17)
/// and Request Response Information (0x19) properties.
fn raw_connect_with_props(
    client_id: &str,
    request_problem_info: Option<bool>,
    request_response_info: Option<bool>,
) -> Vec<u8> {
    let mut props: Vec<u8> = Vec::new();
    if let Some(v) = request_problem_info {
        props.push(0x17);
        props.push(u8::from(v));
    }
    if let Some(v) = request_response_info {
        props.push(0x19);
        props.push(u8::from(v));
    }

    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&[0x00, 0x04]);
    body.extend_from_slice(b"MQTT");
    body.push(5); // level
    body.push(0x02); // clean start
    body.extend_from_slice(&[0x00, 0x3C]); // keep alive 60
                                           // property length varint
    let mut plen = props.len();
    loop {
        let mut b = (plen % 128) as u8;
        plen /= 128;
        if plen > 0 {
            b |= 0x80;
        }
        body.push(b);
        if plen == 0 {
            break;
        }
    }
    body.extend_from_slice(&props);
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

/// Open a raw connection, send the CONNECT, return the stream after a
/// successful CONNACK.
fn raw_connect(
    broker_addr: &str,
    client_id: &str,
    request_problem_info: Option<bool>,
    request_response_info: Option<bool>,
) -> anyhow::Result<TcpStream> {
    let mut stream = TcpStream::connect(broker_addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let pkt = raw_connect_with_props(client_id, request_problem_info, request_response_info);
    stream.write_all(&pkt)?;
    stream.flush()?;
    let connack = read_full_packet(&mut stream)?;
    if connack.len() < 4 || connack[0] != 0x20 || connack[3] != 0 {
        return Err(anyhow::anyhow!("CONNECT refused: {:02x?}", &connack[..connack.len().min(8)]));
    }
    Ok(stream)
}

/// Property ID value-length rules shared by CONNACK / SUBACK / UNSUBACK /
/// PUBACK / DISCONNECT property sections. Returns the list of property IDs
/// found (values skipped).
fn parse_property_ids(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut ids = Vec::new();
    let mut cur = 0usize;
    let read_utf8 = |b: &[u8], cur: &mut usize| -> anyhow::Result<()> {
        if *cur + 2 > b.len() {
            return Err(anyhow::anyhow!("truncated utf8 property"));
        }
        let len = u16::from_be_bytes([b[*cur], b[*cur + 1]]) as usize;
        *cur += 2 + len;
        Ok(())
    };
    let read_u16 = |b: &[u8], cur: &mut usize| -> anyhow::Result<()> {
        if *cur + 2 > b.len() {
            return Err(anyhow::anyhow!("truncated u16 property"));
        }
        *cur += 2;
        Ok(())
    };
    let read_u32 = |b: &[u8], cur: &mut usize| -> anyhow::Result<()> {
        if *cur + 4 > b.len() {
            return Err(anyhow::anyhow!("truncated u32 property"));
        }
        *cur += 4;
        Ok(())
    };
    while cur < bytes.len() {
        let id = bytes[cur];
        cur += 1;
        ids.push(id);
        match id {
            0x01 | 0x02 | 0x03 | 0x08 | 0x09 | 0x12 | 0x13 | 0x1A | 0x1C | 0x1F => {
                read_utf8(bytes, &mut cur)?
            }
            0x11 | 0x27 => read_u32(bytes, &mut cur)?,
            0x21..=0x23 => read_u16(bytes, &mut cur)?,
            0x24 | 0x25 | 0x28 | 0x29 | 0x2A => { /* single byte value */ }
            0x26 => {
                read_utf8(bytes, &mut cur)?; // key
                read_utf8(bytes, &mut cur)?; // value
            }
            _ => return Err(anyhow::anyhow!("unknown property id 0x{id:02X}")),
        }
        if cur > bytes.len() {
            return Err(anyhow::anyhow!("property value overflow"));
        }
    }
    Ok(ids)
}

/// Split the payload of an ack packet (PUBACK / UNSUBACK / SUBACK) into
/// (packet_id, property_ids, reason_codes).
fn parse_ack_payload(payload: &[u8]) -> anyhow::Result<(u16, Vec<u8>, Vec<u8>)> {
    if payload.len() < 2 {
        return Err(anyhow::anyhow!("ack payload too short"));
    }
    let packet_id = u16::from_be_bytes([payload[0], payload[1]]);
    let mut cur = 2usize;
    if payload.len() == cur {
        // Remaining length 2: properties absent, implied reason 0x00.
        return Ok((packet_id, Vec::new(), vec![0x00]));
    }
    // property length varint
    let mut proplen: usize = 0;
    let mut shift = 0u32;
    loop {
        if cur >= payload.len() {
            return Err(anyhow::anyhow!("truncated property length"));
        }
        let b = payload[cur];
        cur += 1;
        proplen |= ((b & 0x7F) as usize) << shift;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    let prop_bytes = &payload[cur..(cur + proplen).min(payload.len())];
    let prop_ids = parse_property_ids(prop_bytes)?;
    cur += proplen;
    let reasons = payload[cur..].to_vec();
    Ok((packet_id, prop_ids, if reasons.is_empty() { vec![0x00] } else { reasons }))
}

const REASON_STRING_ID: u8 = 0x1F;
const USER_PROPERTY_ID: u8 = 0x26;

/// G23: with Request Problem Information = 0, UNSUBACK and PUBACK must not
/// carry Reason String / User Property properties.
pub struct RequestProblemInfoV5Test;

impl TestCase for RequestProblemInfoV5Test {
    fn name(&self) -> &str {
        "request_problem_info_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let uid = uuid::Uuid::new_v4().simple().to_string();
        let result: anyhow::Result<()> = (|| {
            let mut stream = raw_connect(&ctx.config.broker_addr, &format!("rpi5-{uid}"), Some(false), None)?;

            // 1) UNSUBSCRIBE a never-subscribed filter -> UNSUBACK (0xB0)
            let mut unsub: Vec<u8> = vec![0xA2];
            let mut body: Vec<u8> = Vec::new();
            body.extend_from_slice(&[0x00, 0x01]); // packet id 1
            body.push(0x00); // property length 0
            body.extend_from_slice(&[0x00, 0x09]); // filter length
            body.extend_from_slice(b"never/sub"); // 9 bytes
            let mut len = body.len();
            loop {
                let mut b = (len % 128) as u8;
                len /= 128;
                if len > 0 {
                    b |= 0x80;
                }
                unsub.push(b);
                if len == 0 {
                    break;
                }
            }
            unsub.extend_from_slice(&body);
            stream.write_all(&unsub)?;
            stream.flush()?;
            let pkt = read_full_packet(&mut stream)?;
            if pkt[0] & 0xF0 != 0xB0 {
                return Err(anyhow::anyhow!("expected UNSUBACK, got {:02x?}", &pkt[..pkt.len().min(4)]));
            }
            let (pid, prop_ids, reasons) = parse_ack_payload(&pkt[2..])?;
            if pid != 1 {
                return Err(anyhow::anyhow!("UNSUBACK packet id mismatch: {pid}"));
            }
            if prop_ids.contains(&REASON_STRING_ID) || prop_ids.contains(&USER_PROPERTY_ID) {
                return Err(anyhow::anyhow!(
                    "UNSUBACK carried forbidden properties {prop_ids:02x?} with Request Problem Information=0 \
                     [MQTT-3.1.2-29]"
                ));
            }
            if !reasons.iter().all(|r| *r == 0x00 || *r == 0x11) {
                return Err(anyhow::anyhow!("UNSUBACK unexpected reason code(s) {reasons:02x?}"));
            }

            // 2) PUBLISH QoS 1 with no matching subscriber -> PUBACK (0x40)
            // Wire order per MQTT 5.0 §3.3.2.1: Topic Name FIRST, then
            // Packet Identifier (QoS > 0), then Properties, then Payload.
            // (The previous version had these two swapped, which made the
            // broker read the packet id as a topic length -> Malformed
            // Packet -> the connection was closed without a PUBACK.)
            let mut pub_body: Vec<u8> = Vec::new();
            pub_body.extend_from_slice(&[0x00, 0x08]); // topic length
            pub_body.extend_from_slice(b"nosubs/t"); // 8 bytes, no subscriber
            pub_body.extend_from_slice(&[0x00, 0x02]); // packet id 2
            pub_body.push(0x00); // property length 0
            pub_body.extend_from_slice(b"x"); // payload
            let mut pub_pkt = vec![0x32]; // PUBLISH QoS 1
            let mut len = pub_body.len();
            loop {
                let mut b = (len % 128) as u8;
                len /= 128;
                if len > 0 {
                    b |= 0x80;
                }
                pub_pkt.push(b);
                if len == 0 {
                    break;
                }
            }
            pub_pkt.extend_from_slice(&pub_body);
            stream.write_all(&pub_pkt)?;
            stream.flush()?;
            let pkt = read_full_packet(&mut stream)?;
            if pkt[0] & 0xF0 != 0x40 {
                return Err(anyhow::anyhow!("expected PUBACK, got {:02x?}", &pkt[..pkt.len().min(4)]));
            }
            let (pid, prop_ids, reasons) = parse_ack_payload(&pkt[2..])?;
            if pid != 2 {
                return Err(anyhow::anyhow!("PUBACK packet id mismatch: {pid}"));
            }
            if prop_ids.contains(&REASON_STRING_ID) || prop_ids.contains(&USER_PROPERTY_ID) {
                return Err(anyhow::anyhow!(
                    "PUBACK carried forbidden properties {prop_ids:02x?} with Request Problem Information=0 \
                     [MQTT-3.1.2-29]"
                ));
            }
            if !reasons.iter().all(|r| *r == 0x00 || *r == 0x10) {
                return Err(anyhow::anyhow!("PUBACK unexpected reason code(s) {reasons:02x?}"));
            }
            Ok(())
        })();

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(20)
    }
}

/// G24 (record-type): with Request Response Information = 1 the CONNACK MAY
/// carry Response Information (0x1A). rmqtt is expected not to support it,
/// which is compliant (MAY). Records the observation without asserting.
pub struct ConnackResponseInfoV5Test;

impl TestCase for ConnackResponseInfoV5Test {
    fn name(&self) -> &str {
        "connack_response_info_v5"
    }

    fn expectation(&self) -> Expectation {
        Expectation::Info
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let uid = uuid::Uuid::new_v4().simple().to_string();
        let observation: anyhow::Result<String> = (|| -> anyhow::Result<String> {
            let mut stream = TcpStream::connect(&ctx.config.broker_addr)?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            let pkt = raw_connect_with_props(&format!("rri5-{uid}"), None, Some(true));
            stream.write_all(&pkt)?;
            stream.flush()?;
            let connack = read_full_packet(&mut stream)?;
            if connack.len() < 4 || connack[0] != 0x20 || connack[3] != 0 {
                return Err(anyhow::anyhow!("CONNECT refused: {:02x?}", &connack[..connack.len().min(8)]));
            }
            // CONNACK payload: flags(1) + reason(1) + proplen varint + props
            let mut cur = 2usize; // skip flags + reason
            let mut proplen: usize = 0;
            let mut shift = 0u32;
            loop {
                let b = connack[cur];
                cur += 1;
                proplen |= ((b & 0x7F) as usize) << shift;
                if b & 0x80 == 0 {
                    break;
                }
                shift += 7;
            }
            let props = &connack[cur..(cur + proplen).min(connack.len())];
            let prop_ids = parse_property_ids(props)?;
            if prop_ids.contains(&0x1A) {
                Ok("broker returned Response Information (0x1A) in CONNACK".to_string())
            } else {
                Ok(format!(
                    "broker omitted Response Information from CONNACK (props {prop_ids:02x?}) — MAY level, compliant"
                ))
            }
        })();

        let observation = match observation {
            Ok(s) => s,
            Err(e) => format!("observation failed: {e}"),
        };
        TestResult::passed_with_note(self.name(), "functional_v5", start.elapsed(), &observation)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }
}
