//! TLS certificate information extracted from MQTT client connections
//!
//! This module provides the `CertInfo` struct which holds X.509 certificate
//! metadata extracted during TLS handshake, including the Common Name,
//! subject distinguished name, serial number, organization fields, and the
//! SPIFFE ID from the URI SAN.

use serde::{Deserialize, Serialize};
use std::fmt;

/// TLS certificate information extracted from peer
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertInfo {
    /// Common Name from certificate subject
    pub common_name: Option<String>,
    /// Full subject distinguished name
    pub subject: String,
    /// Certificate serial number
    pub serial: Option<String>,
    /// Organization
    pub organization: Option<String>,
    /// SPIFFE ID from the certificate URI SAN (e.g. `spiffe://example.com/user/barry`).
    ///
    /// `Some` only when the certificate carries exactly one URI SAN and its
    /// scheme is `spiffe` with a non-empty trust domain; `None` when there is
    /// no URI SAN, the URI is not a SPIFFE ID, or the SAN is ambiguous
    /// (more than one URI SAN).
    pub spiffe_id: Option<String>,
}

impl CertInfo {
    /// Creates a new `CertInfo` instance with default (empty) fields
    pub fn new() -> Self {
        Self::default()
    }
}

impl fmt::Display for CertInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CN: {:?}, Subject: {}, Org: {:?}, Spiffe: {:?}",
            self.common_name, self.subject, self.organization, self.spiffe_id
        )
    }
}
