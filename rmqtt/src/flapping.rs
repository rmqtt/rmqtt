//! The connection-gate extension point.
//!
//! A gate screens connection attempts *before* the client is authenticated
//! (see [`crate::hook::HookManager::client_connect`]) and answers the offenders
//! with a refusal. The state behind that decision — the counters, the bans, the
//! exemptions — belongs to the plugin that implements the gate; the broker has
//! no reason to know about it. Two readers outside the plugin do: the
//! management API, where an operator lifts a ban by hand, and
//! [`crate::stats::Stats`], which publishes the number of bans in force.
//!
//! [`Flapping`] is the narrow view they share. The core defines the interface
//! and a no-op default; a plugin installs its implementation into
//! [`crate::extend::Manager::flapping`] from its `init`, exactly as the
//! retention, message-storage, delayed and auto-subscription plugins do.
//!
//! Nothing here runs on the connection path: the hot path goes through the
//! plugin's own hook handler, which reads its state directly. These methods
//! are only reached from the management API and from the periodic stats
//! snapshot, so their cost does not matter.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The identity a connection attempt is counted and banned by.
///
/// Every variant is a *dimension*: an independent policy and an independent
/// table. A connection is screened once per enabled dimension.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dimension {
    /// Keyed by the ClientId. Note that a client which connects with an empty
    /// ClientId gets a server-generated one, so those clients never share a key.
    ClientId,
    /// Keyed by the username. Connections that carry no username are not counted.
    UserName,
    /// Keyed by the source IP address.
    PeerHost,
}

impl Dimension {
    /// The lowercase, wire-visible name used by the HTTP API and in logs.
    #[inline]
    pub fn as_str(self) -> &'static str {
        match self {
            Dimension::ClientId => "clientid",
            Dimension::UserName => "username",
            Dimension::PeerHost => "peerhost",
        }
    }

    /// Parses [`Dimension::as_str`]. `None` when the name is unknown.
    #[inline]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "clientid" => Some(Dimension::ClientId),
            "username" => Some(Dimension::UserName),
            "peerhost" => Some(Dimension::PeerHost),
            _ => None,
        }
    }
}

impl fmt::Display for Dimension {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One ban in force, as reported by [`Flapping::banned`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BanInfo {
    /// The dimension this ban belongs to.
    pub dimension: Dimension,
    /// The ClientId, username or source IP the ban is keyed by.
    pub key: String,
    /// The number of attempts in the window that triggered the ban.
    pub count: usize,
    /// When the ban was created, in milliseconds since the epoch, formatted.
    pub banned_at: String,
    /// When the ban lapses, in milliseconds since the epoch, formatted.
    pub banned_until: String,
    /// Milliseconds left before the ban lapses, measured when the answer was built.
    pub remaining_ms: u64,
    /// ClientId of the attempt that triggered the ban.
    pub last_clientid: Option<String>,
    /// Source address of the attempt that triggered the ban.
    pub last_ipaddress: Option<String>,
}

/// The filter and pagination of a [`Flapping::banned`] query.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BanQuery {
    /// Restrict the answer to one dimension. `None` means every dimension.
    pub dimension: Option<Dimension>,
    /// Restrict the answer to one key (exact match). `None` means every key.
    pub key: Option<String>,
    /// Entries to skip before collecting the page.
    pub offset: usize,
    /// Maximum number of entries to collect. `0` means no limit.
    pub limit: usize,
}

/// Read access to the bans a connection gate holds.
///
/// Every method has a default so that [`DefaultFlapping`], the placeholder the
/// broker starts with, needs no code at all: with no gate installed the broker
/// simply reports "nothing is banned".
#[async_trait::async_trait]
pub trait Flapping: Sync + Send {
    /// Whether a gate is installed and currently enabled.
    ///
    /// A gate that is installed but switched off reports `false`, so this
    /// answers "the feature is not in use" rather than "nothing is banned". The
    /// management API publishes it as the `available` field of the ban list and
    /// as `flapping` in the feature summary — which is how a caller tells an
    /// empty ban table apart from a gate that is not screening at all.
    fn available(&self) -> bool {
        false
    }

    /// The bans in force, filtered and paginated as `query` asks.
    async fn banned(&self, query: BanQuery) -> Vec<BanInfo> {
        let _ = query;
        Vec::new()
    }

    /// The number of bans in force, across every dimension.
    async fn banned_count(&self) -> usize {
        0
    }

    /// The number of keys currently tracked by the window counters.
    async fn windows_count(&self) -> usize {
        0
    }

    /// Lifts the ban on `key` in `dimension`. Returns `true` when a ban was
    /// removed and `false` when there was nothing to remove.
    async fn unban(&self, dimension: Dimension, key: &str) -> bool {
        let _ = (dimension, key);
        false
    }
}

/// The placeholder used while no gate plugin is installed.
///
/// It reports `available() == false` and every other answer is the empty one.
#[derive(Copy, Clone, Debug, Default)]
pub struct DefaultFlapping;

#[async_trait::async_trait]
impl Flapping for DefaultFlapping {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimension_names_round_trip() {
        for dim in [Dimension::ClientId, Dimension::UserName, Dimension::PeerHost] {
            assert_eq!(Dimension::parse(dim.as_str()), Some(dim));
            assert_eq!(dim.to_string(), dim.as_str());
        }
        assert_eq!(Dimension::parse("ClientId"), None);
        assert_eq!(Dimension::parse(""), None);
    }

    #[test]
    fn dimension_serializes_as_its_wire_name() {
        assert_eq!(serde_json::to_string(&Dimension::ClientId).unwrap(), "\"clientid\"");
        assert_eq!(serde_json::to_string(&Dimension::PeerHost).unwrap(), "\"peerhost\"");
    }

    #[tokio::test]
    async fn default_flapping_reports_nothing() {
        let f = DefaultFlapping;
        assert!(!f.available());
        assert_eq!(f.banned_count().await, 0);
        assert_eq!(f.windows_count().await, 0);
        assert!(f.banned(BanQuery::default()).await.is_empty());
        assert!(!f.unban(Dimension::ClientId, "c1").await);
    }
}
