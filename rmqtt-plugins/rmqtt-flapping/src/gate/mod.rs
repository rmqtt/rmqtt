//! The connection gate.
//!
//! A gate inspects a decoded CONNECT and decides whether the broker should
//! answer it with a refusal. There is exactly one today — the flapping
//! detector — but a second screening rule (a connection rate limit, a
//! subscription-cycling detector) is a natural neighbour, so the plugin is
//! shaped as a gate set rather than as the detector itself: adding one means
//! writing a [`Gate`] implementation and appending it to [`GateSet`], without
//! touching the plugin's identity, configuration file or module layout.
//!
//! [`DeadlineIndex`] is the piece that makes the set cheap to maintain. It is
//! shared infrastructure because any gate that counts over a window faces the
//! same problem: the authoritative state lives in a sharded map, and the
//! periodic cleanup must not walk it.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;

use rmqtt::types::{ConnectInfo, ConnectRefuse};

pub(crate) mod flapping;

/// Screens connection attempts.
#[async_trait]
pub(crate) trait Gate: Sync + Send {
    /// Inspects one connection attempt, `now` being the current time in
    /// milliseconds since the epoch. `Some` refuses the connection with the
    /// returned reason; `None` lets it through.
    ///
    /// The gate both decides and announces: it owns the notifications and the
    /// counters its decisions feed.
    async fn check(&self, connect_info: &ConnectInfo, now: u64) -> Option<ConnectRefuse>;

    /// Drops the state this gate no longer needs. A gate that holds only
    /// per-connection state has nothing to do.
    fn gc_once(&self, now: u64) {
        let _ = now;
    }
}

/// The gates of one plugin instance, evaluated in order.
///
/// The first gate that refuses wins and the remaining ones are not asked: a
/// refusal is terminal for the connection.
pub(crate) struct GateSet {
    gates: Vec<Box<dyn Gate>>,
}

#[async_trait]
impl<T: Gate + ?Sized> Gate for Arc<T> {
    #[inline]
    async fn check(&self, connect_info: &ConnectInfo, now: u64) -> Option<ConnectRefuse> {
        (**self).check(connect_info, now).await
    }

    #[inline]
    fn gc_once(&self, now: u64) {
        (**self).gc_once(now);
    }
}

impl GateSet {
    #[inline]
    pub(crate) fn new(gates: Vec<Box<dyn Gate>>) -> Self {
        Self { gates }
    }

    #[inline]
    pub(crate) async fn check(&self, connect_info: &ConnectInfo, now: u64) -> Option<ConnectRefuse> {
        for gate in &self.gates {
            if let Some(refuse) = gate.check(connect_info, now).await {
                return Some(refuse);
            }
        }
        None
    }

    /// Runs one cleanup pass on every gate that keeps expiring state.
    #[inline]
    pub(crate) fn gc_once(&self, now: u64) {
        for gate in &self.gates {
            gate.gc_once(now);
        }
    }
}

/// An index over a set of keys, ordered by the moment each key expires.
///
/// Removing the head is the only steady-state operation, so a sweep costs
/// O(entries that are actually due) instead of O(tracked keys): when nothing
/// is due it reads a single entry and stops. The keys themselves are shared
/// with the authoritative map through `Arc`, so an index entry costs a pointer
/// and a timestamp, never a copy of the key.
///
/// The index is *not* authoritative. It may hold a pair whose deadline moved
/// on, and — under a narrow race — a pair whose key was removed in the
/// meantime. Both are harmless: the pair is popped at its deadline, looked up
/// in the map, and either dropped or re-registered with the deadline the entry
/// has now. See `designs/flapping-plugin.md` §4.4 for the analysis.
pub(crate) struct DeadlineIndex<K> {
    deadlines: Mutex<BTreeSet<(u64, Arc<K>)>>,
}

impl<K: Ord> DeadlineIndex<K> {
    #[inline]
    pub(crate) fn new() -> Self {
        Self { deadlines: Mutex::new(BTreeSet::new()) }
    }

    /// Registers `key` to expire at `deadline`.
    #[inline]
    pub(crate) fn insert(&self, deadline: u64, key: Arc<K>) {
        self.deadlines.lock().insert((deadline, key));
    }

    /// Unregisters a pair. Removing a pair that is not registered does nothing.
    #[inline]
    pub(crate) fn remove(&self, deadline: u64, key: Arc<K>) {
        self.deadlines.lock().remove(&(deadline, key));
    }

    /// Takes every pair whose deadline has passed, and stops at the first pair
    /// that is not due yet.
    #[inline]
    pub(crate) fn pop_expired(&self, now: u64) -> Vec<(u64, Arc<K>)> {
        let mut deadlines = self.deadlines.lock();
        let mut due = Vec::new();
        while deadlines.first().is_some_and(|(deadline, _)| *deadline <= now) {
            if let Some(pair) = deadlines.pop_first() {
                due.push(pair);
            }
        }
        due
    }

    #[inline]
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.deadlines.lock().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pop_expired_takes_only_what_is_due_and_keeps_the_order() {
        let index: DeadlineIndex<u32> = DeadlineIndex::new();
        index.insert(30, Arc::new(3));
        index.insert(10, Arc::new(1));
        index.insert(20, Arc::new(2));

        let due = index.pop_expired(20);
        assert_eq!(due.iter().map(|(d, k)| (*d, **k)).collect::<Vec<_>>(), vec![(10, 1), (20, 2)]);
        assert_eq!(index.len(), 1);

        // Nothing is due any more: a sweep is a single comparison.
        assert!(index.pop_expired(20).is_empty());
        assert_eq!(index.pop_expired(30).len(), 1);
        assert_eq!(index.len(), 0);
    }

    #[test]
    fn remove_takes_the_exact_pair_only() {
        let index: DeadlineIndex<u32> = DeadlineIndex::new();
        let key = Arc::new(1);
        index.insert(10, key.clone());
        index.insert(20, key.clone());

        index.remove(10, key.clone());
        assert_eq!(index.pop_expired(20).len(), 1);
        // A pair that is not registered is a no-op, not a panic.
        index.remove(999, key);
        assert_eq!(index.len(), 0);
    }
}
