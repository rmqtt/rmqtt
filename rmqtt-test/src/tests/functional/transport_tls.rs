//! TLS / WebSocket transport tests (P2 G27)
//!
//! Verifies that MQTT 3.1.1 works over the non-TCP transports rmqttd
//! supports: TLS (8883), plain WebSocket (8080) and TLS-WebSocket / WSS
//! (8443), plus mutual TLS on the TLS listener.
//!
//! Runs against a self-managed broker (`configs/transport-tls/`) which
//! enables the tls / ws / wss listeners with mTLS on the TLS/WSS ports
//! (server requests a client certificate validated against root.pem).
//! Certificates are the repository's own test certs in `rmqtt-bin/`.

use std::io;
use std::num::NonZeroU16;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use bytestring::ByteString;
use futures::{ready, Sink, Stream};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{Connector, WebSocketStream};
use tokio_util::codec::{Decoder, Encoder};

use rmqtt_codec::types::{Protocol, Publish, MQTT_LEVEL_311};
use rmqtt_codec::v3::{Codec as V3Codec, Connect, ConnectAckReason, Packet as PacketV3, QoS};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::tests::functional::cluster_session_restart::{rmqttd_binary, ClusterNode};

// Self-managed broker ports (see configs/transport-tls/rmqtt.toml). These
// deliberately avoid the default broker's TLS/WS/WSS listeners (8883/8080/
// 8443), which are always enabled by configs/default/rmqtt.toml.
const TLS_ADDR: &str = "127.0.0.1:1897";
const WS_ADDR: &str = "127.0.0.1:1898";
const WSS_ADDR: &str = "127.0.0.1:1899";
const TRANSPORT_NODE_START_TIMEOUT: Duration = Duration::from_secs(20);

const CERT_ROOT: &str = "rmqtt-bin/root.pem";
const CERT_CLIENT: &str = "rmqtt-bin/client.pem";
const KEY_CLIENT: &str = "rmqtt-bin/client.key";
const SERVER_NAME: &str = "localhost"; // SAN of rmqtt-bin/rmqtt.pem

// ---------------------------------------------------------------------------
// rustls client configuration
// ---------------------------------------------------------------------------

fn load_certs(path: &str) -> anyhow::Result<Vec<CertificateDer<'static>>> {
    let certs: Result<Vec<_>, _> = CertificateDer::pem_file_iter(path)?.collect();
    Ok(certs?)
}

/// Build a rustls ClientConfig trusting `rmqtt-bin/root.pem`. When
/// `with_client_cert` is true the client also presents
/// `rmqtt-bin/client.pem` (mTLS).
fn rustls_config(with_client_cert: bool) -> anyhow::Result<ClientConfig> {
    rustls_config_for(CERT_ROOT, with_client_cert.then_some((CERT_CLIENT, KEY_CLIENT)))
}

/// Build a rustls ClientConfig trusting `root_pem`, optionally presenting the
/// client certificate chain `client_pem` with its key `client_key`.
pub(crate) fn rustls_config_for(
    root_pem: &str,
    client: Option<(&str, &str)>,
) -> anyhow::Result<ClientConfig> {
    let mut roots = RootCertStore::empty();
    for cert in load_certs(root_pem)? {
        roots.add(cert)?;
    }
    let builder = ClientConfig::builder().with_root_certificates(roots);
    if let Some((client_pem, client_key)) = client {
        let chain = load_certs(client_pem)?;
        let key = PrivateKeyDer::from_pem_file(client_key).map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(builder.with_client_auth_cert(chain, key)?)
    } else {
        Ok(builder.with_no_client_auth())
    }
}

// ---------------------------------------------------------------------------
// Stream helpers
// ---------------------------------------------------------------------------

/// Connect to the TLS listener (8883) with the given client config.
async fn connect_tls(cfg: ClientConfig) -> anyhow::Result<TlsStream<TcpStream>> {
    let connector = TlsConnector::from(Arc::new(cfg));
    let stream = TcpStream::connect(TLS_ADDR).await?;
    let server_name = ServerName::try_from(SERVER_NAME).map_err(|_| anyhow::anyhow!("bad server name"))?;
    let tls = connector.connect(server_name, stream).await?;
    Ok(tls)
}

/// Connect to a plain WebSocket (8080) with the MQTT sub-protocol header.
async fn connect_ws() -> anyhow::Result<WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>> {
    connect_ws_inner(WS_ADDR, false).await
}

/// Connect to the WSS listener (8443) with mTLS.
async fn connect_wss() -> anyhow::Result<WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>> {
    connect_ws_inner(WSS_ADDR, true).await
}

async fn connect_ws_inner(
    addr: &str,
    tls: bool,
) -> anyhow::Result<WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>> {
    let mut req = format!("{}://{addr}/", if tls { "wss" } else { "ws" }).into_client_request()?;
    req.headers_mut().insert("Sec-WebSocket-Protocol", HeaderValue::from_static("mqtt"));
    let connector =
        if tls { Some(Connector::Rustls(Arc::new(rustls_config(true)?))) } else { Some(Connector::Plain) };
    let (ws, _resp) = tokio_tungstenite::connect_async_tls_with_config(req, None, true, connector).await?;
    Ok(ws)
}

// ---------------------------------------------------------------------------
// WebSocket byte-stream adapter (mirrors rmqtt-net::WsStream)
// ---------------------------------------------------------------------------

/// Wraps a `WebSocketStream` and exposes it as a byte stream: incoming Binary
/// messages feed `AsyncRead`, outgoing writes become Binary messages.
struct WsStream<S> {
    inner: WebSocketStream<S>,
    cached_data: Option<Bytes>,
    idx: usize,
}

impl<S> WsStream<S> {
    fn new(inner: WebSocketStream<S>) -> Self {
        Self { inner, cached_data: None, idx: 0 }
    }
}

impl<S> AsyncRead for WsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Some(cached_data) = &self.cached_data {
            let cached_buf = &cached_data[self.idx..];
            let remaining = buf.remaining();
            if cached_buf.len() <= remaining {
                buf.put_slice(cached_buf);
                self.idx = 0;
                self.cached_data = None;
            } else {
                let cached_buf = &cached_buf[0..remaining];
                buf.put_slice(cached_buf);
                self.idx += cached_buf.len();
            }
            return Poll::Ready(Ok(()));
        }

        match ready!(Pin::new(&mut self.inner).poll_next(cx)) {
            Some(Ok(msg)) => {
                let data = msg.into_data();
                let remaining = buf.remaining();
                if data.len() <= remaining {
                    buf.put_slice(data.as_ref());
                } else {
                    let cached_buf = &data[0..remaining];
                    buf.put_slice(cached_buf);
                    self.idx = cached_buf.len();
                    self.cached_data = Some(data);
                }
                Poll::Ready(Ok(()))
            }
            Some(Err(e)) => Poll::Ready(Err(io::Error::other(e.to_string()))),
            None => Poll::Ready(Err(io::Error::from(io::ErrorKind::UnexpectedEof))),
        }
    }
}

impl<S> AsyncWrite for WsStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        if let Err(e) = Pin::new(&mut self.inner).start_send(Message::Binary(buf.to_vec().into())) {
            return Poll::Ready(Err(io::Error::other(e.to_string())));
        }
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        match ready!(Pin::new(&mut self.inner).poll_flush(cx)) {
            Ok(()) => Poll::Ready(Ok(())),
            Err(e) => Poll::Ready(Err(io::Error::other(e.to_string()))),
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        match ready!(Pin::new(&mut self.inner).poll_close(cx)) {
            Ok(()) => Poll::Ready(Ok(())),
            Err(e) => Poll::Ready(Err(io::Error::other(e.to_string()))),
        }
    }
}

// ---------------------------------------------------------------------------
// MQTT client over an arbitrary byte stream
// ---------------------------------------------------------------------------

pub(crate) struct MqttOverTransport<S> {
    stream: S,
    codec: V3Codec,
    read_buf: BytesMut,
}

impl<S: AsyncRead + AsyncWrite + Unpin> MqttOverTransport<S> {
    pub(crate) async fn connect(stream: S, client_id: &str) -> anyhow::Result<Self> {
        Self::connect_with_username(stream, client_id, None).await
    }

    /// CONNECT with an explicit username (None = no username flag).
    pub(crate) async fn connect_with_username(
        mut stream: S,
        client_id: &str,
        username: Option<&str>,
    ) -> anyhow::Result<Self> {
        let mut codec = V3Codec::new(0);
        let conn = Connect {
            protocol: Protocol(MQTT_LEVEL_311),
            clean_session: true,
            keep_alive: 30,
            last_will: None,
            client_id: client_id.into(),
            username: username.map(ByteString::from),
            password: None,
            cert: None,
        };
        let mut out = BytesMut::new();
        codec.encode(PacketV3::Connect(Box::new(conn)), &mut out)?;
        stream.write_all(&out).await?;
        stream.flush().await?;

        let mut this = Self { stream, codec, read_buf: BytesMut::new() };
        match this.recv_packet().await? {
            PacketV3::ConnectAck(ack) if ack.return_code == ConnectAckReason::ConnectionAccepted => {}
            PacketV3::ConnectAck(ack) => {
                return Err(anyhow::anyhow!("CONNECT rejected: {:?}", ack.return_code));
            }
            other => return Err(anyhow::anyhow!("expected CONNACK, got {:?}", other)),
        }
        Ok(this)
    }

    async fn recv_packet(&mut self) -> anyhow::Result<PacketV3> {
        loop {
            if let Some((pkt, _consumed)) = self.codec.decode(&mut self.read_buf)? {
                return Ok(pkt);
            }
            let n = self.stream.read_buf(&mut self.read_buf).await?;
            if n == 0 {
                return Err(anyhow::anyhow!("connection closed while reading a packet"));
            }
        }
    }

    async fn send_packet(&mut self, pkt: &PacketV3) -> anyhow::Result<()> {
        let mut out = BytesMut::new();
        self.codec.encode(pkt.clone(), &mut out)?;
        self.stream.write_all(&out).await?;
        self.stream.flush().await?;
        Ok(())
    }

    async fn subscribe(&mut self, topic: &str, qos: QoS) -> anyhow::Result<()> {
        let sub = PacketV3::Subscribe {
            packet_id: NonZeroU16::new(1).unwrap(),
            topic_filters: vec![(topic.into(), qos)],
        };
        self.send_packet(&sub).await?;
        match self.recv_packet().await? {
            PacketV3::SubscribeAck { .. } => Ok(()),
            other => Err(anyhow::anyhow!("expected SUBACK, got {:?}", other)),
        }
    }

    async fn publish(&mut self, topic: &str, payload: &[u8], qos: QoS, retain: bool) -> anyhow::Result<()> {
        let msg = Publish {
            dup: false,
            retain,
            qos,
            topic: topic.into(),
            packet_id: None,
            payload: Bytes::copy_from_slice(payload),
            properties: None,
        };
        self.send_packet(&PacketV3::Publish(Box::new(msg))).await
    }

    async fn recv_publish(&mut self, timeout: Duration) -> anyhow::Result<Option<(Bytes, bool)>> {
        let pkt = tokio::time::timeout(timeout, self.recv_packet()).await;
        match pkt {
            Ok(Ok(PacketV3::Publish(p))) => Ok(Some((p.payload, p.retain))),
            Ok(Ok(other)) => Err(anyhow::anyhow!("expected PUBLISH, got {:?}", other)),
            Ok(Err(e)) => Err(e),
            Err(_) => Ok(None),
        }
    }

    pub(crate) async fn disconnect(&mut self) -> anyhow::Result<()> {
        self.send_packet(&PacketV3::Disconnect).await
    }
}

// ---------------------------------------------------------------------------
// Self-managed broker
// ---------------------------------------------------------------------------

fn spawn_transport_broker() -> Result<(ClusterNode, PathBuf), anyhow::Error> {
    let binary = rmqttd_binary();
    if !binary.exists() {
        return Err(anyhow::anyhow!(
            "rmqttd binary not found at {:?}; build it first (cargo build -p rmqttd)",
            binary
        ));
    }
    let config = crate::tests::config_path("transport-tls");
    let log_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target");
    let log_file = log_dir.join("transport-tls-node.log");
    let mut node = ClusterNode::new(config, TLS_ADDR, log_file);
    node.spawn(&binary)?;
    if !node.wait_healthy(TRANSPORT_NODE_START_TIMEOUT) {
        return Err(anyhow::anyhow!("transport-tls broker did not become healthy"));
    }
    Ok((node, binary))
}

// ---------------------------------------------------------------------------
// Test cases
// ---------------------------------------------------------------------------

/// MQTT 3.1.1 over TLS: connect (with client cert, since the server requests
/// mTLS), then a full subscribe/publish round trip. [G27]
pub struct TransportTlsV311Test;

impl TestCase for TransportTlsV311Test {
    fn name(&self) -> &str {
        "transport_tls_v311"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let (_node, _binary) = spawn_transport_broker()?;
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let topic = format!("transport/tls/{uid}");
            let payload = format!("TLS-{uid}");

            let tls = connect_tls(rustls_config(true)?).await?;
            let mut mqtt = MqttOverTransport::connect(tls, &format!("tls-{uid}")).await?;
            mqtt.subscribe(&topic, QoS::AtMostOnce).await?;
            mqtt.publish(&topic, payload.as_bytes(), QoS::AtMostOnce, false).await?;
            let msg = mqtt
                .recv_publish(Duration::from_secs(3))
                .await?
                .ok_or_else(|| anyhow::anyhow!("no message over TLS"))?;
            if msg.0.as_ref() != payload.as_bytes() {
                return Err(anyhow::anyhow!("payload mismatch over TLS"));
            }
            let _ = mqtt.disconnect().await;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// MQTT 3.1.1 over TLS-WebSocket (WSS): connect + round trip. [G27]
pub struct TransportWssV311Test;

impl TestCase for TransportWssV311Test {
    fn name(&self) -> &str {
        "transport_wss_v311"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let (_node, _binary) = spawn_transport_broker()?;
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let topic = format!("transport/wss/{uid}");
            let payload = format!("WSS-{uid}");

            let ws = connect_wss().await?;
            let mut mqtt = MqttOverTransport::connect(WsStream::new(ws), &format!("wss-{uid}")).await?;
            mqtt.subscribe(&topic, QoS::AtMostOnce).await?;
            mqtt.publish(&topic, payload.as_bytes(), QoS::AtMostOnce, false).await?;
            let msg = mqtt
                .recv_publish(Duration::from_secs(3))
                .await?
                .ok_or_else(|| anyhow::anyhow!("no message over WSS"))?;
            if msg.0.as_ref() != payload.as_bytes() {
                return Err(anyhow::anyhow!("payload mismatch over WSS"));
            }
            let _ = mqtt.disconnect().await;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// MQTT 3.1.1 over plain WebSocket: connect + round trip. [G27]
pub struct TransportWsV311Test;

impl TestCase for TransportWsV311Test {
    fn name(&self) -> &str {
        "transport_ws_v311"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let (_node, _binary) = spawn_transport_broker()?;
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let topic = format!("transport/ws/{uid}");
            let payload = format!("WS-{uid}");

            let ws = connect_ws().await?;
            let mut mqtt = MqttOverTransport::connect(WsStream::new(ws), &format!("ws-{uid}")).await?;
            mqtt.subscribe(&topic, QoS::AtMostOnce).await?;
            mqtt.publish(&topic, payload.as_bytes(), QoS::AtMostOnce, false).await?;
            let msg = mqtt
                .recv_publish(Duration::from_secs(3))
                .await?
                .ok_or_else(|| anyhow::anyhow!("no message over WS"))?;
            if msg.0.as_ref() != payload.as_bytes() {
                return Err(anyhow::anyhow!("payload mismatch over WS"));
            }
            let _ = mqtt.disconnect().await;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// Mutual TLS on the TLS listener: a client WITHOUT a client certificate must
/// not be able to use the connection (the server requires one), and a client
/// WITH the client.pem certificate must succeed. [G27]
///
/// Note: rustls surfaces the server's `CertificateRequired` alert on the
/// first read rather than at connect() (the handshake itself returns Ok), so
/// "no certificate" is asserted at the MQTT layer: sending the CONNECT must
/// fail (alert / close), not yield a CONNACK.
pub struct TransportTlsMtlsV311Test;

impl TestCase for TransportTlsMtlsV311Test {
    fn name(&self) -> &str {
        "transport_tls_mtls_v311"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let (_node, _binary) = spawn_transport_broker()?;
            let uid = uuid::Uuid::new_v4().simple().to_string();
            let topic = format!("transport/mtls/{uid}");
            let payload = format!("MTLS-{uid}");

            // Without a client certificate the connection must be unusable:
            // either the handshake fails outright, or (rustls) the server
            // kills it with a CertificateRequired alert once data is sent.
            if let Ok(no_cert_tls) = connect_tls(rustls_config(false)?).await {
                if MqttOverTransport::connect(no_cert_tls, "no-cert-probe").await.is_ok() {
                    return Err(anyhow::anyhow!(
                        "client without a certificate completed an MQTT session, but the server \
                         requires mTLS (cross_certificate=true)"
                    ));
                }
            }

            // With the client certificate everything works.
            let tls = connect_tls(rustls_config(true)?).await?;
            let mut mqtt = MqttOverTransport::connect(tls, &format!("mtls-{uid}")).await?;
            mqtt.subscribe(&topic, QoS::AtMostOnce).await?;
            mqtt.publish(&topic, payload.as_bytes(), QoS::AtMostOnce, false).await?;
            let msg = mqtt
                .recv_publish(Duration::from_secs(3))
                .await?
                .ok_or_else(|| anyhow::anyhow!("no message over mTLS"))?;
            if msg.0.as_ref() != payload.as_bytes() {
                return Err(anyhow::anyhow!("payload mismatch over mTLS"));
            }
            let _ = mqtt.disconnect().await;
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}
