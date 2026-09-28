//! Session takeover racing a session whose task is busy and cannot answer
//!
//! ## The defect
//!
//! `LockEntry::kick` (`rmqtt/src/shared.rs`) asks the outgoing session to stop
//! by pushing a `Message::Kick(oneshot::Sender<()>, ..)` into its channel and
//! then waits up to 5s for the ack. The ack is sent by the session task when it
//! reads the message (`session.rs`, `Message::Kick`). If the task ends without
//! ever reading it, the queued Kick — and with it the oneshot sender — is
//! dropped together with the channel, and the kicker's `rx` resolves to
//! `Err(RecvError)`.
//!
//! The old code turned that into `return Err(..)` before it ever reached
//! `self._remove(..)`, so `v3.rs` / `v5.rs` answered CONNACK 0x03 / 0x88
//! ServerUnavailable: a reconnect the broker is able to serve was refused, and
//! the stale `peers` entry was left behind for the next attempt to trip over.
//! The ack only feeds a log line, and the two sibling paths already degrade the
//! same situation gracefully — the 5s timeout branch only logs, and the cluster
//! implementations map a failed remote kick to `OfflineSession::NotExist`.
//!
//! ## How the unread Kick is produced deterministically
//!
//! A naive "close the socket, reconnect at once" does NOT even reach the kick
//! handshake: the old session's whole teardown (run loop return → `clean()` →
//! entry removal) takes ≲1 ms even with 16384 subscriptions (measured), which
//! is *faster* than the newcomer's TCP connect + CONNECT. The newcomer's kick
//! then finds no `peers` entry and succeeds trivially as
//! `OfflineSession::NotExist` (measured on that pattern: 32 kick calls, 0 with
//! a peer).
//!
//! Instead, every round parks the outgoing session task in a *guaranteed* busy
//! stretch that outlasts the reconnect: the client sends one SUBSCRIBE carrying
//! [`SUBSCRIPTIONS`] topic filters and — without waiting for the SUBACK — shuts
//! the socket down and immediately reconnects with the same ClientId. The
//! broker's session task spends far longer processing the subscription batch
//! (measured: seconds at debug log level, hundreds of ms at info) than the
//! newcomer needs to reach `entry.kick(..)`, so the entry is present, the
//! channel is open, and the Kick is queued behind work the task has not
//! finished. When the task is done it writes the SUBACK into the closed socket
//! (`send_subscribe_ack(..).await?`, `session.rs`), which fails and returns
//! from the run loop with the Kick still unread — exactly the dropped-oneshot
//! situation above. (If that write ever succeeded instead, the task would reach
//! `select!` with both the socket EOF and the queued Kick ready, which still
//! drops the Kick about half the time.) [`ROUNDS`] independent rounds make a
//! miss across all of them vanishingly unlikely.
//!
//! Two window calibrations matter and are measured, not guessed:
//!
//! - the busy stretch must be far longer than the newcomer's ~1 ms handshake
//!   (orders of magnitude, trivially satisfied);
//! - it must also be **shorter than `kick`'s 5s ack timeout**. If the session
//!   task is still busy when that fires, the kick lands in the *timeout*
//!   branch — which the old code also survives — and, worse, its `_remove`
//!   yanks the entry out from under the still-busy session (every remaining
//!   filter then fails with "session is not exist"; observed 9054 such warns
//!   in one run). [`SUBSCRIPTIONS`] is sized so the batch takes well under 5s
//!   at both info and debug log levels, and [`open`]'s read timeout is well
//!   above 5s so the client outlives the broker's kick timeout.
//!
//! ## What this test asserts
//!
//! The takeover's CONNACK MUST be 0x00: whatever the busy session managed to do
//! before it died, "it did not answer my kick" is not a reason to refuse the
//! newcomer. A control arm pings the surviving connection (PINGREQ -> PINGRESP)
//! so a refusal cannot be blamed on a wedged broker.
//!
//! These cases PASS against a fixed broker and FAIL against the old one, so
//! they double as the reproduction for the defect.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream};
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Fixed header byte of a CONNACK packet.
const PKT_CONNACK: u8 = 0x20;
/// Fixed header byte of a SUBSCRIBE packet (QoS 1 fixed-header flags).
const PKT_SUBSCRIBE: u8 = 0x82;
/// Fixed header byte of a PINGREQ packet.
const PKT_PINGREQ: u8 = 0xC0;
/// Fixed header byte of a PINGRESP packet.
const PKT_PINGRESP: u8 = 0xD0;
/// Fixed header byte of a DISCONNECT packet.
const PKT_DISCONNECT: u8 = 0xE0;
/// CONNACK return / reason code 0x00 — the connection was accepted.
const RC_ACCEPTED: u8 = 0x00;

/// Rounds; each is an independent trial of the busy-session takeover above.
const ROUNDS: usize = 8;
/// Topic filters the busy outgoing session subscribes, in a single SUBSCRIBE packet.
///
/// The batch's processing time is the "busy stretch" that outlasts the
/// newcomer's handshake, so it only has to be a few orders of magnitude above
/// the ~1 ms a reconnect needs. It must however stay well **below** `kick`'s 5s
/// ack timeout (see the module docs): 4096 measures ~0.1 s at info log level
/// and ~0.6 s at debug, a ~45 KB packet (the default listener allows 1 MB).
const SUBSCRIPTIONS: usize = 4096;
/// Topic prefix for the filters; kept short because the filters are the only
/// size driver of the SUBSCRIBE packet.
const TOPIC_PREFIX: &str = "kr/";

/// MQTT protocol version under test. The takeover path is protocol-neutral
/// (`shared.rs`), but the CONNACK code a refusal produces is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Proto {
    V311,
    V5,
}

impl Proto {
    fn level(self) -> u8 {
        match self {
            Proto::V311 => 4,
            Proto::V5 => 5,
        }
    }

    fn tag(self) -> &'static str {
        match self {
            Proto::V311 => "v311",
            Proto::V5 => "v5",
        }
    }

    /// CONNACK return / reason code the broker answers with when the takeover
    /// is refused by the `kick` error path this case reproduces.
    fn refusal_code(self) -> u8 {
        match self {
            // ConnectAckReasonV3::ServiceUnavailable
            Proto::V311 => 0x03,
            // ConnectAckReasonV5::ServerUnavailable
            Proto::V5 => 0x88,
        }
    }
}

/// MQTT remaining-length encoding (variable byte integer, MQTT-1.5.5).
fn varint(mut value: usize) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (value % 128) as u8;
        value /= 128;
        if value > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            return out;
        }
    }
}

/// A CONNECT that starts a fresh session: Clean Start (v5) / Clean Session
/// (v3.1.1) set, keep alive 60s, no Will, no username, no properties.
fn connect_packet(proto: Proto, client_id: &str) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&[0x00, 0x04]);
    body.extend_from_slice(b"MQTT");
    body.push(proto.level());
    body.push(0x02); // clean start / clean session
    body.extend_from_slice(&[0x00, 0x3C]); // keep alive 60s
    if proto == Proto::V5 {
        body.push(0x00); // property length = 0
    }
    let cid = client_id.as_bytes();
    body.extend_from_slice(&(cid.len() as u16).to_be_bytes());
    body.extend_from_slice(cid);

    let mut pkt = vec![0x10];
    pkt.extend_from_slice(&varint(body.len()));
    pkt.extend_from_slice(&body);
    pkt
}

/// A SUBSCRIBE carrying `filters` QoS 0 filters under `TOPIC_PREFIX`, packet id 1.
fn subscribe_packet(proto: Proto, filters: usize) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&[0x00, 0x01]); // packet id 1
    if proto == Proto::V5 {
        body.push(0x00); // property length = 0 (v5 only; v3.1.1 has no properties)
    }
    for i in 0..filters {
        let topic = format!("{TOPIC_PREFIX}{i}");
        body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
        body.extend_from_slice(topic.as_bytes());
        body.push(0x00); // requested QoS 0
    }

    let mut pkt = vec![PKT_SUBSCRIBE];
    pkt.extend_from_slice(&varint(body.len()));
    pkt.extend_from_slice(&body);
    pkt
}

/// Read one complete MQTT packet (fixed header + Remaining Length + body).
///
/// `Ok(None)` means the peer closed the connection (EOF). This case reads at
/// most two packets per connection. The timeout is deliberately far above
/// `kick`'s 5s ack timeout: while the takeover waits for the busy session to
/// die, the CONNACK legitimately takes up to ~5s to arrive.
const READ_TIMEOUT: Duration = Duration::from_secs(20);

fn read_packet(stream: &mut TcpStream) -> anyhow::Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    let mut b = [0u8; 1];

    match stream.read(&mut b) {
        Ok(0) => return Ok(None),
        Ok(_) => buf.push(b[0]),
        Err(e) => return Err(e.into()),
    }

    // Remaining Length, a variable byte integer of at most 4 bytes.
    let mut remaining: u32 = 0;
    let mut shift = 0u32;
    loop {
        match stream.read(&mut b) {
            Ok(0) => return Err(anyhow::anyhow!("connection closed mid-header")),
            Ok(_) => {}
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

/// Connect with `client_id`, send the CONNECT and return the stream together
/// with the CONNACK code.
///
/// The code is returned rather than asserted: a refusal is the observation this
/// case is about, and the caller adds the diagnosis.
fn open(broker_addr: &str, proto: Proto, client_id: &str) -> anyhow::Result<(TcpStream, u8)> {
    let mut stream = TcpStream::connect(broker_addr)?;
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.write_all(&connect_packet(proto, client_id))?;
    stream.flush()?;

    let pkt = read_packet(&mut stream)?.ok_or_else(|| {
        anyhow::anyhow!("connect {client_id}: broker closed the connection instead of sending CONNACK")
    })?;
    if pkt.len() < 4 || pkt[0] != PKT_CONNACK {
        return Err(anyhow::anyhow!("connect {client_id}: expected CONNACK, got {:02X?}", pkt));
    }
    Ok((stream, pkt[3]))
}

/// Send a SUBSCRIBE carrying `filters` QoS 0 filters under `TOPIC_PREFIX`,
/// packet id 1, and do NOT wait for the SUBACK: the whole point is that the
/// broker's session task stays busy processing the batch while this client
/// closes and reconnects.
fn send_subscribe(stream: &mut TcpStream, proto: Proto, filters: usize) -> anyhow::Result<()> {
    stream.write_all(&subscribe_packet(proto, filters))?;
    stream.flush()?;
    Ok(())
}

/// Control arm: the connection must still be serviced (PINGREQ -> PINGRESP).
fn ping_expect_pingresp(stream: &mut TcpStream) -> anyhow::Result<()> {
    stream.write_all(&[PKT_PINGREQ, 0x00])?;
    stream.flush()?;
    match read_packet(stream)? {
        Some(pkt) if pkt[0] == PKT_PINGRESP => Ok(()),
        Some(pkt) => Err(anyhow::anyhow!("expected PINGRESP (0xD0), got {:02X?}", pkt)),
        None => Err(anyhow::anyhow!("connection closed while waiting for PINGRESP")),
    }
}

/// Close a connection the way a client does when it is not shutting down:
/// no DISCONNECT, just a shut-down socket.
fn abrupt_close(stream: TcpStream) {
    let _ = stream.shutdown(Shutdown::Both);
}

/// Run `ROUNDS` busy-session takeover rounds.
fn run_race(ctx: &TestContext, proto: Proto) -> anyhow::Result<()> {
    let broker_addr = &ctx.config.broker_addr;
    let uid = uuid::Uuid::new_v4().simple().to_string();

    for round in 1..=ROUNDS {
        let client_id = format!("kick-race-{}-{round}-{uid}", proto.tag());

        // The outgoing session: a fresh clean-start connection whose task is
        // parked processing a huge subscription batch (see the module docs for
        // why that — and not a teardown window — is what makes the race hit).
        let (mut old, code) = open(broker_addr, proto, &client_id)?;
        if code != RC_ACCEPTED {
            return Err(anyhow::anyhow!(
                "round {round}/{ROUNDS}: the *first* connect of {client_id} was already refused \
                 with CONNACK 0x{code:02X} — the broker does not accept a plain clean-start \
                 connection, so this case cannot observe a takeover at all"
            ));
        }
        send_subscribe(&mut old, proto, SUBSCRIPTIONS).map_err(|e| {
            anyhow::anyhow!(
                "round {round}/{ROUNDS}: sending the {SUBSCRIPTIONS}-filter SUBSCRIBE failed: {e}"
            )
        })?;

        // Drop it without a DISCONNECT — and without waiting for the SUBACK,
        // which is what parks the broker's session task — and take the ClientId
        // over right away: the retry pattern of a client that just lost its
        // socket.
        abrupt_close(old);
        let (mut new, code) = open(broker_addr, proto, &client_id)?;

        if code != RC_ACCEPTED {
            let hint = if code == proto.refusal_code() {
                "that is the ServerUnavailable code `v3.rs` / `v5.rs` answer with when \
                 `LockEntry::kick` returns an error"
            } else {
                "not the ServerUnavailable code this case reproduces, but still a refusal"
            };
            return Err(anyhow::anyhow!(
                "round {round}/{ROUNDS}: taking over {client_id} was refused with CONNACK \
                 0x{code:02X} — {hint}. The previous connection was closed abruptly in the middle \
                 of its {SUBSCRIPTIONS}-filter subscribe, so its session task was still busy when \
                 the takeover's `Message::Kick` was queued: the task never read it (it died writing \
                 the SUBACK into the closed socket), the oneshot ack came back as `RecvError`, and \
                 `LockEntry::kick` (`rmqtt/src/shared.rs`) must treat that the way it treats the 5s \
                 timeout and the way the cluster implementations treat a failed remote kick: log \
                 it, then fall through to `self._remove(..)` and let the takeover complete. The \
                 ack only feeds a log line, so it must not decide the outcome — the session being \
                 replaced is already gone, and refusing the newcomer leaves its stale entry behind"
            ));
        }

        // Control arm: the takeover succeeded *and* the new connection is
        // serviced, so nothing below can be blamed on a wedged broker.
        ping_expect_pingresp(&mut new).map_err(|e| {
            anyhow::anyhow!("round {round}/{ROUNDS}: the new connection is not serviced: {e}")
        })?;

        // Tidy close: this round is done with the ClientId.
        let _ = new.write_all(&[PKT_DISCONNECT, 0x00]);
        let _ = new.flush();
        abrupt_close(new);
    }
    Ok(())
}

/// v3.1.1: a takeover of a session whose task is busy must not be refused with
/// 0x03 ServerUnavailable.
pub struct KickRaceSessionTakeoverV311Test;

impl TestCase for KickRaceSessionTakeoverV311Test {
    fn name(&self) -> &str {
        "kick_race_session_takeover_v311"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        match run_race(ctx, Proto::V311) {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(120)
    }
}

/// MQTT 5.0: the same observation, where a refused takeover is 0x88
/// ServerUnavailable.
pub struct KickRaceSessionTakeoverV5Test;

impl TestCase for KickRaceSessionTakeoverV5Test {
    fn name(&self) -> &str {
        "kick_race_session_takeover_v5"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        match run_race(ctx, Proto::V5) {
            Ok(()) => TestResult::passed(self.name(), "functional_v5", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v5", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(120)
    }
}
