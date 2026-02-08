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
    pub fn new() -> Self {
        Self::default()
    }
}

impl fmt::Display for CertInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CN: {:?}, Subject: {}, Org: {:?}", self.common_name, self.subject, self.organization)
    }
}
