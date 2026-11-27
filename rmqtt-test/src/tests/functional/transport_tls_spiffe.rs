//! SPIFFE username binding over mTLS (issue #535)
//!
//! Verifies that a TLS listener with `cert_spiffe_as_username = true` binds
//! the SPIFFE ID (URI SAN) of the client certificate as the session
//! username, in both protocol versions, and that a certificate without a
//! URI SAN leaves the username the client sent untouched:
//!
//! - `transport_tls_spiffe_v311` — a CONNECT without a username whose
//!   client certificate carries the URI SAN
//!   `spiffe://example.org/user/barry`; the session's username, read back
//!   over the HTTP API, must be the SPIFFE ID.
//! - `transport_tls_spiffe_v5` — the same over MQTT 5.0.
//! - `transport_tls_spiffe_fallback_v311` — a CONNECT whose client
//!   certificate has no URI SAN, carrying an explicit username; that
//!   username must survive (no certificate source hits, and neither
//!   cert_subject_dn_as_username nor cert_cn_as_username is enabled).
//!
//! **Every case is fully self-contained**: it mints its own CA, server
//! certificate and client certificate at runtime (via `rcgen`, no openssl
//! on the runner), writes its own `rmqtt.toml` + `rmqtt-http-api.toml`
//! into a private temporary directory, and spawns its own broker process
//! on its own ports — so the three cases share nothing with each other,
//! with the repo's test certs in `rmqtt-bin/`, or with any other fixture:
//!
//! | case | TLS | gRPC | HTTP |
//! |---|---|---|---|
//! | `transport_tls_spiffe_v311` | 1904 | 5381 | 6068 |
//! | `transport_tls_spiffe_v5` | 1905 | 5382 | 6069 |
//! | `transport_tls_spiffe_fallback_v311` | 1906 | 5383 | 6071 |
//!
//! (6070 belongs to `session-sled-stress`; the neighbouring ports are the
//! next free ones above the flapping/webhook fixtures — see the port
//! inventory in `configs/flapping/rmqtt.toml`.)

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::BytesMut;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;
use tokio_util::codec::{Decoder, Encoder};

use rmqtt_codec::v5::Packet as PacketV5;
use rmqtt_codec::v5::{Codec as V5Codec, Connect as ConnectV5, ConnectAckReason as ConnectAckReasonV5};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::tests::functional::cluster_session_restart::{rmqttd_binary, ClusterNode};
use crate::tests::functional::transport_tls::MqttOverTransport;

use super::flapping_v5::http_request;

const SPIFFE_ID: &str = "spiffe://example.org/user/barry";
const SERVER_NAME: &str = "localhost";
const NODE_START_TIMEOUT: Duration = Duration::from_secs(20);

/// Per-case identity: private ports and whether the minted client
/// certificate carries the SPIFFE URI SAN.
struct CaseSpec {
    tag: &'static str,
    tls_port: u16,
    grpc_port: u16,
    http_port: u16,
    client_with_uri_san: bool,
}

const CASE_V311: CaseSpec =
    CaseSpec { tag: "v311", tls_port: 1904, grpc_port: 5381, http_port: 6068, client_with_uri_san: true };
const CASE_V5: CaseSpec =
    CaseSpec { tag: "v5", tls_port: 1905, grpc_port: 5382, http_port: 6069, client_with_uri_san: true };
const CASE_FALLBACK: CaseSpec = CaseSpec {
    tag: "fallback",
    tls_port: 1906,
    grpc_port: 5383,
    http_port: 6071,
    client_with_uri_san: false,
};

// ---------------------------------------------------------------------------
// Per-case certificate minting (rcgen, no openssl on the runner)
// ---------------------------------------------------------------------------

/// The PEM material a case needs: the CA (trust root for both directions),
/// the listener's server certificate + key, and the client certificate +
/// key to present.
struct CaseCerts {
    ca_pem: String,
    server_pem: String,
    server_key_pem: String,
    client_pem: String,
    client_key_pem: String,
}

/// Mints a fresh CA, a server certificate (`localhost` / `127.0.0.1` SANs,
/// serverAuth) and one client certificate (clientAuth, with the SPIFFE URI
/// SAN iff `with_uri_san`), all signed by that CA.
fn mint_case_certs(with_uri_san: bool) -> anyhow::Result<CaseCerts> {
    use rcgen::{
        BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
        SanType,
    };

    let ca_key = KeyPair::generate()?;
    let mut ca_params = CertificateParams::new(Vec::<String>::new())?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca_params.distinguished_name.push(DnType::CommonName, "rmqtt-test-spiffe-root");
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
    client_params
        .distinguished_name
        .push(DnType::CommonName, if with_uri_san { "SpiffeClientBarry" } else { "SpiffeClientNoSan" });
    if with_uri_san {
        client_params.subject_alt_names = vec![SanType::URI(SPIFFE_ID.try_into()?)];
    }
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

struct SpiffeBroker {
    node: ClusterNode,
    spec: &'static CaseSpec,
    certs: CaseCerts,
    _dir: PathBuf,
}

impl Drop for SpiffeBroker {
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

fn spawn_spiffe_broker(spec: &'static CaseSpec) -> anyhow::Result<SpiffeBroker> {
    let binary = rmqttd_binary();
    if !binary.exists() {
        return Err(anyhow::anyhow!(
            "rmqttd binary not found at {:?}; build it first (cargo build -p rmqttd)",
            binary
        ));
    }

    let uid = uuid::Uuid::new_v4().simple().to_string();
    let dir = std::env::temp_dir().join(format!("rmqtt-tls-spiffe-{}-{uid}", spec.tag));
    std::fs::create_dir_all(dir.join("plugins"))?;

    let certs = mint_case_certs(spec.client_with_uri_san)?;

    // Private plugin config: http-api on the case's own HTTP port, no
    // bearer token (loopback only; the case under test is the certificate
    // to username binding, not authentication).
    std::fs::write(
        dir.join("plugins").join("rmqtt-http-api.toml"),
        format!(
            "# Runtime-generated for the {} SPIFFE-username case (issue #535).\n\
             max_row_limit = 10_000\n\
             http_laddr = \"127.0.0.1:{}\"\n\
             http_request_log = false\n",
            spec.tag, spec.http_port
        ),
    )?;

    // Private broker config: TLS listener only, mTLS against the case's own
    // CA, SPIFFE binding on, every shared listener disabled.
    let config_path = dir.join("rmqtt.toml");
    let tls_addr = format!("127.0.0.1:{}", spec.tls_port);
    std::fs::write(
        &config_path,
        format!(
            "# Runtime-generated for the {tag} SPIFFE-username case (issue #535).\n\
             node.id = 1\n\
             node.busy.check_enable = false\n\
             log.to = \"console\"\n\
             log.level = \"info\"\n\
             plugins.dir = \"{plugins_dir}\"\n\
             plugins.default_startups = [\"rmqtt-http-api\"]\n\
             plugins.disabled_default_startups = [\"rmqtt-acl\", \"rmqtt-counter\"]\n\
             rpc.server_addr = \"127.0.0.1:{grpc}\"\n\
             listener.tcp.external.enable = false\n\
             listener.tcp.internal.enable = false\n\
             listener.ws.external.enable = false\n\
             listener.wss.external.enable = false\n\
             listener.quic.external.enable = false\n\
             listener.tls.external.enable = true\n\
             listener.tls.external.addr = \"{tls_addr}\"\n\
             listener.tls.external.allow_anonymous = true\n\
             listener.tls.external.cross_certificate = true\n\
             listener.tls.external.cert = \"{server_pem}\"\n\
             listener.tls.external.key = \"{server_key}\"\n\
             listener.tls.external.client_ca_certs = \"{ca_pem}\"\n\
             listener.tls.external.cert_spiffe_as_username = true\n",
            tag = spec.tag,
            plugins_dir = toml_path(&dir.join("plugins")),
            grpc = spec.grpc_port,
            tls_addr = tls_addr,
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
    let log_file = log_dir.join(format!("tls-spiffe-{}-node.log", spec.tag));
    let mut node = ClusterNode::new(config_path, &tls_addr, log_file);
    node.spawn(&binary)?;
    if !node.wait_healthy(NODE_START_TIMEOUT) {
        let tail = node.log_tail(30).unwrap_or_else(|| "<log file unreadable>".into());
        return Err(anyhow::anyhow!(
            "tls-spiffe[{tag}] broker did not become healthy; log tail:\n{tail}",
            tag = spec.tag
        ));
    }

    Ok(SpiffeBroker { node, spec, certs, _dir: dir })
}

// ---------------------------------------------------------------------------
// TLS client (trusts the case's own CA, presents the case's own client cert)
// ---------------------------------------------------------------------------

fn case_rustls_config(certs: &CaseCerts) -> anyhow::Result<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from_pem_slice(certs.ca_pem.as_bytes())?)?;
    let chain = vec![CertificateDer::from_pem_slice(certs.client_pem.as_bytes())?];
    let key = PrivateKeyDer::from_pem_slice(certs.client_key_pem.as_bytes())?;
    Ok(ClientConfig::builder().with_root_certificates(roots).with_client_auth_cert(chain, key)?)
}

async fn connect_tls_to(addr: &str, cfg: ClientConfig) -> anyhow::Result<TlsStream<tokio::net::TcpStream>> {
    let connector = TlsConnector::from(Arc::new(cfg));
    let stream = tokio::net::TcpStream::connect(addr).await?;
    let server_name = ServerName::try_from(SERVER_NAME).map_err(|_| anyhow::anyhow!("bad server name"))?;
    Ok(connector.connect(server_name, stream).await?)
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
// Minimal MQTT 5.0 client over the TLS stream
// ---------------------------------------------------------------------------

struct MqttV5OverTls {
    stream: TlsStream<tokio::net::TcpStream>,
    codec: V5Codec,
    read_buf: BytesMut,
}

impl MqttV5OverTls {
    /// CONNECT without a username; returns after a successful CONNACK.
    async fn connect(stream: TlsStream<tokio::net::TcpStream>, client_id: &str) -> anyhow::Result<Self> {
        let mut codec = V5Codec::new(0, 0);
        let conn = ConnectV5 {
            clean_start: true,
            keep_alive: 30,
            session_expiry_interval_secs: 0,
            auth_method: None,
            auth_data: None,
            request_problem_info: false,
            request_response_info: false,
            receive_max: None,
            topic_alias_max: 0,
            user_properties: Vec::new(),
            max_packet_size: None,
            last_will: None,
            client_id: client_id.into(),
            username: None,
            password: None,
            cert: None,
        };
        let mut out = BytesMut::new();
        codec.encode(PacketV5::Connect(Box::new(conn)), &mut out)?;
        let mut this = Self { stream, codec, read_buf: BytesMut::new() };
        this.stream.write_all(&out).await?;
        this.stream.flush().await?;

        match this.recv_packet().await? {
            PacketV5::ConnectAck(ack) if ack.reason_code == ConnectAckReasonV5::Success => {}
            PacketV5::ConnectAck(ack) => {
                return Err(anyhow::anyhow!("CONNECT rejected: {:?}", ack.reason_code));
            }
            other => return Err(anyhow::anyhow!("expected CONNACK, got {:?}", other)),
        }
        Ok(this)
    }

    async fn recv_packet(&mut self) -> anyhow::Result<PacketV5> {
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
}

// ---------------------------------------------------------------------------
// Test cases
// ---------------------------------------------------------------------------

/// MQTT 3.1.1 over mTLS: a CONNECT without a username gets the SPIFFE ID of
/// the client certificate as its session username (issue #535).
pub struct TransportTlsSpiffeV311Test;

impl TestCase for TransportTlsSpiffeV311Test {
    fn name(&self) -> &str {
        "transport_tls_spiffe_v311"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let broker = spawn_spiffe_broker(&CASE_V311)?;
            let client_id = format!("spiffe-{}", uuid::Uuid::new_v4().simple());

            let tls = connect_tls_to(
                &format!("127.0.0.1:{}", CASE_V311.tls_port),
                case_rustls_config(&broker.certs)?,
            )
            .await?;
            // No username in the CONNECT: the certificate is the only
            // identity source available to the broker.
            let mut mqtt = MqttOverTransport::connect_with_username(tls, &client_id, None).await?;
            let username = http_username(&format!("127.0.0.1:{}", CASE_V311.http_port), &client_id).await?;
            if username != SPIFFE_ID {
                return Err(anyhow::anyhow!(
                    "expected the session username to be the SPIFFE ID, got {username:?}"
                ));
            }
            let _ = mqtt.disconnect().await;
            drop(broker);
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

/// MQTT 5.0 over mTLS: the same binding as `transport_tls_spiffe_v311`,
/// through the v5 handshake path.
pub struct TransportTlsSpiffeV5Test;

impl TestCase for TransportTlsSpiffeV5Test {
    fn name(&self) -> &str {
        "transport_tls_spiffe_v5"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let broker = spawn_spiffe_broker(&CASE_V5)?;
            let client_id = format!("spiffe-v5-{}", uuid::Uuid::new_v4().simple());

            let tls = connect_tls_to(
                &format!("127.0.0.1:{}", CASE_V5.tls_port),
                case_rustls_config(&broker.certs)?,
            )
            .await?;
            let _mqtt = MqttV5OverTls::connect(tls, &client_id).await?;
            let username = http_username(&format!("127.0.0.1:{}", CASE_V5.http_port), &client_id).await?;
            if username != SPIFFE_ID {
                return Err(anyhow::anyhow!(
                    "expected the session username to be the SPIFFE ID, got {username:?}"
                ));
            }
            drop(broker);
            Ok::<(), anyhow::Error>(())
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

/// Fallback: a client certificate without a URI SAN binds nothing, so the
/// username the client sent in the CONNECT is kept (neither
/// cert_subject_dn_as_username nor cert_cn_as_username is enabled on this
/// listener, so no certificate source can overwrite it either).
pub struct TransportTlsSpiffeFallbackV311Test;

impl TestCase for TransportTlsSpiffeFallbackV311Test {
    fn name(&self) -> &str {
        "transport_tls_spiffe_fallback_v311"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let broker = spawn_spiffe_broker(&CASE_FALLBACK)?;
            let client_id = format!("spiffe-fb-{}", uuid::Uuid::new_v4().simple());

            // The minted client certificate chains to the listener's CA but
            // carries no SAN at all: extraction yields no SPIFFE ID, the
            // Subject DN/CN are not configured as username sources, so the
            // self-reported username must survive.
            let tls = connect_tls_to(
                &format!("127.0.0.1:{}", CASE_FALLBACK.tls_port),
                case_rustls_config(&broker.certs)?,
            )
            .await?;
            let mut mqtt = MqttOverTransport::connect_with_username(tls, &client_id, Some("carol")).await?;
            let username =
                http_username(&format!("127.0.0.1:{}", CASE_FALLBACK.http_port), &client_id).await?;
            if username != "carol" {
                return Err(anyhow::anyhow!(
                    "expected the self-reported username to be kept, got {username:?}"
                ));
            }
            let _ = mqtt.disconnect().await;
            drop(broker);
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
