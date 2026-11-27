//! The flapping detector.
//!
//! A *flapping* client is one that reconnects over and over — a broken device,
//! a runaway retry loop, or an attempt to brute-force credentials. This gate
//! counts connection attempts per ClientId, username and source address in a
//! sliding window, and answers the offender with a refusal for a while once it
//! crosses the threshold.
//!
//! # Where the decision is taken
//!
//! The gate is driven from the `ClientConnect` hook, which runs before
//! authentication and before the existing session is taken over. That is not an
//! optimisation: it is the only hook a client which sends no username reaches
//! while anonymous access is enabled (see
//! [`rmqtt::hook::HookManager::client_authenticate`]), so a policy that must
//! see *every* attempt can live nowhere else.
//!
//! # What is counted
//!
//! Attempts, not successes. While a ban is in force the gate short-circuits
//! before touching the window, so the attempts that pile up during a ban are
//! not counted — and, because a ban is never extended, they change nothing.
//!
//! # The two tables of a dimension
//!
//! * `wins` — the sliding window of one key: the timestamps of its recent
//!   attempts. Its length is capped at `max_count` because the attempt that
//!   reaches the threshold creates a ban and the window is dropped with it.
//! * `bans` — the keys currently refused, each with the moment it lapses.
//!
//! Dropping the window when the ban is created is load-bearing: with
//! `ban_time < window_time` the old timestamps would still be inside the
//! window when the ban lapses, so the first reconnect after a ban would
//! re-trigger it and the offender would be banned forever. Dropping the window
//! hands a lapsed ban a fresh budget, which is also what EMQX does.
//!
//! # Cleaning up
//!
//! Each table has a [`DeadlineIndex`] beside it. The connection path never
//! touches an index — it only registers a pair when a key is first tracked —
//! so a sweep costs O(entries due) rather than O(keys tracked).

use std::collections::{HashSet, VecDeque};
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytestring::ByteString;
use tokio::sync::RwLock;

use rmqtt::{
    context::ServerContext,
    flapping::{BanInfo, BanQuery, Dimension},
    types::{ClientId, ConnectInfo, ConnectRefuse, DashMap, NodeId, QoS, UserName},
};

use super::{DeadlineIndex, Gate};
use crate::config::{PluginConfig, Policy};
use crate::notify::{self, Notifier};

/// The key a dimension counts by.
///
/// Implemented for the two shapes the dimensions need: the text keys
/// (`ClientId` and `UserName`, which are both [`ByteString`]) and the source
/// address. The textual form is what an operator sees in the management API
/// and in `$SYS` messages, and what they type to lift a ban by hand.
pub(crate) trait Key: Ord + std::hash::Hash + Eq + Send + Sync + 'static {
    /// Parses the textual form accepted by the management API.
    fn parse(text: &str) -> Option<Self>
    where
        Self: Sized;
    /// The textual form used in logs, notifications and API answers.
    fn as_text(&self) -> String;
}

impl Key for ByteString {
    #[inline]
    fn parse(text: &str) -> Option<Self> {
        Some(ByteString::from(text))
    }

    #[inline]
    fn as_text(&self) -> String {
        String::from_utf8_lossy(self.as_ref()).into_owned()
    }
}

impl Key for IpAddr {
    #[inline]
    fn parse(text: &str) -> Option<Self> {
        text.parse().ok()
    }

    #[inline]
    fn as_text(&self) -> String {
        self.to_string()
    }
}

/// The waiting room of a key that is currently refused.
struct Ban {
    /// The moment the ban lapses.
    until: u64,
    /// The moment the ban was created.
    banned_at: u64,
    /// The number of attempts in the window that triggered it.
    count: usize,
    /// The ClientId of the attempt that triggered it.
    last_clientid: Option<String>,
    /// The source address of the attempt that triggered it.
    last_ipaddress: Option<String>,
}

/// The sliding window of one key.
struct Window {
    /// Attempt timestamps inside the window, oldest first.
    ts: VecDeque<u64>,
    /// This window's own width. Kept per entry rather than read from the live
    /// configuration so that an entry stays self-describing while a reload is
    /// in flight.
    window_ms: u64,
    /// The deadline currently registered for this key in the window index.
    indexed: u64,
}

impl Window {
    /// The moment this window becomes empty: its newest attempt plus its width.
    #[inline]
    fn empty_at(&self) -> u64 {
        self.ts.back().map_or(0, |ts| ts + self.window_ms)
    }

    #[inline]
    fn is_empty_at(&self, now: u64) -> bool {
        self.ts.back().is_none_or(|ts| ts + self.window_ms <= now)
    }
}

/// What one screening of one key concluded.
enum Verdict {
    /// Nothing to report: the attempt was counted, or deliberately not tracked.
    Allow,
    /// A ban is in force; this attempt was not counted.
    Denied,
    /// This attempt reached the threshold and created the ban.
    Banned { count: usize, until: u64 },
}

/// The state of one dimension: two authoritative tables, each with a deadline
/// index beside it.
struct KeyTable<K> {
    dim: Dimension,
    /// The windows of the keys currently counted.
    wins: DashMap<Arc<K>, Window>,
    /// The bans currently in force.
    bans: DashMap<Arc<K>, Ban>,
    win_idx: DeadlineIndex<K>,
    ban_idx: DeadlineIndex<K>,
    /// Number of keys in `wins`. An atomic because the admission check runs on
    /// the connection path, where summing the shard lengths of a `DashMap`
    /// would be too expensive to do per attempt.
    tracked: AtomicUsize,
    /// How many attempts have been given up on because the table was full, for
    /// the rate-limited warning.
    saturated: AtomicUsize,
}

impl<K: Key> KeyTable<K> {
    fn new(dim: Dimension) -> Self {
        Self {
            dim,
            wins: DashMap::default(),
            bans: DashMap::default(),
            win_idx: DeadlineIndex::new(),
            ban_idx: DeadlineIndex::new(),
            tracked: AtomicUsize::new(0),
            saturated: AtomicUsize::new(0),
        }
    }

    /// Screens one attempt keyed by `key`.
    ///
    /// `last_clientid` and `last_ipaddress` are recorded on a ban, so an
    /// operator lifting it can see who was behind it.
    fn check(
        &self,
        key: Arc<K>,
        policy: &Policy,
        max_track: usize,
        now: u64,
        last_clientid: Option<String>,
        last_ipaddress: Option<String>,
    ) -> Verdict {
        // A ban still in force short-circuits: the offender is refused without
        // its window being touched.
        if self.bans.get(&key).is_some_and(|ban| ban.until > now) {
            return Verdict::Denied;
        }

        // The ban has lapsed since the last sweep. Drop it now rather than
        // waiting for the sweep, so the key stops occupying a slot and the next
        // attempt starts from a full budget.
        if let Some((key, ban)) = self.bans.remove_if(&key, |_, ban| ban.until <= now) {
            self.ban_idx.remove(ban.until, key);
        }

        // Admission control for the window table. Below the bound this is a
        // single relaxed load; at the bound one cleanup pass is paid before the
        // key is given up, because the sweep may well free the room.
        if self.tracked.load(Ordering::Relaxed) >= max_track && !self.wins.contains_key(&key) {
            self.gc(now);
            if self.tracked.load(Ordering::Relaxed) >= max_track {
                self.warn_saturated();
                // Fail open, untracked: this key is simply not counted. The
                // alternative — refusing it — would let a flood of random
                // ClientIds lock out every well-behaved client.
                return Verdict::Allow;
            }
        }

        let window_ms = policy.window_ms();
        let windows = &self.wins;
        let win_idx = &self.win_idx;
        let tracked = &self.tracked;
        let deadline = now + window_ms;
        let entry_key = Arc::clone(&key);
        let mut window = windows.entry(entry_key).or_insert_with(|| {
            // First sighting: the key becomes tracked and its deadline is
            // registered — the only index write the connection path ever makes.
            tracked.fetch_add(1, Ordering::Relaxed);
            win_idx.insert(deadline, Arc::clone(&key));
            Window { ts: VecDeque::with_capacity(policy.max_count.min(64)), window_ms, indexed: deadline }
        });

        // Slide the window, then count this attempt.
        let cutoff = now.saturating_sub(window.window_ms);
        while window.ts.front().is_some_and(|ts| *ts <= cutoff) {
            window.ts.pop_front();
        }
        window.ts.push_back(now);

        if window.ts.len() < policy.max_count {
            return Verdict::Allow;
        }

        let count = window.ts.len();
        let indexed = window.indexed;
        drop(window);

        // The ban replaces the window. The guard is the window's own
        // registration, so a window another thread has already replaced is left
        // alone.
        if let Some((removed_key, _)) = windows.remove_if(&key, |_, window| window.indexed == indexed) {
            tracked.fetch_sub(1, Ordering::Relaxed);
            win_idx.remove(indexed, removed_key);
        }

        let until = now + policy.ban_ms();
        self.bans
            .insert(Arc::clone(&key), Ban { until, banned_at: now, count, last_clientid, last_ipaddress });
        self.ban_idx.insert(until, key);

        Verdict::Banned { count, until }
    }

    /// Drops the windows that have emptied and the bans that have lapsed.
    ///
    /// Only the heads of the two indexes are looked at, so the cost tracks the
    /// number of entries that actually expired since the last pass.
    fn gc(&self, now: u64) {
        for (_, key) in self.win_idx.pop_expired(now) {
            if self.wins.get(&key).is_none_or(|window| window.is_empty_at(now)) {
                if let Some((removed_key, _)) = self.wins.remove_if(&key, |_, w| w.is_empty_at(now)) {
                    self.tracked.fetch_sub(1, Ordering::Relaxed);
                    drop(removed_key);
                }
                // Nothing removed means another thread got there first — it
                // created a ban, or another sweep did — and the index pair goes
                // away with the window.
                continue;
            }
            // Still receiving attempts: re-register with the deadline it has
            // now. This is also what makes a pair left behind by a race
            // harmless — it is corrected here, or dropped above.
            let current = self.wins.get(&key).map(|window| window.empty_at());
            if let Some(current) = current {
                if current > now {
                    self.win_idx.insert(current, Arc::clone(&key));
                }
            }
        }

        for (deadline, key) in self.ban_idx.pop_expired(now) {
            if self.bans.get(&key).is_none_or(|ban| ban.until <= now) {
                self.bans.remove_if(&key, |_, ban| ban.until <= now);
            } else {
                // Unreachable while bans are never extended; kept so that the
                // index stays correct if that ever changes.
                let until = self.bans.get(&key).map(|ban| ban.until);
                if let Some(until) = until {
                    if until > deadline {
                        self.ban_idx.insert(until, Arc::clone(&key));
                    }
                }
            }
        }
    }

    /// Lifts a ban. Returns the removed ban so the caller can announce it.
    fn unban(&self, key: &Arc<K>) -> Option<Ban> {
        let (key, ban) = self.bans.remove(key)?;
        self.ban_idx.remove(ban.until, key);
        Some(ban)
    }

    /// Appends the bans in force to `out`.
    fn live_bans(&self, now: u64, out: &mut Vec<BanInfo>) {
        for entry in self.bans.iter() {
            let ban = entry.value();
            if ban.until > now {
                out.push(BanInfo {
                    dimension: self.dim,
                    key: entry.key().as_text(),
                    count: ban.count,
                    banned_at: notify::fmt_time(ban.banned_at),
                    banned_until: notify::fmt_time(ban.until),
                    remaining_ms: ban.until.saturating_sub(now),
                    last_clientid: ban.last_clientid.clone(),
                    last_ipaddress: ban.last_ipaddress.clone(),
                });
            }
        }
    }

    /// The number of bans in force.
    fn live_ban_count(&self, now: u64) -> usize {
        self.bans.iter().filter(|entry| entry.value().until > now).count()
    }

    /// Warns about a full table at most once per thousand rejections, so that a
    /// sustained flood cannot turn the log into the bottleneck.
    fn warn_saturated(&self) {
        let n = self.saturated.fetch_add(1, Ordering::Relaxed);
        if n.is_multiple_of(1000) {
            log::warn!(
                "flapping: the {}-key table is saturated at {} entries, new keys are not counted \
                 (raise max_track or tighten the policy)",
                self.dim.as_str(),
                self.tracked.load(Ordering::Relaxed),
            );
        }
    }
}

/// The three exemption lists, as lookup sets.
///
/// Held behind a lock of its own so that a configuration reload that changes a
/// list takes effect on the next attempt, without rebuilding the tables (which
/// would throw away every ban in force).
#[derive(Default)]
pub(crate) struct Exempt {
    clientids: HashSet<String>,
    usernames: HashSet<String>,
    peerhosts: HashSet<String>,
}

impl Exempt {
    pub(crate) fn from_config(cfg: &PluginConfig) -> Self {
        Self {
            clientids: cfg.allow_clientids.iter().cloned().collect(),
            usernames: cfg.allow_usernames.iter().cloned().collect(),
            peerhosts: cfg.allow_peerhosts.iter().cloned().collect(),
        }
    }

    /// The exemption list that belongs to `dim`.
    fn for_dimension(&self, dim: Dimension) -> &HashSet<String> {
        match dim {
            Dimension::ClientId => &self.clientids,
            Dimension::UserName => &self.usernames,
            Dimension::PeerHost => &self.peerhosts,
        }
    }
}

/// The flapping detector.
pub(crate) struct FlappingGate {
    clientid: KeyTable<ClientId>,
    username: KeyTable<UserName>,
    peerhost: KeyTable<IpAddr>,
    exempt: parking_lot::RwLock<Arc<Exempt>>,
    cfg: Arc<RwLock<PluginConfig>>,
    notifier: Arc<Notifier>,
    node_id: NodeId,
    /// The broker context, used for the two counters the gate feeds.
    scx: ServerContext,
    /// Whether the gate is switched on. Kept apart from the configuration so
    /// that `available()` can answer without taking an async lock.
    enabled: AtomicBool,
}

impl FlappingGate {
    pub(crate) async fn new(
        scx: &ServerContext,
        cfg: Arc<RwLock<PluginConfig>>,
        notifier: Arc<Notifier>,
    ) -> Self {
        let exempt = {
            let configured = cfg.read().await;
            Arc::new(Exempt::from_config(&configured))
        };
        Self {
            clientid: KeyTable::new(Dimension::ClientId),
            username: KeyTable::new(Dimension::UserName),
            peerhost: KeyTable::new(Dimension::PeerHost),
            exempt: parking_lot::RwLock::new(exempt),
            cfg,
            notifier,
            node_id: scx.node.id(),
            scx: scx.clone(),
            enabled: AtomicBool::new(false),
        }
    }

    /// Replaces the exemption lists, after a configuration reload.
    #[inline]
    pub(crate) fn set_exempt(&self, exempt: Exempt) {
        *self.exempt.write() = Arc::new(exempt);
    }

    #[inline]
    pub(crate) fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    #[inline]
    pub(crate) fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// The bans in force, filtered and paginated.
    ///
    /// Only this node's bans are visible: the counters and the table are
    /// node-local, so an aggregate would advertise bans this node does not
    /// enforce.
    pub(crate) fn banned(&self, query: &BanQuery, now: u64) -> Vec<BanInfo> {
        let mut found = Vec::new();
        let wanted = query.dimension;
        if wanted.is_none() || wanted == Some(Dimension::ClientId) {
            self.clientid.live_bans(now, &mut found);
        }
        if wanted.is_none() || wanted == Some(Dimension::UserName) {
            self.username.live_bans(now, &mut found);
        }
        if wanted.is_none() || wanted == Some(Dimension::PeerHost) {
            self.peerhost.live_bans(now, &mut found);
        }
        if let Some(key) = query.key.as_deref() {
            found.retain(|info| info.key == key);
        }
        // A stable order across the three tables, so pagination is repeatable.
        // The timestamps are formatted, and that format sorts chronologically.
        found.sort_by(|a, b| a.banned_at.cmp(&b.banned_at));

        found
            .into_iter()
            .skip(query.offset)
            .take(if query.limit == 0 { usize::MAX } else { query.limit })
            .collect()
    }

    #[inline]
    pub(crate) fn banned_count(&self, now: u64) -> usize {
        self.clientid.live_ban_count(now)
            + self.username.live_ban_count(now)
            + self.peerhost.live_ban_count(now)
    }

    /// The number of keys being tracked by the window counters.
    #[inline]
    pub(crate) fn windows_count(&self) -> usize {
        self.clientid.tracked.load(Ordering::Relaxed)
            + self.username.tracked.load(Ordering::Relaxed)
            + self.peerhost.tracked.load(Ordering::Relaxed)
    }

    /// Lifts the ban on `key`, announcing it when the configuration asks for it.
    pub(crate) async fn unban(&self, dimension: Dimension, key: &str) -> bool {
        let removed = match dimension {
            Dimension::ClientId => {
                <ClientId as Key>::parse(key).and_then(|k| self.clientid.unban(&Arc::new(k)))
            }
            Dimension::UserName => {
                <UserName as Key>::parse(key).and_then(|k| self.username.unban(&Arc::new(k)))
            }
            Dimension::PeerHost => {
                <IpAddr as Key>::parse(key).and_then(|k| self.peerhost.unban(&Arc::new(k)))
            }
        };
        let Some(ban) = removed else {
            return false;
        };
        log::info!(
            "flapping: the ban on {} '{}' was lifted by hand, {} ms after it was created",
            dimension,
            key,
            crate::now_ms().saturating_sub(ban.banned_at),
        );
        let cfg = self.cfg.read().await;
        if cfg.notify_sys_topic && cfg.notify_on_unban {
            let (topic, qos, expiry) = self.unban_channel(&cfg);
            let payload = notify::unban_payload(
                self.node_id,
                dimension,
                key,
                ban.count,
                ban.banned_at,
                ban.until,
                ban.last_clientid.as_deref(),
                ban.last_ipaddress.as_deref(),
            );
            self.notifier.publish(topic, qos, expiry, payload);
        }
        true
    }

    /// The `$SYS` channel ban announcements go out on.
    fn ban_channel(&self, cfg: &PluginConfig) -> (String, QoS, Duration) {
        (notify::resolve_topic(&cfg.sys_topic, self.node_id), cfg.sys_topic_qos, cfg.message_expiry_interval)
    }

    /// The `$SYS` channel manual-unban announcements go out on.
    fn unban_channel(&self, cfg: &PluginConfig) -> (String, QoS, Duration) {
        (
            notify::resolve_topic(&cfg.sys_topic_unban, self.node_id),
            cfg.sys_topic_qos,
            cfg.message_expiry_interval,
        )
    }

    /// Screens one dimension, announcing and counting whatever it decides.
    fn screen<K: Key>(
        &self,
        cfg: &PluginConfig,
        table: &KeyTable<K>,
        policy: &Policy,
        key: K,
        ci: &ConnectInfo,
        now: u64,
    ) -> Option<ConnectRefuse> {
        let text = key.as_text();
        {
            let exempt = self.exempt.read();
            if exempt.for_dimension(table.dim).contains(&text) {
                return None;
            }
        }

        let last_clientid = Some(ci.client_id().as_text());
        let last_ipaddress = ci.id().remote_addr.map(|addr| addr.to_string());
        let verdict = table.check(Arc::new(key), policy, cfg.max_track, now, last_clientid, last_ipaddress);
        match verdict {
            Verdict::Allow => None,
            Verdict::Denied => {
                // `$SYS` only when asked for: one message per reconnect from a
                // banned client is exactly the flood the ban is meant to stop.
                if cfg.notify_sys_topic && cfg.notify_on_every_refusal {
                    let (topic, qos, expiry) = self.ban_channel(cfg);
                    let payload = notify::refused_payload(self.node_id, table.dim, &text, ci, now);
                    self.notifier.publish(topic, qos, expiry, payload);
                }
                self.scx.metrics.conn_flapping_refused_inc();
                // Only `debug`: a banned client retrying is exactly the repeat
                // this line would otherwise add one of per attempt, while the
                // *new* ban (the `Verdict::Banned` arm below) stays at `info` —
                // that one is a state change, not a repeat.
                log::debug!("flapping: refusing {} '{}', a ban is in force", table.dim, text);
                Some(ConnectRefuse::Banned)
            }
            Verdict::Banned { count, until } => {
                if cfg.notify_sys_topic {
                    let (topic, qos, expiry) = self.ban_channel(cfg);
                    let payload =
                        notify::ban_payload(self.node_id, table.dim, &text, ci, count, now, until, policy);
                    self.notifier.publish(topic, qos, expiry, payload);
                }
                self.scx.metrics.conn_flapping_banned_inc();
                log::info!(
                    "flapping: banning {} '{}' for {} ms after {} attempts in {} ms",
                    table.dim,
                    text,
                    until.saturating_sub(now),
                    count,
                    policy.window_ms(),
                );
                Some(ConnectRefuse::Banned)
            }
        }
    }
}

#[async_trait]
impl Gate for FlappingGate {
    async fn check(&self, ci: &ConnectInfo, now: u64) -> Option<ConnectRefuse> {
        let cfg = self.cfg.read().await;
        if !cfg.enable {
            return None;
        }

        if let Some(policy) = cfg.by_clientid.as_ref() {
            let key = ci.client_id().clone();
            if let Some(refuse) = self.screen(&cfg, &self.clientid, policy, key, ci, now) {
                return Some(refuse);
            }
        }

        if let Some(policy) = cfg.by_username.as_ref() {
            if let Some(username) = ci.username() {
                let key = username.clone();
                if let Some(refuse) = self.screen(&cfg, &self.username, policy, key, ci, now) {
                    return Some(refuse);
                }
            }
        }

        if let Some(policy) = cfg.by_peerhost.as_ref() {
            if let Some(addr) = ci.id().remote_addr {
                let key = addr.ip();
                if let Some(refuse) = self.screen(&cfg, &self.peerhost, policy, key, ci, now) {
                    return Some(refuse);
                }
            }
        }

        None
    }

    fn gc_once(&self, now: u64) {
        self.clientid.gc(now);
        self.username.gc(now);
        self.peerhost.gc(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(window_ms: u64, max_count: usize, ban_ms: u64) -> Policy {
        Policy {
            window_time: Duration::from_millis(window_ms),
            max_count,
            ban_time: Duration::from_millis(ban_ms),
        }
    }

    /// Screens `key` with a generous admission bound and no recorded identity.
    fn screen_once<K: Key + Clone>(table: &KeyTable<K>, key: &K, p: &Policy, now: u64) -> Verdict {
        table.check(Arc::new(key.clone()), p, 1000, now, None, None)
    }

    fn key(text: &str) -> ByteString {
        ByteString::from(text)
    }

    #[test]
    fn the_threshold_attempt_is_the_one_refused() {
        let table: KeyTable<ByteString> = KeyTable::new(Dimension::ClientId);
        let p = policy(60_000, 3, 300_000);
        let k = key("dev-1");

        assert!(matches!(screen_once(&table, &k, &p, 1_000), Verdict::Allow));
        assert!(matches!(screen_once(&table, &k, &p, 1_100), Verdict::Allow));
        assert!(matches!(screen_once(&table, &k, &p, 1_200), Verdict::Banned { count: 3, .. }));
        assert_eq!(table.live_ban_count(1_200), 1);
    }

    #[test]
    fn attempts_outside_the_window_do_not_accumulate() {
        let table: KeyTable<ByteString> = KeyTable::new(Dimension::ClientId);
        let p = policy(1_000, 3, 300_000);
        let k = key("dev-2");

        assert!(matches!(screen_once(&table, &k, &p, 1_000), Verdict::Allow));
        // Long idle: the first attempt falls out of the window.
        assert!(matches!(screen_once(&table, &k, &p, 2_500), Verdict::Allow));
        assert!(matches!(screen_once(&table, &k, &p, 2_600), Verdict::Allow));
        // Three attempts now sit inside the window, so this one is refused.
        assert!(matches!(screen_once(&table, &k, &p, 2_700), Verdict::Banned { count: 3, .. }));
    }

    #[test]
    fn a_refused_attempt_is_not_counted_while_the_ban_is_in_force() {
        let table: KeyTable<ByteString> = KeyTable::new(Dimension::ClientId);
        let p = policy(60_000, 2, 10_000);
        let k = key("dev-3");

        assert!(matches!(screen_once(&table, &k, &p, 1_000), Verdict::Allow));
        assert!(matches!(screen_once(&table, &k, &p, 1_100), Verdict::Banned { .. }));
        assert!(matches!(screen_once(&table, &k, &p, 5_000), Verdict::Denied));
        assert_eq!(table.live_ban_count(5_000), 1);
    }

    #[test]
    fn a_lapsed_ban_hands_back_a_full_budget() {
        // `ban_time < window_time`: the case that would ban forever if the
        // window were kept when the ban was created.
        let table: KeyTable<ByteString> = KeyTable::new(Dimension::ClientId);
        let p = policy(60_000, 2, 1_000);
        let k = key("dev-4");

        assert!(matches!(screen_once(&table, &k, &p, 1_000), Verdict::Allow));
        assert!(matches!(screen_once(&table, &k, &p, 1_100), Verdict::Banned { .. }));
        assert_eq!(table.live_ban_count(1_100), 1);
        // The ban lapses 1s later; the window that produced it is gone, so this
        // attempt starts a fresh one instead of re-triggering immediately.
        assert!(matches!(screen_once(&table, &k, &p, 2_200), Verdict::Allow));
        assert_eq!(table.live_ban_count(2_200), 0);
        assert!(matches!(screen_once(&table, &k, &p, 2_300), Verdict::Banned { .. }));
    }

    #[test]
    fn unban_removes_the_ban_and_is_idempotent() {
        let table: KeyTable<ByteString> = KeyTable::new(Dimension::ClientId);
        let p = policy(60_000, 1, 300_000);
        let k = key("dev-5");

        assert!(matches!(screen_once(&table, &k, &p, 1_000), Verdict::Banned { .. }));
        let removed = table.unban(&Arc::new(k.clone())).expect("the ban must be there");
        assert_eq!(removed.count, 1);
        assert!(table.unban(&Arc::new(k.clone())).is_none());
        assert_eq!(table.live_ban_count(1_000), 0);
        assert_eq!(table.win_idx.len(), 0);
        assert_eq!(table.ban_idx.len(), 0);
    }

    #[test]
    fn a_full_table_lets_the_unknown_key_through_and_frees_room_on_the_next_pass() {
        let table: KeyTable<ByteString> = KeyTable::new(Dimension::ClientId);
        let p = policy(1_000, 5, 300_000);

        // Fill the table to its bound with distinct keys.
        for i in 0..3 {
            let k = key(&format!("full-{i}"));
            table.check(Arc::new(k), &p, 3, 1_000, None, None);
        }
        assert_eq!(table.tracked.load(Ordering::Relaxed), 3);

        // The table is full and nothing has expired: the newcomer is not
        // tracked, and — crucially — not refused either.
        let newcomer = key("newcomer");
        assert!(matches!(table.check(Arc::new(newcomer.clone()), &p, 3, 1_100, None, None), Verdict::Allow));
        assert_eq!(table.tracked.load(Ordering::Relaxed), 3);
        assert!(!table.wins.contains_key(&newcomer));

        // Once the windows have emptied, the admission pass reclaims the room.
        assert!(matches!(table.check(Arc::new(newcomer.clone()), &p, 3, 5_000, None, None), Verdict::Allow));
        assert!(table.wins.contains_key(&newcomer));
        assert_eq!(table.tracked.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn gc_empties_the_windows_and_keeps_the_ones_still_in_use() {
        let table: KeyTable<ByteString> = KeyTable::new(Dimension::ClientId);
        let p = policy(1_000, 5, 300_000);

        let idle = key("idle");
        let busy = key("busy");
        screen_once(&table, &idle, &p, 1_000);
        screen_once(&table, &busy, &p, 1_000);
        assert_eq!(table.win_idx.len(), 2);

        // The busy key keeps connecting, so its window is alive when the
        // deadline registered for it elapses.
        screen_once(&table, &busy, &p, 1_900);
        table.gc(2_100);
        assert_eq!(table.tracked.load(Ordering::Relaxed), 1);
        assert!(!table.wins.contains_key(&idle));
        assert!(table.wins.contains_key(&busy));
        // The surviving key is re-registered with the deadline it has now.
        assert_eq!(table.win_idx.len(), 1);
        assert_eq!(table.win_idx.pop_expired(2_100).len(), 0);

        table.gc(2_901);
        assert_eq!(table.tracked.load(Ordering::Relaxed), 0);
        assert_eq!(table.win_idx.len(), 0);
    }

    #[test]
    fn gc_drops_a_lapsed_ban_from_both_the_table_and_the_index() {
        let table: KeyTable<ByteString> = KeyTable::new(Dimension::ClientId);
        let p = policy(60_000, 1, 1_000);
        let k = key("dev-6");

        assert!(matches!(screen_once(&table, &k, &p, 1_000), Verdict::Banned { .. }));
        assert_eq!(table.ban_idx.len(), 1);
        assert_eq!(table.live_ban_count(1_100), 1);

        table.gc(2_001);
        assert_eq!(table.ban_idx.len(), 0);
        assert_eq!(table.live_ban_count(2_001), 0);
    }

    #[test]
    fn keys_parse_back_from_their_textual_form() {
        assert_eq!(<ByteString as Key>::parse("dev-1").unwrap().as_text(), "dev-1");
        assert_eq!(<IpAddr as Key>::parse("192.168.1.20").unwrap().as_text(), "192.168.1.20");
        assert!(<IpAddr as Key>::parse("192.168.1.20:53124").is_none());
    }

    #[test]
    fn a_window_empties_one_width_after_its_newest_attempt() {
        let mut window = Window { ts: VecDeque::new(), window_ms: 1_000, indexed: 0 };
        assert_eq!(window.empty_at(), 0);
        assert!(window.is_empty_at(0));
        window.ts.push_back(1_000);
        window.ts.push_back(1_500);
        assert_eq!(window.empty_at(), 2_500);
        assert!(!window.is_empty_at(2_499));
        assert!(window.is_empty_at(2_500));
    }
}
