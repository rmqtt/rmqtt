//! WebSocket frame-boundary cases (issue #551).
//!
//! #551: a zero-length binary WebSocket message made the WS listener drop the
//! connection. `WsStream::poll_read` (rmqtt-net/src/ws.rs) reported the empty
//! frame as a *successful* read of zero bytes, which is the `AsyncRead`
//! contract for EOF, so `Framed` ended the stream and the session was torn down
//! without a closing handshake (the client observes `close 1005`).
//!
//! Three further consequences of the same code path are covered here, because
//! they shared the root cause and each had a different blast radius:
//!
//! * an empty WS **Ping** (what most WS keepalives send) killed the session;
//! * a **non-empty control frame** had its payload spliced into the MQTT byte
//!   stream, because `Message::into_data()` was applied to every frame type;
//! * an empty frame **between** the fragments of one packet was enough to end
//!   the stream, so frames cannot be treated as EOF-ish noise either.
//!
//! Every case speaks MQTT over a **raw** `WebSocketStream` instead of the
//! `WsStream` byte adapter: the adapter hides frame boundaries, and the copy of
//! it in `transport_tls.rs` carried the same defect, so a case built on either
//! could not tell a broken broker from a broken client.
//!
//! The four cases ran red against 0.24.0/0.25.0 and are the acceptance
//! criteria for the fix; they are registered in the `functional_transport`
//! suite and stay as the guard for that read loop:
//!
//! ```text
//! ./target/debug/mqtt_harness --workspace . --suites functional_transport -t issue551
//! ```

use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use bytestring::ByteString;
use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_util::codec::{Decoder, Encoder};

use rmqtt_codec::types::{Protocol, MQTT_LEVEL_311};
use rmqtt_codec::v3::{Codec as V3Codec, Connect, ConnectAckReason, Packet as PacketV3};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

use super::transport_tls::{connect_ws, spawn_transport_broker};

/// Deadline for the broker to answer a packet (generous: a red case must fail
/// on the assertion, not on a tight timer).
const REPLY_TIMEOUT: Duration = Duration::from_secs(3);
/// Window used to assert that the broker stayed silent.
const SILENCE_WINDOW: Duration = Duration::from_millis(1200);

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

// ---------------------------------------------------------------------------
// Raw MQTT-over-WebSocket client (control-frame aware)
// ---------------------------------------------------------------------------

/// MQTT session over a raw WebSocket: every outbound packet is written as one
/// Binary message, inbound control frames are skipped instead of being read as
/// MQTT bytes.
struct RawWsMqtt {
    ws: Ws,
    codec: V3Codec,
    buf: BytesMut,
}

impl RawWsMqtt {
    /// Opens the connection without sending CONNECT (for cases that must build
    /// the CONNECT frame themselves).
    async fn open() -> anyhow::Result<Self> {
        Ok(Self { ws: connect_ws().await?, codec: V3Codec::new(0), buf: BytesMut::new() })
    }

    /// Opens the connection, sends CONNECT and requires a CONNACK 0x00.
    async fn connect(client_id: &str) -> anyhow::Result<Self> {
        let mut this = Self::open().await?;
        this.ws.send(Message::Binary(connect_frame(client_id)?.freeze())).await?;
        match this.recv_packet(REPLY_TIMEOUT).await? {
            Some(PacketV3::ConnectAck(ack)) if ack.return_code == ConnectAckReason::ConnectionAccepted => {
                Ok(this)
            }
            Some(PacketV3::ConnectAck(ack)) => {
                anyhow::bail!("CONNECT rejected: {:?}", ack.return_code)
            }
            Some(other) => anyhow::bail!("expected CONNACK, got {other:?}"),
            None => anyhow::bail!("no CONNACK within {REPLY_TIMEOUT:?}"),
        }
    }

    /// Writes one pre-built WebSocket frame (empty data frame, Ping, ...).
    async fn send_msg(&mut self, msg: Message) -> anyhow::Result<()> {
        self.ws.send(msg).await?;
        Ok(())
    }

    async fn send_packet(&mut self, pkt: PacketV3) -> anyhow::Result<()> {
        let mut out = BytesMut::new();
        self.codec.encode(pkt, &mut out)?;
        self.ws.send(Message::Binary(out.freeze())).await?;
        Ok(())
    }

    /// Reads one MQTT packet; `Ok(None)` means nothing arrived inside
    /// `timeout`. Control frames are consumed and skipped, Close/reset is an
    /// error (the session is gone — the #551 symptom).
    async fn recv_packet(&mut self, timeout: Duration) -> anyhow::Result<Option<PacketV3>> {
        loop {
            if let Some((pkt, _consumed)) = self.codec.decode(&mut self.buf)? {
                return Ok(Some(pkt));
            }
            let msg = match tokio::time::timeout(timeout, self.ws.next()).await {
                Err(_) => return Ok(None),
                Ok(None) => {
                    anyhow::bail!("WebSocket stream ended with no frame: the broker dropped the session")
                }
                Ok(Some(Err(e))) => anyhow::bail!("WebSocket error instead of a packet: {e}"),
                Ok(Some(Ok(msg))) => msg,
            };
            match msg {
                Message::Binary(data) => self.buf.extend_from_slice(data.as_ref()),
                Message::Text(text) => self.buf.extend_from_slice(text.as_str().as_bytes()),
                // Broker keepalives are not MQTT bytes.
                Message::Ping(_) | Message::Pong(_) => continue,
                Message::Frame(_) => continue,
                Message::Close(frame) => anyhow::bail!("broker closed the WebSocket: {frame:?}"),
            }
        }
    }

    /// Asserts the broker sends nothing for `window`.
    async fn expect_silence(&mut self, window: Duration) -> anyhow::Result<()> {
        match self.recv_packet(window).await? {
            Some(pkt) => anyhow::bail!("expected no packet, got {pkt:?}"),
            None => Ok(()),
        }
    }
}

/// Encodes a MQTT 3.1.1 CONNECT for `client_id`.
fn connect_frame(client_id: &str) -> anyhow::Result<BytesMut> {
    let conn = Connect {
        protocol: Protocol(MQTT_LEVEL_311),
        clean_session: true,
        keep_alive: 30,
        last_will: None,
        client_id: ByteString::from(client_id),
        username: None,
        password: None,
        cert: None,
    };
    let mut out = BytesMut::new();
    V3Codec::new(0).encode(PacketV3::Connect(Box::new(conn)), &mut out)?;
    Ok(out)
}

fn uid() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// The issue as reported: CONNECT → CONNACK → **empty binary frame** → PINGREQ
/// must still be answered with PINGRESP (the empty message carries no MQTT
/// bytes and RFC 6455 permits it).
pub struct Issue551WsEmptyFrameTest;

impl TestCase for Issue551WsEmptyFrameTest {
    fn name(&self) -> &str {
        "issue551_ws_empty_frame"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let (_node, _binary) = spawn_transport_broker()?;
            let mut mqtt = RawWsMqtt::connect(&format!("ws551-empty-{}", uid())).await?;

            // Exactly what paho.golang does for SUBSCRIBE: an empty write.
            mqtt.send_msg(Message::Binary(Bytes::new())).await?;
            mqtt.send_packet(PacketV3::PingRequest).await?;

            match mqtt.recv_packet(REPLY_TIMEOUT).await? {
                Some(PacketV3::PingResponse) => Ok::<(), anyhow::Error>(()),
                Some(other) => anyhow::bail!("expected PINGRESP after an empty frame, got {other:?}"),
                None => anyhow::bail!(
                    "no PINGRESP within {REPLY_TIMEOUT:?}: the zero-length binary frame closed the \
                     session (issue #551)"
                ),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_transport", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_transport", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// A WS-level **Ping with an empty payload** — what most WebSocket keepalives
/// send — must not disturb the MQTT session either. Same code path, wider blast
/// radius than the reported case: any client-side WS ping would reconnect-loop.
pub struct Issue551WsEmptyPingTest;

impl TestCase for Issue551WsEmptyPingTest {
    fn name(&self) -> &str {
        "issue551_ws_empty_ping"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let (_node, _binary) = spawn_transport_broker()?;
            let mut mqtt = RawWsMqtt::connect(&format!("ws551-ping-{}", uid())).await?;

            mqtt.send_msg(Message::Ping(Bytes::new())).await?;
            mqtt.send_packet(PacketV3::PingRequest).await?;

            match mqtt.recv_packet(REPLY_TIMEOUT).await? {
                Some(PacketV3::PingResponse) => Ok::<(), anyhow::Error>(()),
                Some(other) => anyhow::bail!("expected PINGRESP after an empty WS Ping, got {other:?}"),
                None => anyhow::bail!(
                    "no PINGRESP within {REPLY_TIMEOUT:?}: an empty WS Ping closed the session \
                     (issue #551)"
                ),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_transport", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_transport", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// A control frame's payload must never reach the MQTT decoder. The Ping here
/// carries a complete PINGREQ wire image (`C0 00`); if the adapter splices it
/// into the byte stream the broker answers it, and the real PINGREQ that
/// follows produces a *second* PINGRESP.
pub struct Issue551WsPingPayloadTest;

impl TestCase for Issue551WsPingPayloadTest {
    fn name(&self) -> &str {
        "issue551_ws_ping_payload"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let (_node, _binary) = spawn_transport_broker()?;
            let mut mqtt = RawWsMqtt::connect(&format!("ws551-payload-{}", uid())).await?;

            // A PINGREQ hidden inside a Ping payload.
            mqtt.send_msg(Message::Ping(Bytes::from_static(&[0xC0, 0x00]))).await?;
            mqtt.send_packet(PacketV3::PingRequest).await?;

            match mqtt.recv_packet(REPLY_TIMEOUT).await? {
                Some(PacketV3::PingResponse) => {}
                Some(other) => anyhow::bail!("expected PINGRESP, got {other:?}"),
                None => anyhow::bail!("no PINGRESP within {REPLY_TIMEOUT:?}"),
            }
            // Exactly one PINGRESP: the one answering the real PINGREQ.
            match mqtt.recv_packet(SILENCE_WINDOW).await? {
                Some(PacketV3::PingResponse) => anyhow::bail!(
                    "second PINGRESP: the Ping payload was spliced into the MQTT byte stream and \
                     decoded as a PINGREQ (issue #551)"
                ),
                Some(other) => anyhow::bail!("expected silence, got {other:?}"),
                None => Ok::<(), anyhow::Error>(()),
            }
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_transport", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_transport", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// An empty frame **inside** a split packet. The case carries its own control:
/// the same CONNECT split over two non-empty frames must be accepted (the
/// reporter notes splitting works), and only the variant with a zero-length
/// frame in between must fail — otherwise the split, not the empty frame, would
/// be the suspect.
pub struct Issue551WsSplitWithEmptyFrameTest;

impl TestCase for Issue551WsSplitWithEmptyFrameTest {
    fn name(&self) -> &str {
        "issue551_ws_split_connect"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let (_node, _binary) = spawn_transport_broker()?;

            // Control: two non-empty frames, nothing in between.
            let control = send_split_connect(None).await?;
            if control != ConnectAckReason::ConnectionAccepted {
                anyhow::bail!(
                    "control failed: a CONNECT split over two non-empty frames was answered with \
                     {control:?}; splitting a packet across frames is supposed to work"
                );
            }

            // Repro: the same split, with a zero-length frame in between.
            let repro = send_split_connect(Some(Message::Binary(Bytes::new()))).await?;
            if repro != ConnectAckReason::ConnectionAccepted {
                anyhow::bail!(
                    "CONNECT split over two frames with an empty frame in between was answered with \
                     {repro:?} while the control got {control:?}: the empty frame ended the stream \
                     mid-packet (issue #551)"
                );
            }
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_transport", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_transport", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// Writes one CONNECT as two non-empty frames, optionally with `interlude`
/// between them, and returns the CONNACK reason code.
async fn send_split_connect(interlude: Option<Message>) -> anyhow::Result<ConnectAckReason> {
    let mut mqtt = RawWsMqtt::open().await?;
    let frame = connect_frame(&format!("ws551-split-{}", uid()))?;
    let (head, tail) = frame.split_at(frame.len() / 2);

    mqtt.send_msg(Message::Binary(Bytes::copy_from_slice(head))).await?;
    if let Some(msg) = interlude {
        mqtt.send_msg(msg).await?;
    }
    mqtt.send_msg(Message::Binary(Bytes::copy_from_slice(tail))).await?;

    match mqtt.recv_packet(REPLY_TIMEOUT).await? {
        Some(PacketV3::ConnectAck(ack)) => Ok(ack.return_code),
        Some(other) => anyhow::bail!("expected CONNACK, got {other:?}"),
        None => anyhow::bail!("no CONNACK within {REPLY_TIMEOUT:?}"),
    }
}
