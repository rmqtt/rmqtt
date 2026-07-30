//! TLS certificate information extracted from MQTT client connections
//!
//! This module provides the `CertInfo` struct which holds X.509 certificate
//! metadata extracted during TLS handshake, including the Common Name,
//! subject distinguished name, serial number, and organization fields.

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
}

impl CertInfo {
    /// Creates a new `CertInfo` instance with default (empty) fields
    pub fn new() -> Self {
        Self::default()
    }
}

impl fmt::Display for CertInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CN: {:?}, Subject: {}, Org: {:?}", self.common_name, self.subject, self.organization)
    }
}
