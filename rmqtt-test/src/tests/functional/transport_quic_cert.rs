//! Client-certificate identity over QUIC (PR #555)
//!
//! The QUIC listener requests (and, with `cross_certificate = true`, requires)
//! a client certificate, but before PR #555 the dispatcher was built with
//! `cert_info = None` on that transport only:
//!
//! ```text
//! // rmqtt-net/src/builder.rs, `Acceptor::quic()` before the fix
//! Ok(Dispatcher::new(self.socket, self.remote_addr, None, self.cfg))
//! ```
//!
//! `MqttStream::recv_connect` only overwrites `connect.username` when
//! `self.cert_info` is `Some(..)`, so under QUIC every `cert_*_as_username`
//! setting (and `collect_cert_info`) was silently inert: the username the
//! client wrote in the CONNECT survived untouched. Any client holding a valid
//! certificate could therefore claim any identity — an authorization bypass
//! whenever auth/ACL keys on the username, not on the certificate.
//!
//! `transport_quic_cert_spiffe_v311` pins that difference. It connects over
//! QUIC, presents a client certificate whose URI SAN is
//! `spiffe://example.org/user/barry`, and sends `username = "attacker"` in the
//! CONNECT. The listener has `cert_spiffe_as_username = true`, so the session
//! username must be the SPIFFE ID, read back over the HTTP API:
//!
//! * broker built from a tree **without** PR #555 → username is `attacker`
//!   (the certificate was ignored) → the case FAILS, which is the point;
//! * broker built **with** PR #555 → username is the SPIFFE ID → the case
//!   PASSES.
//!
//! **The case is self-contained**: it mints its own CA, server certificate and
//! client certificate at runtime (via `rcgen`, no openssl on the runner),
//! writes its own `rmqtt.toml` + `rmqtt-http-api.toml` into a private
//! temporary directory, and spawns its own broker process — so it shares
//! nothing with `configs/transport-tls`, the repo's test certs in
//! `rmqtt-bin/`, or any other fixture.
//!
//! | case | TCP (health probe) | QUIC | gRPC | HTTP |
//! |---|---|---|---|---|
//! | `transport_quic_cert_spiffe_v311` | 1926 | 1927 | 5393 | 6080 |
//!
//! The TCP listener exists purely because the harness' readiness probe
//! (`ClusterNode::wait_healthy`) waits for a TCP port: a QUIC-only broker has
//! no TCP socket to probe. It carries no traffic; the case only ever speaks
//! MQTT over QUIC. All listeners are bound during `MqttServerBuilder::bind`,
//! before any accept loop runs, so an open TCP port also means the QUIC
//! endpoint is already bound.
//!
//! Run it alone with:
//!
//!     ./target/debug/mqtt_harness --workspace . --suites functional_transport \
//!         -t transport_quic_cert

use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use quinn::{Connection, Endpoint, RecvStream, SendStream};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::{ClientConfig, RootCertStore};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::tests::functional::cluster_session_restart::{rmqttd_binary, ClusterNode};
use crate::tests::functional::transport_tls::MqttOverTransport;

use super::flapping_v5::http_request;

const SPIFFE_ID: &str = "spiffe://example.org/user/barry";
/// What the client puts in the CONNECT packet. A broker that extracts the
/// certificate replaces it; a broker that does not leaves it in the session.
const SPOOFED_USERNAME: &str = "attacker";
const SERVER_NAME: &str = "localhost";
const NODE_START_TIMEOUT: Duration = Duration::from_secs(20);

struct CaseSpec {
    tag: &'static str,
    /// TCP listener, health probe only (see the module docs).
    tcp_port: u16,
    quic_port: u16,
    grpc_port: u16,
    http_port: u16,
}

const CASE_V311: CaseSpec =
    CaseSpec { tag: "v311", tcp_port: 1926, quic_port: 1927, grpc_port: 5393, http_port: 6080 };

// ---------------------------------------------------------------------------
// Per-case certificate minting (rcgen, no openssl on the runner)
// ---------------------------------------------------------------------------

struct CaseCerts {
    ca_pem: String,
    server_pem: String,
    server_key_pem: String,
    client_pem: String,
    client_key_pem: String,
}

/// Mints a fresh CA, a server certificate (`localhost` / `127.0.0.1` SANs,
/// serverAuth) and one client certificate (clientAuth, CN `QuicCertClient`,
/// SPIFFE URI SAN), all signed by that CA.
fn mint_case_certs() -> anyhow::Result<CaseCerts> {
    use rcgen::{
        BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
        SanType,
    };

    let ca_key = KeyPair::generate()?;
    let mut ca_params = CertificateParams::new(Vec::<String>::new())?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca_params.distinguished_name.push(DnType::CommonName, "rmqtt-test-quic-cert-root");
    let ca_cert = ca_params.self_signed(&ca_key)?;

    let server_key = KeyPair::generate()?;
    let mut server_params = CertificateParams::new(Vec::<String>::new())?;
    server_params.distinguished_name.push(DnType::CommonName, "localhost");
    server_params.subject_alt_names = vec![
        SanType::DnsName("localhost".try_into()?),
        SanType::IpAddress(std::net::IpAddr::from([127, 0, 0, 1])),
    ];
    server_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server_cert = server_params.signed_by(&server_key, &ca_cert, &ca_key)?;

    let client_key = KeyPair::generate()?;
    let mut client_params = CertificateParams::new(Vec::<String>::new())?;
    client_params.distinguished_name.push(DnType::CommonName, "QuicCertClient");
    client_params.subject_alt_names = vec![SanType::URI(SPIFFE_ID.try_into()?)];
    client_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let client_cert = client_params.signed_by(&client_key, &ca_cert, &ca_key)?;

    Ok(CaseCerts {
        ca_pem: ca_cert.pem(),
        server_pem: server_cert.pem(),
        server_key_pem: server_key.serialize_pem(),
        client_pem: client_cert.pem(),
        client_key_pem: client_key.serialize_pem(),
    })
}

// ---------------------------------------------------------------------------
// Per-case broker: private config file, private ports, own process
// ---------------------------------------------------------------------------

struct QuicBroker {
    node: ClusterNode,
    certs: CaseCerts,
    _dir: PathBuf,
}

impl Drop for QuicBroker {
    fn drop(&mut self) {
        self.node.kill();
        // Best-effort cleanup of the private directory; files may still be
        // locked for a moment on Windows, which is fine (temp dir).
        let _ = std::fs::remove_dir_all(&self._dir);
    }
}

/// Formats a path with forward slashes so it can be embedded in TOML
/// without escaping.
fn toml_path(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn spawn_quic_broker(spec: &'static CaseSpec) -> anyhow::Result<QuicBroker> {
    let binary = rmqttd_binary();
    if !binary.exists() {
        return Err(anyhow::anyhow!(
            "rmqttd binary not found at {:?}; build it first (cargo build -p rmqttd)",
            binary
        ));
    }

    let uid = uuid::Uuid::new_v4().simple().to_string();
    let dir = std::env::temp_dir().join(format!("rmqtt-quic-cert-{}-{uid}", spec.tag));
    std::fs::create_dir_all(dir.join("plugins"))?;

    let certs = mint_case_certs()?;

    // Private plugin config: http-api on the case's own HTTP port, no
    // bearer token (loopback only; the case under test is the certificate
    // to username binding, not authentication).
    std::fs::write(
        dir.join("plugins").join("rmqtt-http-api.toml"),
        format!(
            "# Runtime-generated for the {} QUIC client-certificate case (PR #555).\n\
             max_row_limit = 10_000\n\
             http_laddr = \"127.0.0.1:{}\"\n\
             http_request_log = false\n",
            spec.tag, spec.http_port
        ),
    )?;

    // Private broker config: QUIC listener only for real traffic, mTLS against
    // the case's own CA, SPIFFE binding on, every shared listener disabled.
    // The TCP listener is the harness' readiness probe and nothing else.
    let config_path = dir.join("rmqtt.toml");
    let tcp_addr = format!("127.0.0.1:{}", spec.tcp_port);
    let quic_addr = format!("127.0.0.1:{}", spec.quic_port);
    std::fs::write(
        &config_path,
        format!(
            "# Runtime-generated for the {tag} QUIC client-certificate case (PR #555).\n\
             node.id = 1\n\
             node.busy.check_enable = false\n\
             log.to = \"console\"\n\
             log.level = \"info\"\n\
             plugins.dir = \"{plugins_dir}\"\n\
             plugins.default_startups = [\"rmqtt-http-api\"]\n\
             plugins.disabled_default_startups = [\"rmqtt-acl\", \"rmqtt-counter\"]\n\
             rpc.server_addr = \"127.0.0.1:{grpc}\"\n\
             listener.tcp.internal.enable = false\n\
             listener.tcp.external.enable = true\n\
             listener.tcp.external.addr = \"{tcp_addr}\"\n\
             listener.tcp.external.allow_anonymous = true\n\
             listener.ws.external.enable = false\n\
             listener.wss.external.enable = false\n\
             listener.tls.external.enable = false\n\
             listener.quic.external.enable = true\n\
             listener.quic.external.addr = \"{quic_addr}\"\n\
             listener.quic.external.allow_anonymous = true\n\
             listener.quic.external.cross_certificate = true\n\
             listener.quic.external.cert = \"{server_pem}\"\n\
             listener.quic.external.key = \"{server_key}\"\n\
             listener.quic.external.client_ca_certs = \"{ca_pem}\"\n\
             listener.quic.external.cert_spiffe_as_username = true\n",
            tag = spec.tag,
            plugins_dir = toml_path(&dir.join("plugins")),
            grpc = spec.grpc_port,
            tcp_addr = tcp_addr,
            quic_addr = quic_addr,
            server_pem = toml_path(&dir.join("server.pem")),
            server_key = toml_path(&dir.join("server.key")),
            ca_pem = toml_path(&dir.join("ca.pem")),
        ),
    )?;

    // The PEM files the config points at.
    std::fs::write(dir.join("ca.pem"), &certs.ca_pem)?;
    std::fs::write(dir.join("server.pem"), &certs.server_pem)?;
    std::fs::write(dir.join("server.key"), &certs.server_key_pem)?;

    let log_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target");
    let log_file = log_dir.join(format!("quic-cert-{}-node.log", spec.tag));
    let mut node = ClusterNode::new(config_path, &tcp_addr, log_file);
    node.spawn(&binary)?;
    if !node.wait_healthy(NODE_START_TIMEOUT) {
        let tail = node.log_tail(30).unwrap_or_else(|| "<log file unreadable>".into());
        return Err(anyhow::anyhow!(
            "quic-cert[{tag}] broker did not become healthy; log tail:\n{tail}",
            tag = spec.tag
        ));
    }

    Ok(QuicBroker { node, certs, _dir: dir })
}

// ---------------------------------------------------------------------------
// QUIC client (trusts the case's own CA, presents the case's own client cert)
// ---------------------------------------------------------------------------

/// One bidirectional QUIC stream exposed as `AsyncRead + AsyncWrite`, so the
/// harness' MQTT client can drive it exactly like the TLS/WS transports. This
/// is deliberately a local adapter (not `rmqtt_net::QuinnBiStream`): the case
/// must compile unchanged against a tree with and without PR #555.
struct QuicBiStream {
    send: SendStream,
    recv: RecvStream,
}

impl AsyncRead for QuicBiStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        // Fully qualified: quinn has inherent `poll_*` helpers whose error type
        // is `WriteError`/`ReadError`, which would shadow the trait methods.
        AsyncRead::poll_read(Pin::new(&mut self.recv), cx, buf)
    }
}

impl AsyncWrite for QuicBiStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(&mut self.send), cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.send), cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(&mut self.send), cx)
    }
}

impl Unpin for QuicBiStream {}

fn quic_client_config(certs: &CaseCerts) -> anyhow::Result<quinn::ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from_pem_slice(certs.ca_pem.as_bytes())?)?;
    let chain = vec![CertificateDer::from_pem_slice(certs.client_pem.as_bytes())?];
    let key = PrivateKeyDer::from_pem_slice(certs.client_key_pem.as_bytes())?;
    let mut tls = ClientConfig::builder().with_root_certificates(roots).with_client_auth_cert(chain, key)?;
    // The broker pins `alpn = ["mqtt", "mqttv5"]` on its QUIC endpoint
    // (`bind_quic`); offering nothing the server accepts aborts the handshake.
    tls.alpn_protocols = vec![b"mqtt".to_vec()];

    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls)?;
    let mut cfg = quinn::ClientConfig::new(Arc::new(crypto));
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(quinn::IdleTimeout::try_from(Duration::from_secs(30))?));
    cfg.transport_config(Arc::new(transport));
    Ok(cfg)
}

/// Opens the endpoint, completes the QUIC handshake with the client
/// certificate and returns the first bidirectional stream. `Endpoint` and
/// `Connection` are handed back so the caller keeps them alive for the whole
/// session (dropping every handle closes the connection).
async fn connect_quic(
    certs: &CaseCerts,
    spec: &CaseSpec,
) -> anyhow::Result<(Endpoint, Connection, QuicBiStream)> {
    let mut endpoint = Endpoint::client(SocketAddr::from(([127, 0, 0, 1], 0)))?;
    endpoint.set_default_client_config(quic_client_config(certs)?);
    let addr = SocketAddr::from(([127, 0, 0, 1], spec.quic_port));
    let conn = endpoint.connect(addr, SERVER_NAME)?.await?;
    let (send, recv) = conn.open_bi().await?;
    Ok((endpoint, conn, QuicBiStream { send, recv }))
}

/// Reads the session's username back from the case's HTTP API
/// (`GET /api/v1/clients/{clientid}` → `SearchResult.username`). The broker
/// creates the session on CONNACK and the API reads the same registry, so
/// the retries only guard against scheduling jitter between the two.
async fn http_username(http_addr: &str, client_id: &str) -> anyhow::Result<String> {
    let path = format!("/api/v1/clients/{client_id}");
    let mut body = String::new();
    for _ in 0..10 {
        let (status, b) = http_request(http_addr, "GET", &path).await?;
        if status == 200 {
            body = b;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    if body.is_empty() {
        return Err(anyhow::anyhow!("GET {path} never answered 200"));
    }
    let needle = "\"username\":";
    let start =
        body.find(needle).ok_or_else(|| anyhow::anyhow!("no username field in {body:.200}"))? + needle.len();
    let rest = &body[start..];
    let end = rest.find(',').unwrap_or(rest.len());
    Ok(rest[..end].trim().trim_matches('"').to_string())
}

// ---------------------------------------------------------------------------
// Test case
// ---------------------------------------------------------------------------

/// MQTT 3.1.1 over mTLS-on-QUIC: a CONNECT claiming `username = "attacker"`
/// must be replaced by the SPIFFE ID of the client certificate. Before
/// PR #555 the QUIC dispatcher never received the certificate, so the
/// self-reported username survived — this case is red there and green after.
pub struct TransportQuicCertSpiffeV311Test;

impl TestCase for TransportQuicCertSpiffeV311Test {
    fn name(&self) -> &str {
        "transport_quic_cert_spiffe_v311"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let broker = spawn_quic_broker(&CASE_V311)?;
            let client_id = format!("quic-cert-{}", uuid::Uuid::new_v4().simple());

            let (_endpoint, _conn, stream) = connect_quic(&broker.certs, &CASE_V311).await?;
            // The certificate is the only identity source that must survive:
            // the CONNECT carries a username of its own, precisely to show
            // whether the transport-level identity overrides it.
            let mut mqtt =
                MqttOverTransport::connect_with_username(stream, &client_id, Some(SPOOFED_USERNAME)).await?;
            let username = http_username(&format!("127.0.0.1:{}", CASE_V311.http_port), &client_id).await?;
            let _ = mqtt.disconnect().await;

            if username != SPIFFE_ID {
                return Err(anyhow::anyhow!(
                    "QUIC listener ignored the client certificate: the session username is {username:?} \
                     (the self-reported value) instead of the certificate's SPIFFE ID {SPIFFE_ID:?}"
                ));
            }
            drop(broker);
            Ok::<(), anyhow::Error>(())
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(40)
    }
}
