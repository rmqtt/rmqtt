//! TLS certificate information extraction from network streams.
//!
//! Provides the [`TlsCertExtractor`] trait and its implementations for TLS and non-TLS
//! stream types, enabling extraction of certificate metadata such as common name,
//! organization, subject, serial number, and the SPIFFE ID from the URI SAN.

#[cfg(feature = "ws")]
use crate::ws::WsStream;
use rmqtt_codec::cert::CertInfo;

/// Trait for extracting TLS certificate information from streams
pub trait TlsCertExtractor {
    /// Extracts certificate information (common name, organization, subject, serial,
    /// SPIFFE ID) from the underlying TLS session, if available.
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

        let spiffe_id = extract_spiffe_id(&parsed);

        Some(CertInfo { common_name, subject, serial: Some(serial), organization, spiffe_id })
    }
}

/// Extracts the SPIFFE ID from the certificate's SAN extension.
///
/// This is an identity mapping only — the SPIFFE leaf-certificate validation
/// rules (`cA=false`, key usage, non-root path) are deliberately not enforced
/// here; path validation is already done by the TLS layer.
#[cfg(feature = "tls")]
fn extract_spiffe_id(parsed: &x509_parser::certificate::X509Certificate<'_>) -> Option<String> {
    use x509_parser::extensions::GeneralName;

    match parsed.subject_alternative_name() {
        // The SAN extension appears more than once or is malformed: the
        // identity is ambiguous, treat it as "no SPIFFE ID".
        Err(e) => {
            log::warn!(
                "Client certificate has a duplicated or malformed SAN extension ({e}), \
                 skip SPIFFE username binding"
            );
            None
        }
        Ok(None) => None,
        Ok(Some(san)) => {
            let uris: Vec<&str> = san
                .value
                .general_names
                .iter()
                .filter_map(|gn| match gn {
                    GeneralName::URI(uri) => Some(*uri),
                    _ => None,
                })
                .collect();
            spiffe_id_from_uris(&uris)
        }
    }
}

/// Picks the SPIFFE ID out of the certificate's URI SANs.
///
/// A SPIFFE ID is accepted only when the certificate carries **exactly one**
/// URI SAN (SPIFFE X509-SVID spec: "An X.509 SVID MUST contain exactly one
/// URI SAN"), its scheme is `spiffe` (case-insensitive, RFC 3986) and its
/// trust domain is non-empty. Any other shape — no URI SAN, a non-SPIFFE URI,
/// or more than one URI SAN — yields `None`. The ambiguous case (more than
/// one URI SAN) is logged as a warning: binding an ambiguous identity as a
/// username would merge distinct workloads. A certificate with no URI SAN is
/// simply not a SPIFFE certificate and must not spam the log, so it stays
/// silent.
#[cfg(feature = "tls")]
fn spiffe_id_from_uris(uris: &[&str]) -> Option<String> {
    if uris.len() > 1 {
        log::warn!("Client certificate carries {} URI SANs, skip SPIFFE username binding", uris.len());
        return None;
    }

    let uri = *uris.first()?;
    let (scheme, rest) = uri.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("spiffe") {
        return None;
    }
    // The trust domain is everything up to the first '/', and must be non-empty.
    let trust_domain = rest.split('/').next().unwrap_or_default();
    if trust_domain.is_empty() {
        return None;
    }
    Some(uri.to_owned())
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

#[cfg(all(test, feature = "tls"))]
mod tests {
    use super::{extract_spiffe_id, spiffe_id_from_uris};
    use crate::builder::{Builder, SpiffeUsernameFormat};
    use crate::rustls::pki_types::pem::PemObject;
    use crate::rustls::pki_types::CertificateDer;
    use x509_parser::parse_x509_certificate;

    const SPIFFE_SINGLE: &str = include_str!("../tests/fixtures/spiffe_single.pem");
    const SPIFFE_MULTI: &str = include_str!("../tests/fixtures/spiffe_multi.pem");
    const NO_URI_SAN: &str = include_str!("../tests/fixtures/no_uri_san.pem");

    /// Parses a fixture PEM and asserts the extracted SPIFFE ID.
    fn assert_extracted(pem: &str, expected: Option<&str>) {
        let der = CertificateDer::from_pem_slice(pem.as_bytes()).expect("valid PEM");
        let (_, parsed) = parse_x509_certificate(der.as_ref()).expect("valid DER");
        assert_eq!(extract_spiffe_id(&parsed).as_deref(), expected);
    }

    #[test]
    fn single_spiffe_uri_san_is_extracted() {
        assert_extracted(SPIFFE_SINGLE, Some("spiffe://example.org/user/barry"));
    }

    #[test]
    fn two_uri_sans_are_not_bound() {
        assert_extracted(SPIFFE_MULTI, None);
    }

    #[test]
    fn certificate_without_uri_san_yields_none() {
        assert_extracted(NO_URI_SAN, None);
    }

    #[test]
    fn exactly_one_spiffe_uri_is_accepted() {
        assert_eq!(
            spiffe_id_from_uris(&["spiffe://example.org/user/barry"]).as_deref(),
            Some("spiffe://example.org/user/barry")
        );
        // Root path is still the identity the certificate declares.
        assert_eq!(spiffe_id_from_uris(&["spiffe://example.org"]).as_deref(), Some("spiffe://example.org"));
    }

    #[test]
    fn two_uri_sans_are_rejected() {
        assert_eq!(spiffe_id_from_uris(&["spiffe://a/x", "spiffe://b/y"]), None);
        // A SPIFFE URI mixed with any other URI SAN stays ambiguous.
        assert_eq!(spiffe_id_from_uris(&["spiffe://a/x", "https://b/y"]), None);
    }

    #[test]
    fn non_spiffe_uri_is_rejected() {
        assert_eq!(spiffe_id_from_uris(&["https://example.com/workload"]), None);
        assert_eq!(spiffe_id_from_uris(&["mailto:barry@example.com"]), None);
    }

    #[test]
    fn empty_trust_domain_is_rejected() {
        assert_eq!(spiffe_id_from_uris(&["spiffe:///user/barry"]), None);
        assert_eq!(spiffe_id_from_uris(&["spiffe://"]), None);
    }

    #[test]
    fn scheme_matching_is_case_insensitive() {
        assert_eq!(
            spiffe_id_from_uris(&["SPIFFE://example.org/user/barry"]).as_deref(),
            Some("SPIFFE://example.org/user/barry")
        );
    }

    #[test]
    fn no_uri_at_all_is_silent_none() {
        assert_eq!(spiffe_id_from_uris(&[]), None);
    }

    #[test]
    fn spiffe_username_format_uri_keeps_the_full_id() {
        assert_eq!(
            SpiffeUsernameFormat::Uri.apply("spiffe://example.com/user/barry").as_deref(),
            Some("spiffe://example.com/user/barry")
        );
    }

    #[test]
    fn spiffe_username_format_path_keeps_the_leading_slash() {
        assert_eq!(
            SpiffeUsernameFormat::Path.apply("spiffe://example.com/user/barry").as_deref(),
            Some("/user/barry")
        );
        assert_eq!(SpiffeUsernameFormat::Path.apply("spiffe://example.com/a/b/").as_deref(), Some("/a/b/"));
    }

    #[test]
    fn spiffe_username_format_name_takes_the_last_non_empty_segment() {
        assert_eq!(
            SpiffeUsernameFormat::Name.apply("spiffe://example.com/user/barry").as_deref(),
            Some("barry")
        );
        assert_eq!(SpiffeUsernameFormat::Name.apply("spiffe://example.com/a/b/").as_deref(), Some("b"));
    }

    #[test]
    fn root_or_empty_path_falls_through_for_path_and_name() {
        assert_eq!(SpiffeUsernameFormat::Path.apply("spiffe://example.com"), None);
        assert_eq!(SpiffeUsernameFormat::Path.apply("spiffe://example.com/"), None);
        assert_eq!(SpiffeUsernameFormat::Name.apply("spiffe://example.com"), None);
        assert_eq!(SpiffeUsernameFormat::Name.apply("spiffe://example.com/"), None);
        // `Uri` is deliberately lenient: it is still the identity the cert declares.
        assert_eq!(
            SpiffeUsernameFormat::Uri.apply("spiffe://example.com").as_deref(),
            Some("spiffe://example.com")
        );
    }

    #[test]
    fn apply_rejects_non_spiffe_input() {
        // `Path` / `Name` require the spiffe scheme; `Uri` is deliberately
        // lenient and returns the input as-is — apply() is only ever called
        // with a value `spiffe_id_from_uris` already validated.
        assert_eq!(SpiffeUsernameFormat::Path.apply("https://example.com/x"), None);
        assert_eq!(SpiffeUsernameFormat::Name.apply("https://example.com/x"), None);
        assert_eq!(SpiffeUsernameFormat::Path.apply("not-a-spiffe-id"), None);
        assert_eq!(SpiffeUsernameFormat::Name.apply("spiffe:///x"), None);
    }

    #[test]
    fn apply_matches_the_scheme_case_insensitively() {
        assert_eq!(
            SpiffeUsernameFormat::Name.apply("SPIFFE://example.com/user/barry").as_deref(),
            Some("barry")
        );
    }

    #[test]
    fn builder_defaults_keep_spiffe_binding_off() {
        let b = Builder::new();
        assert!(!b.cert_spiffe_as_username);
        assert_eq!(b.cert_spiffe_username_format, SpiffeUsernameFormat::Uri);
    }
}
