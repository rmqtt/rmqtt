//! CRL-based client certificate revocation over mTLS
//!
//! Verifies that a TLS listener with `crl` configured rejects a client
//! certificate listed in the CRL during the handshake, while a certificate
//! from the same CA that is not listed still connects:
//!
//! - `transport_tls_crl` — one broker, one CA, two client certificates
//!   (serial 10 and serial 11) and a CRL revoking serial 11. The
//!   non-revoked client must get CONNACK 0x00; the revoked one must be
//!   refused with a `CertificateRevoked` TLS alert and never reach CONNECT.
//!
//! Revocation is decided in the TLS handshake, before the broker reads any
//! MQTT byte, so the case is protocol-version independent: the MQTT 3.1.1
//! CONNECT is only the probe that an accepted client gets a working session.
//!
//! Like the SPIFFE cases, the case is self-contained: it mints its own PKI
//! and CRL with `rcgen`, writes its own `rmqtt.toml` into a private
//! temporary directory and spawns its own broker on its own ports
//! (TLS 1941 / gRPC 5396, unused anywhere else in `rmqtt-test`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::pki_types::pem::PemObject;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tokio_rustls::TlsConnector;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::tests::functional::cluster_session_restart::{rmqttd_binary, ClusterNode};
use crate::tests::functional::transport_tls::MqttOverTransport;

const TLS_PORT: u16 = 1941;
const GRPC_PORT: u16 = 5396;
const SERVER_NAME: &str = "localhost";
const NODE_START_TIMEOUT: Duration = Duration::from_secs(20);
const GOOD_SERIAL: u64 = 10;
const REVOKED_SERIAL: u64 = 11;

struct ClientCert {
    pem: String,
    key_pem: String,
}

struct CrlCerts {
    ca_pem: String,
    server_pem: String,
    server_key_pem: String,
    crl_pem: String,
    good: ClientCert,
    revoked: ClientCert,
}

/// Mints a CA, a server certificate, two client certificates and a CRL
/// (issued by the CA) that revokes only `REVOKED_SERIAL`.
fn mint_certs() -> anyhow::Result<CrlCerts> {
    use rcgen::{
        date_time_ymd, BasicConstraints, CertificateParams, CertificateRevocationListParams, DnType,
        ExtendedKeyUsagePurpose, IsCa, KeyIdMethod, KeyPair, KeyUsagePurpose, RevocationReason,
        RevokedCertParams, SanType, SerialNumber,
    };

    let ca_key = KeyPair::generate()?;
    let mut ca_params = CertificateParams::new(Vec::<String>::new())?;
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    ca_params.distinguished_name.push(DnType::CommonName, "rmqtt-test-crl-root");
    let ca_cert = ca_params.self_signed(&ca_key)?;

    let server_key = KeyPair::generate()?;
    let mut server_params = CertificateParams::new(Vec::<String>::new())?;
    server_params.distinguished_name.push(DnType::CommonName, "localhost");
    server_params.subject_alt_names = vec![
        SanType::DnsName("localhost".try_into()?),
        SanType::IpAddress(std::net::IpAddr::from([127, 0, 0, 1])),
    ];
    server_params.serial_number = Some(SerialNumber::from(2u64));
    server_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server_cert = server_params.signed_by(&server_key, &ca_cert, &ca_key)?;

    let client = |cn: &str, serial: u64| -> anyhow::Result<ClientCert> {
        let key = KeyPair::generate()?;
        let mut params = CertificateParams::new(Vec::<String>::new())?;
        params.distinguished_name.push(DnType::CommonName, cn);
        params.serial_number = Some(SerialNumber::from(serial));
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let cert = params.signed_by(&key, &ca_cert, &ca_key)?;
        Ok(ClientCert { pem: cert.pem(), key_pem: key.serialize_pem() })
    };
    let good = client("CrlClientGood", GOOD_SERIAL)?;
    let revoked = client("CrlClientRevoked", REVOKED_SERIAL)?;

    let crl = CertificateRevocationListParams {
        this_update: date_time_ymd(2024, 1, 1),
        next_update: date_time_ymd(2999, 1, 1),
        crl_number: SerialNumber::from(1u64),
        issuing_distribution_point: None,
        revoked_certs: vec![RevokedCertParams {
            serial_number: SerialNumber::from(REVOKED_SERIAL),
            revocation_time: date_time_ymd(2024, 1, 1),
            reason_code: Some(RevocationReason::KeyCompromise),
            invalidity_date: None,
        }],
        key_identifier_method: KeyIdMethod::Sha256,
    }
    .signed_by(&ca_cert, &ca_key)?;

    Ok(CrlCerts {
        ca_pem: ca_cert.pem(),
        server_pem: server_cert.pem(),
        server_key_pem: server_key.serialize_pem(),
        crl_pem: crl.pem()?,
        good,
        revoked,
    })
}

struct CrlBroker {
    node: ClusterNode,
    certs: CrlCerts,
    dir: PathBuf,
}

impl Drop for CrlBroker {
    fn drop(&mut self) {
        self.node.kill();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn toml_path(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn spawn_crl_broker() -> anyhow::Result<CrlBroker> {
    let binary = rmqttd_binary();
    if !binary.exists() {
        return Err(anyhow::anyhow!(
            "rmqttd binary not found at {:?}; build it first (cargo build -p rmqttd)",
            binary
        ));
    }

    let uid = uuid::Uuid::new_v4().simple().to_string();
    let dir = std::env::temp_dir().join(format!("rmqtt-tls-crl-{uid}"));
    std::fs::create_dir_all(dir.join("plugins"))?;

    let certs = mint_certs()?;
    std::fs::write(dir.join("ca.pem"), &certs.ca_pem)?;
    std::fs::write(dir.join("server.pem"), &certs.server_pem)?;
    std::fs::write(dir.join("server.key"), &certs.server_key_pem)?;
    std::fs::write(dir.join("crl.pem"), &certs.crl_pem)?;

    let config_path = dir.join("rmqtt.toml");
    let tls_addr = format!("127.0.0.1:{TLS_PORT}");
    std::fs::write(
        &config_path,
        format!(
            "# Runtime-generated for the CRL revocation case.\n\
             node.id = 1\n\
             node.busy.check_enable = false\n\
             log.to = \"console\"\n\
             log.level = \"info\"\n\
             plugins.dir = \"{plugins_dir}\"\n\
             plugins.default_startups = []\n\
             plugins.disabled_default_startups = [\"rmqtt-acl\", \"rmqtt-counter\", \"rmqtt-http-api\"]\n\
             rpc.server_addr = \"127.0.0.1:{GRPC_PORT}\"\n\
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
             listener.tls.external.crl = \"{crl_pem}\"\n",
            plugins_dir = toml_path(&dir.join("plugins")),
            server_pem = toml_path(&dir.join("server.pem")),
            server_key = toml_path(&dir.join("server.key")),
            ca_pem = toml_path(&dir.join("ca.pem")),
            crl_pem = toml_path(&dir.join("crl.pem")),
        ),
    )?;

    let log_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target");
    let mut node = ClusterNode::new(config_path, &tls_addr, log_dir.join("tls-crl-node.log"));
    node.spawn(&binary)?;
    if !node.wait_healthy(NODE_START_TIMEOUT) {
        let tail = node.log_tail(30).unwrap_or_else(|| "<log file unreadable>".into());
        return Err(anyhow::anyhow!("tls-crl broker did not become healthy; log tail:\n{tail}"));
    }
    Ok(CrlBroker { node, certs, dir })
}

fn client_config(ca_pem: &str, client: &ClientCert) -> anyhow::Result<ClientConfig> {
    let mut roots = RootCertStore::empty();
    roots.add(CertificateDer::from_pem_slice(ca_pem.as_bytes())?)?;
    let chain = vec![CertificateDer::from_pem_slice(client.pem.as_bytes())?];
    let key = PrivateKeyDer::from_pem_slice(client.key_pem.as_bytes())?;
    Ok(ClientConfig::builder().with_root_certificates(roots).with_client_auth_cert(chain, key)?)
}

async fn connect_tls(cfg: ClientConfig) -> anyhow::Result<TlsStream<tokio::net::TcpStream>> {
    let connector = TlsConnector::from(Arc::new(cfg));
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", TLS_PORT)).await?;
    let server_name = ServerName::try_from(SERVER_NAME).map_err(|_| anyhow::anyhow!("bad server name"))?;
    Ok(connector.connect(server_name, stream).await?)
}

/// TLS handshake + MQTT CONNECT with `client`'s certificate. Under TLS 1.3
/// the server judges the client certificate after the client already
/// considers the handshake done, so a rejection may surface on the CONNECT
/// round-trip instead of on `connect`; both count as one attempt here.
async fn try_connect(certs: &CrlCerts, client: &ClientCert, client_id: &str) -> anyhow::Result<()> {
    let tls = connect_tls(client_config(&certs.ca_pem, client)?).await?;
    let mut mqtt = MqttOverTransport::connect(tls, client_id).await?;
    let _ = mqtt.disconnect().await;
    Ok(())
}

/// mTLS with a CRL: the revoked client is refused during the TLS handshake,
/// the non-revoked client from the same CA is accepted.
pub struct TransportTlsCrlTest;

impl TestCase for TransportTlsCrlTest {
    fn name(&self) -> &str {
        "transport_tls_crl"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            let broker = spawn_crl_broker()?;
            let id = uuid::Uuid::new_v4().simple().to_string();

            // Control: the CRL must not turn the listener into one that
            // refuses everybody.
            try_connect(&broker.certs, &broker.certs.good, &format!("crl-good-{id}"))
                .await
                .map_err(|e| anyhow::anyhow!("non-revoked client was refused: {e:#}"))?;

            match try_connect(&broker.certs, &broker.certs.revoked, &format!("crl-revoked-{id}")).await {
                Ok(()) => return Err(anyhow::anyhow!("revoked client certificate was accepted")),
                Err(e) => {
                    let msg = format!("{e:#}");
                    if !msg.contains("CertificateRevoked") {
                        return Err(anyhow::anyhow!(
                            "revoked client refused, but not with a CertificateRevoked alert: {msg}"
                        ));
                    }
                }
            }
            drop(broker);
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
