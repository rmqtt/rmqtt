//! Server-side DISCONNECT delivery (MQTT 5.0)
//!
//! MQTT 5.0 gives the Server a set of Reason Codes that only the Server can
//! send — 0x8D Keep Alive timeout, 0x8E Session taken over, 0x93 Receive
//! Maximum exceeded, 0x95 Packet too large, 0x81/0x82 for malformed packets.
//! Their whole purpose is to let a Client tell "the Server hung up on me, and
//! here is why" apart from "the network died". That only works if the
//! DISCONNECT packet actually reaches the wire.
//!
//! ## The requirement
//!
//! * [MQTT-3.1.4-3] (MUST): "If the ClientID represents a Client already
//!   connected to the Server, the Server sends a DISCONNECT packet to the
//!   existing Client with Reason Code of 0x8E (Session taken over) as
//!   described in section 4.13 and MUST close the Network Connection of the
//!   existing Client."
//! * MQTT 5.0 §4.13.1 (SHOULD): "In the case of errors in other packets it
//!   SHOULD send a DISCONNECT packet containing a Reason Code before closing
//!   the Network Connection."
//!
//! ## What the broker used to do instead
//!
//! `Session::run` tore the connection down before it explained why: it called
//! `sink.close()` — which drives `Framed::close()`, i.e. `poll_flush()` +
//! `poll_shutdown()`, sending the FIN — *before* building and sending the v5
//! DISCONNECT. `send_disconnect` is the only place the Server ever emits one
//! (`grep -n send_disconnect rmqtt/src` returns exactly one hit), so it could
//! only fail, and the failure was discarded by a `let _ =`: a bare FIN on the
//! wire, and no log line either. Because that call site is the only one, every
//! Server-originated Reason Code was dead at once — 0x8D, 0x8E, 0x93, 0x95 and
//! 0x81/0x82 alike. The ordering was lost in `a70360078` ("Move `sink.close()`
//! earlier in session termination process").
//!
//! ## What this test asserts
//!
//! Two raw TCP connections share one Client ID. The first must receive a
//! DISCONNECT carrying Reason Code 0x8E and only then be closed; a broker that
//! closes first FAILs, and one that explains itself PASSes. Ordering and reason
//! code are asserted separately, so a partial fix is still reported precisely:
//! against a broker that sends 0x8E before closing the test PASSes, and
//! against one that sends 0x87 it fails on the reason code alone.
//!
//! A control arm runs in the same test: the second connection is pinged
//! (PINGREQ -> PINGRESP) to prove the broker itself is healthy, so a missing
//! DISCONNECT cannot be blamed on a broker that crashed.
//!
//! NOTE: the reason code is asserted because the takeover path has a trap of
//! its own. `rmqtt/src/v5.rs` kicks the existing session with `is_admin =
//! false`, so the code comes from `Reason::ConnectKicked(false)`, where "not an
//! administrative kick" means "displaced by a session takeover": [MQTT-3.1.4-3]
//! requires 0x8E there, not 0x87 NotAuthorized, which would claim the Client
//! was rejected when it was in fact displaced.

use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Fixed header byte of an MQTT DISCONNECT packet.
const PKT_DISCONNECT: u8 = 0xE0;
/// Fixed header byte of an MQTT PINGRESP packet.
const PKT_PINGRESP: u8 = 0xD0;
/// Reason Code 0x8E, "Session taken over" (MQTT 5.0 table 3-10).
const RC_SESSION_TAKEN_OVER: u8 = 0x8E;

/// Build a raw MQTT v5 CONNECT ("MQTT" / level 5, clean start, no properties).
fn raw_connect_v5(client_id: &str) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&[0x00, 0x04]);
    body.extend_from_slice(b"MQTT");
    body.push(5); // protocol level
    body.push(0x02); // clean start
    body.extend_from_slice(&[0x00, 0x3C]); // keep alive 60s
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

/// Read one complete MQTT packet (fixed header + Remaining Length + body).
///
/// Returns `Ok(None)` when the peer closed the connection (EOF), and `Err`
/// when `wait` elapses with the connection still open but silent — the two
/// outcomes must stay distinguishable, because "closed without a DISCONNECT"
/// and "never closed at all" are different protocol violations.
fn read_packet_or_eof(stream: &mut TcpStream, wait: Duration) -> anyhow::Result<Option<Vec<u8>>> {
    let deadline = Instant::now() + wait;
    let mut buf = Vec::new();
    let mut b = [0u8; 1];

    // Fixed header byte.
    loop {
        match stream.read(&mut b) {
            Ok(0) => return Ok(None),
            Ok(_) => break,
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if Instant::now() >= deadline {
                    return Err(anyhow::anyhow!("timed out after {wait:?} with the connection still open"));
                }
            }
            Err(e) => return Err(e.into()),
        }
    }
    buf.push(b[0]);

    // Remaining Length, a variable byte integer of at most 4 bytes.
    let mut remaining: u32 = 0;
    let mut shift = 0u32;
    loop {
        match stream.read(&mut b) {
            Ok(0) => return Err(anyhow::anyhow!("connection closed mid-header")),
            Ok(_) => {}
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if Instant::now() >= deadline {
                    return Err(anyhow::anyhow!(
                        "timed out after {wait:?} while reading the Remaining Length"
                    ));
                }
                continue;
            }
            Err(e) => return Err(e.into()),
        }
        buf.push(b[0]);
        remaining |= ((b[0] & 0x7F) as u32) << shift;
        if b[0] & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift > 21 {
            return Err(anyhow::anyhow!("malformed Remaining Length"));
        }
    }

    let mut rest = vec![0u8; remaining as usize];
    stream.read_exact(&mut rest)?;
    buf.extend_from_slice(&rest);
    Ok(Some(buf))
}

/// Open a raw TCP connection, send a valid v5 CONNECT and consume the CONNACK.
/// The read timeout is kept short so `read_packet_or_eof` can poll against its
/// own deadline.
fn raw_connect(broker_addr: &str, client_id: &str) -> anyhow::Result<TcpStream> {
    let mut stream = TcpStream::connect(broker_addr)?;
    stream.set_read_timeout(Some(Duration::from_millis(200)))?;
    stream.write_all(&raw_connect_v5(client_id))?;
    stream.flush()?;

    let connack = read_packet_or_eof(&mut stream, Duration::from_secs(8))?
        .ok_or_else(|| anyhow::anyhow!("broker closed the connection instead of sending CONNACK"))?;
    if connack.len() < 4 || connack[0] != 0x20 {
        return Err(anyhow::anyhow!("expected CONNACK, got {:02X?}", connack));
    }
    if connack[3] != 0x00 {
        return Err(anyhow::anyhow!("CONNECT refused with reason code 0x{:02X}", connack[3]));
    }
    Ok(stream)
}

/// Control arm: the connection must still be serviced (PINGREQ -> PINGRESP).
fn ping_expect_pingresp(stream: &mut TcpStream) -> anyhow::Result<()> {
    stream.write_all(&[0xC0, 0x00])?;
    stream.flush()?;
    match read_packet_or_eof(stream, Duration::from_secs(5))? {
        Some(p) if p.first() == Some(&PKT_PINGRESP) => Ok(()),
        Some(p) => Err(anyhow::anyhow!("expected PINGRESP (0xD0), got {:02X?}", p)),
        None => Err(anyhow::anyhow!("connection closed while waiting for PINGRESP")),
    }
}

/// Session takeover must tell the old connection why it is being dropped:
/// DISCONNECT with Reason Code 0x8E, then a close. [MQTT-3.1.4-3]
pub struct TakeoverSendsDisconnect0x8eV5Test;

impl TakeoverSendsDisconnect0x8eV5Test {
    fn run(&self, ctx: &TestContext) -> anyhow::Result<()> {
        let uid = uuid::Uuid::new_v4().simple().to_string();
        let client_id = format!("takeover-disc5-{uid}");

        // First connection owns the Client ID.
        let mut old = raw_connect(&ctx.config.broker_addr, &client_id)?;

        // Second connection with the SAME Client ID takes the session over.
        let mut new = raw_connect(&ctx.config.broker_addr, &client_id)?;

        // Control arm: the broker is alive and still serving this Client ID,
        // so a missing DISCONNECT on the old connection is not collateral
        // damage from a broker that stopped working.
        ping_expect_pingresp(&mut new)?;

        // The old connection must be handed a DISCONNECT before it is closed.
        let pkt = match read_packet_or_eof(&mut old, Duration::from_secs(5)) {
            Ok(Some(pkt)) => pkt,
            Ok(None) => {
                return Err(anyhow::anyhow!(
                    "[MQTT-3.1.4-3]: taking over client id '{client_id}' closed the first \
                     connection with a bare FIN — EOF with no DISCONNECT packet. The Server MUST \
                     send a DISCONNECT packet with Reason Code 0x8E (Session taken over) to the \
                     existing Client before closing its Network Connection, so that the Client \
                     can tell 'taken over' from 'network dropped'. The DISCONNECT is written at \
                     the single `send_disconnect` call site in `Session::run`, which has to run \
                     while the sink's write half is still open"
                ));
            }
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "BUG REPRODUCED [MQTT-3.1.4-3]: after the takeover the first connection was \
                     neither sent a DISCONNECT nor closed ({e}) — the Server MUST close the \
                     Network Connection of the taken-over Client"
                ));
            }
        };

        if pkt[0] != PKT_DISCONNECT {
            return Err(anyhow::anyhow!(
                "[MQTT-3.1.4-3]: expected a DISCONNECT (0xE0) after the takeover, got {:02X?}",
                pkt
            ));
        }

        // A DISCONNECT body is at least [Reason Code, Property Length]; an
        // absent Reason Code means 0x00 Normal disconnection.
        let reason = pkt.get(2).copied().unwrap_or(0x00);
        if reason != RC_SESSION_TAKEN_OVER {
            return Err(anyhow::anyhow!(
                "[MQTT-3.1.4-3]: the DISCONNECT after the takeover carries Reason Code \
                 0x{reason:02X}, expected 0x8E (Session taken over). The ordering is right, but \
                 the code is wrong: the takeover path kicks the existing session with \
                 `is_admin = false`, which must map to 0x8E. 0x87 NotAuthorized would tell the \
                 Client it was rejected when it was in fact displaced"
            ));
        }

        // ... and then the connection must actually be closed.
        match read_packet_or_eof(&mut old, Duration::from_secs(3)) {
            Ok(None) => Ok(()),
            Ok(Some(p)) => Err(anyhow::anyhow!(
                "[MQTT-3.1.4-3]: after the DISCONNECT the connection stayed open and sent {:02X?}; \
                 the Server MUST close the Network Connection of the taken-over Client",
                p
            )),
            Err(e) => Err(anyhow::anyhow!(
                "[MQTT-3.1.4-3]: after the DISCONNECT the connection was not closed ({e})"
            )),
        }
    }
}

impl TestCase for TakeoverSendsDisconnect0x8eV5Test {
    fn name(&self) -> &str {
        "takeover_sends_disconnect_0x8e_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        match self.run(ctx) {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}
