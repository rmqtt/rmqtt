//! Configuration of the flapping plugin.
//!
//! The layout follows EMQX's `flapping_detect`: a policy per dimension, each
//! with its own window, threshold and ban time. A dimension is enabled by
//! *presenting* its policy table, so the shipped configuration enables
//! `by_clientid` only and leaves the other two commented out.

use std::time::Duration;

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

use rmqtt::{
    types::QoS,
    utils::{deserialize_duration, to_duration},
    Result,
};

/// The policy of a single dimension.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Policy {
    /// The width of the sliding window attempts are counted in.
    #[serde(default = "Policy::window_time_default", deserialize_with = "deserialize_duration")]
    pub window_time: Duration,

    /// The number of attempts inside the window that triggers a ban. The
    /// attempt that reaches the count is itself refused.
    #[serde(default = "Policy::max_count_default")]
    pub max_count: usize,

    /// How long a ban lasts. A ban is never extended while it is in force, so
    /// the offender regains a full budget of attempts the moment it lapses.
    #[serde(default = "Policy::ban_time_default", deserialize_with = "deserialize_duration")]
    pub ban_time: Duration,
}

impl Policy {
    /// The window width in milliseconds.
    #[inline]
    pub fn window_ms(&self) -> u64 {
        self.window_time.as_millis() as u64
    }

    /// The ban time in milliseconds.
    #[inline]
    pub fn ban_ms(&self) -> u64 {
        self.ban_time.as_millis() as u64
    }

    #[inline]
    fn window_time_default() -> Duration {
        Duration::from_secs(60)
    }

    #[inline]
    fn max_count_default() -> usize {
        15
    }

    #[inline]
    fn ban_time_default() -> Duration {
        Duration::from_secs(300)
    }

    /// A threshold of zero would ban the very first attempt, which is a
    /// configuration mistake rather than an intent.
    #[inline]
    fn sanitize(&mut self, dim: &str) {
        if self.max_count == 0 {
            log::warn!("flapping: {dim}.max_count is 0, using 1");
            self.max_count = 1;
        }
    }
}

/// Top-level configuration of the flapping plugin.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    /// Whether the gate screens connections at all.
    #[serde(default = "PluginConfig::enable_default")]
    pub enable: bool,

    /// Upper bound on the keys tracked *per dimension*.
    ///
    /// A key is only admitted while the table has room, so a flood of random
    /// ClientIds cannot grow it without bound. When the table is full the gate
    /// runs one cleanup pass and, if that freed nothing, lets the attempt
    /// through untracked: refusing it would turn a flood of unknown keys into
    /// an outage for everyone else.
    #[serde(default = "PluginConfig::max_track_default")]
    pub max_track: usize,

    /// How often the sweep task runs. A sweep costs one clock comparison when
    /// nothing is due, so this only trades memory retention against wake-ups.
    #[serde(
        default = "PluginConfig::gc_interval_default",
        deserialize_with = "PluginConfig::deserialize_gc_interval"
    )]
    pub gc_interval: Duration,

    /// Counts connection attempts that share a ClientId. Omit the table to
    /// disable the dimension.
    #[serde(default)]
    pub by_clientid: Option<Policy>,

    /// Counts connection attempts that share a username. Connections without a
    /// username are not counted.
    #[serde(default)]
    pub by_username: Option<Policy>,

    /// Counts connection attempts that come from the same source IP address.
    #[serde(default)]
    pub by_peerhost: Option<Policy>,

    /// Publishes one `$SYS` message per newly created ban.
    #[serde(default = "PluginConfig::notify_sys_topic_default")]
    pub notify_sys_topic: bool,

    /// The topic bans are announced on. `{node}` is replaced with the node id.
    #[serde(default = "PluginConfig::sys_topic_default")]
    pub sys_topic: String,

    /// QoS of the announcement. At most once by default: a ban notice must not
    /// be retried into a business system that is already struggling.
    #[serde(
        default = "PluginConfig::sys_topic_qos_default",
        deserialize_with = "PluginConfig::deserialize_qos"
    )]
    pub sys_topic_qos: QoS,

    /// Lifetime of the announcement, consumed by `rmqtt-retainer` when it
    /// happens to be loaded.
    #[serde(
        default = "PluginConfig::message_expiry_interval_default",
        deserialize_with = "deserialize_duration"
    )]
    pub message_expiry_interval: Duration,

    /// Announces every attempt refused while a ban is in force, not just the
    /// ban itself. Off by default: a determined offender would otherwise fill
    /// the topic with one message per reconnect.
    #[serde(default)]
    pub notify_on_every_refusal: bool,

    /// Announces a ban lifted through the management API. Bans that simply
    /// lapse are not announced, so the topic stays free of routine noise.
    #[serde(default = "PluginConfig::notify_on_unban_default")]
    pub notify_on_unban: bool,

    /// The topic manual unbans are announced on. `{node}` is replaced with the
    /// node id.
    #[serde(default = "PluginConfig::sys_topic_unban_default")]
    pub sys_topic_unban: String,

    /// ClientIds that are never counted and never banned (exact match).
    #[serde(default)]
    pub allow_clientids: Vec<String>,

    /// Usernames that are never counted and never banned (exact match).
    #[serde(default)]
    pub allow_usernames: Vec<String>,

    /// Source IP addresses that are never counted and never banned (exact match).
    #[serde(default)]
    pub allow_peerhosts: Vec<String>,
}

impl PluginConfig {
    #[inline]
    fn enable_default() -> bool {
        true
    }

    #[inline]
    fn max_track_default() -> usize {
        100_000
    }

    #[inline]
    fn gc_interval_default() -> Duration {
        Duration::from_secs(10)
    }

    #[inline]
    fn notify_sys_topic_default() -> bool {
        true
    }

    #[inline]
    fn sys_topic_default() -> String {
        String::from("$SYS/brokers/{node}/flapping/banned")
    }

    #[inline]
    fn sys_topic_qos_default() -> QoS {
        QoS::AtMostOnce
    }

    #[inline]
    fn message_expiry_interval_default() -> Duration {
        Duration::from_secs(300)
    }

    #[inline]
    fn notify_on_unban_default() -> bool {
        true
    }

    #[inline]
    fn sys_topic_unban_default() -> String {
        String::from("$SYS/brokers/{node}/flapping/unbanned")
    }

    /// Clamps the values that would either spin a task or ban everyone.
    #[inline]
    pub fn sanitize(&mut self) {
        if self.max_track == 0 {
            log::warn!("flapping: max_track is 0, using 1");
            self.max_track = 1;
        }
        if let Some(policy) = self.by_clientid.as_mut() {
            policy.sanitize("by_clientid");
        }
        if let Some(policy) = self.by_username.as_mut() {
            policy.sanitize("by_username");
        }
        if let Some(policy) = self.by_peerhost.as_mut() {
            policy.sanitize("by_peerhost");
        }
    }

    /// Whether any dimension is enabled. With every dimension off the gate has
    /// nothing to screen and would only cost a hash lookup per attempt.
    #[inline]
    pub fn is_any_dimension_enabled(&self) -> bool {
        self.enable
            && (self.by_clientid.is_some() || self.by_username.is_some() || self.by_peerhost.is_some())
    }

    /// The names of the dimensions in evaluation order, for the startup log.
    #[inline]
    pub fn enabled_dimensions(&self) -> Vec<&'static str> {
        let mut dims = Vec::with_capacity(3);
        if self.by_clientid.is_some() {
            dims.push("clientid");
        }
        if self.by_username.is_some() {
            dims.push("username");
        }
        if self.by_peerhost.is_some() {
            dims.push("peerhost");
        }
        dims
    }

    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }

    /// Rejects an interval below one second: a sweep that runs hot is a
    /// configuration mistake, and the reload path has no other guard.
    #[inline]
    fn deserialize_gc_interval<'de, D>(deserializer: D) -> std::result::Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = String::deserialize(deserializer)?;
        let d = to_duration(&v);
        if d < Duration::from_secs(1) {
            Err(de::Error::custom("'gc_interval' must be at least 1 second"))
        } else {
            Ok(d)
        }
    }

    #[inline]
    fn deserialize_qos<'de, D>(deserializer: D) -> std::result::Result<QoS, D::Error>
    where
        D: Deserializer<'de>,
    {
        match u8::deserialize(deserializer)? {
            0 => Ok(QoS::AtMostOnce),
            1 => Ok(QoS::AtLeastOnce),
            2 => Ok(QoS::ExactlyOnce),
            _ => Err(de::Error::custom("QoS configuration error, only values (0,1,2) are supported")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_documented_values() {
        let cfg: PluginConfig = toml::from_str("").unwrap();
        assert!(cfg.enable);
        assert_eq!(cfg.max_track, 100_000);
        assert_eq!(cfg.gc_interval, Duration::from_secs(10));
        assert!(cfg.by_clientid.is_none());
        assert!(cfg.notify_sys_topic);
        assert_eq!(cfg.sys_topic, "$SYS/brokers/{node}/flapping/banned");
        assert_eq!(cfg.sys_topic_qos, QoS::AtMostOnce);
        assert!(cfg.notify_on_unban);
        assert!(!cfg.notify_on_every_refusal);
        assert!(cfg.enabled_dimensions().is_empty());
        assert!(!cfg.is_any_dimension_enabled());
    }

    #[test]
    fn a_present_policy_table_enables_its_dimension() {
        let cfg: PluginConfig = toml::from_str(
            r#"
            [by_clientid]
            window_time = "5s"
            max_count = 3
            ban_time = "3s"
            "#,
        )
        .unwrap();
        let policy = cfg.by_clientid.as_ref().unwrap();
        assert_eq!(policy.window_time, Duration::from_secs(5));
        assert_eq!(policy.max_count, 3);
        assert_eq!(policy.ban_time, Duration::from_secs(3));
        assert_eq!(cfg.enabled_dimensions(), vec!["clientid"]);
        assert!(cfg.is_any_dimension_enabled());
    }

    #[test]
    fn a_policy_may_omit_its_fields() {
        let cfg: PluginConfig = toml::from_str("[by_peerhost]\n").unwrap();
        let policy = cfg.by_peerhost.as_ref().unwrap();
        assert_eq!(policy.window_time, Duration::from_secs(60));
        assert_eq!(policy.max_count, 15);
        assert_eq!(policy.ban_time, Duration::from_secs(300));
    }

    #[test]
    fn sanitize_replaces_the_values_that_would_break_the_gate() {
        let mut cfg: PluginConfig = toml::from_str(
            r#"
            max_track = 0
            [by_clientid]
            max_count = 0
            "#,
        )
        .unwrap();
        cfg.sanitize();
        assert_eq!(cfg.max_track, 1);
        assert_eq!(cfg.by_clientid.as_ref().unwrap().max_count, 1);
    }

    #[test]
    fn a_hot_sweep_interval_is_rejected() {
        assert!(toml::from_str::<PluginConfig>("gc_interval = \"500ms\"").is_err());
        assert!(toml::from_str::<PluginConfig>("gc_interval = \"1s\"").is_ok());
    }

    #[test]
    fn an_out_of_range_qos_is_rejected() {
        assert!(toml::from_str::<PluginConfig>("sys_topic_qos = 3").is_err());
        assert!(toml::from_str::<PluginConfig>("sys_topic_qos = 1").is_ok());
    }
}
