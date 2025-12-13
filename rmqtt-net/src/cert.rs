#[cfg(feature = "ws")]
use crate::ws::WsStream;
use std::fmt;

/// TLS certificate information extracted from peer
#[derive(Debug, Clone, Default)]
pub struct CertInfo {
    /// Common Name from certificate subject
    pub common_name: Option<String>,
    /// Full subject distinguished name
    pub subject: String,
    /// Certificate serial number
    pub serial: Option<String>,
    /// Organization
    pub organization: Option<String>,
}

impl CertInfo {
    pub fn new() -> Self {
        Self::default()
    }
}

impl fmt::Display for CertInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CN: {:?}, Subject: {}, Org: {:?}", self.common_name, self.subject, self.organization)
    }
}

/// Trait for extracting TLS certificate information from streams
pub trait TlsCertExtractor {
    fn extract_cert_info(&self) -> Option<CertInfo>;
}

// Implementation for TLS streams
#[cfg(feature = "tls")]
impl<S> TlsCertExtractor for tokio_rustls::server::TlsStream<S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    fn extract_cert_info(&self) -> Option<CertInfo> {
        use x509_parser::parse_x509_certificate;

        let (_, session) = self.get_ref();
        let certs = session.peer_certificates()?;
        let cert = certs.first()?;

        let (_, parsed) = parse_x509_certificate(cert.as_ref()).ok()?;

        let common_name =
            parsed.subject().iter_common_name().next().and_then(|cn| cn.as_str().ok()).map(|s| s.to_string());

        let organization = parsed
            .subject()
            .iter_organization()
            .next()
            .and_then(|org| org.as_str().ok())
            .map(|s| s.to_string());

        let subject = parsed.subject().to_string();

        let serial = parsed.serial.to_str_radix(16);

        Some(CertInfo { common_name, subject, serial: Some(serial), organization })
    }
}

// Default implementation for non-TLS streams (TCP)
impl TlsCertExtractor for tokio::net::TcpStream {
    fn extract_cert_info(&self) -> Option<CertInfo> {
        None
    }
}

#[cfg(feature = "ws")]
impl TlsCertExtractor for WsStream<tokio::net::TcpStream> {
    fn extract_cert_info(&self) -> Option<CertInfo> {
        None
    }
}

#[cfg(feature = "ws")]
#[cfg(feature = "tls")]
impl TlsCertExtractor for WsStream<tokio_rustls::server::TlsStream<tokio::net::TcpStream>> {
    fn extract_cert_info(&self) -> Option<CertInfo> {
        self.get_inner().get_ref().extract_cert_info()
    }
}
