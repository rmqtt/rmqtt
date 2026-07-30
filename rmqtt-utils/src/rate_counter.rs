//! Lightweight rate counter for tracking cumulative throughput,
//! per-second processing speed, current in-flight count, and peak.
//!
//! Uses `AtomicU64` / `AtomicI64` fields with Relaxed ordering
//! throughout. The caller drives the sampling by calling
//! [`RateCounter::tick`] at a known interval.
//!
//! [`inc`](RateCounter::inc) / [`incs`](RateCounter::incs) increment
//! **both** the cumulative `total` **and** the current in-flight count,
//! and update the peak (`max`) via atomc `fetch_max`.
//! [`dec`](RateCounter::dec) / [`decs`](RateCounter::decs) only
//! decrement `current` — matching the pattern of a
//! producer-consumer pipeline.
//!
//! # Example
//! ```
//! use std::time::Duration;
//! use rmqtt_utils::RateCounter;
//!
//! let rc = RateCounter::new();
//!
//! // Task arrives: track throughput, in-flight, and peak
//! rc.incs(42);
//! assert_eq!(rc.total(), 42);
//! assert_eq!(rc.current(), 42);
//! assert_eq!(rc.max(), 42);
//!
//! // Higher peak
//! rc.incs(10);
//! assert_eq!(rc.max(), 52);
//!
//! // Task completes: in-flight decreases, peak unchanged
//! rc.decs(20);
//! assert_eq!(rc.current(), 32);
//! assert_eq!(rc.max(), 52);
//!
//! // Compute per-second rate over a 3 s interval
//! rc.tick(Duration::from_secs(3));
//! assert!((rc.speed() - 17.333333333333332).abs() < 1e-12);
//! ```

use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::json;

use crate::StatsMergeMode;

/// A lock-free, thread-safe rate counter with current in-flight tracking
/// and peak water mark.
///
/// Wraps five atomic values in `Arc` so the counter is [`Clone`] —
/// clones share the same underlying counters, making it easy to pass
/// into concurrent tasks (e.g. [`tokio::spawn`]).
///
/// The `speed` field stores a true **per-second rate** as an `f64`
/// (encoded via `f64::to_bits` / `f64::from_bits` in the `AtomicU64`),
/// normalised by the sampling interval passed to [`tick`](RateCounter::tick).
///
/// The `current` field tracks the current in-flight or active count.
/// [`inc`](RateCounter::inc) / [`incs`](RateCounter::incs) increment **both**
/// `total` and `current` simultaneously and update `max` to the new peak.
/// [`dec`](RateCounter::dec) / [`decs`](RateCounter::decs) only decrement
/// `current` without affecting `max`.
/// [`tick`](RateCounter::tick) does **not** affect `current` or `max`.
///
/// # Serialisation
///
/// `RateCounter` serialises as a snapshot of the **current** counter values
/// (total, speed, current, max). Deserialisation produces a fresh,
/// **independent** counter initialised to those values — it does NOT share
/// atomics with the original. This makes it safe for cross-node transfer
/// (e.g. via gRPC).
#[derive(Debug)]
pub struct RateCounter {
    /// Total cumulative count since construction (or last reset).
    total: Arc<AtomicU64>,
    /// Per-second rate, stored as `f64::to_bits` in an `AtomicU64`.
    speed: Arc<AtomicU64>,
    /// Snapshot of `total` at the time of the last [`tick`](RateCounter::tick).
    last_total: Arc<AtomicU64>,
    /// Current (in-flight / active) count.
    current: Arc<AtomicI64>,
    /// Peak (historical maximum) of the current count.
    max: Arc<AtomicI64>,
    /// Merge behaviour for [`Stats::add`] aggregation.
    mode: StatsMergeMode,
}

impl Clone for RateCounter {
    fn clone(&self) -> Self {
        Self {
            total: Arc::clone(&self.total),
            speed: Arc::clone(&self.speed),
            last_total: Arc::clone(&self.last_total),
            current: Arc::clone(&self.current),
            max: Arc::clone(&self.max),
            mode: self.mode.clone(),
        }
    }
}

impl Default for RateCounter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateCounter {
    /// Creates a new `RateCounter` with all values initialised to zero
    /// and [`StatsMergeMode::None`].
    #[inline]
    pub fn new() -> Self {
        Self::new_with_mode(StatsMergeMode::None)
    }

    /// Creates a new `RateCounter` with a given merge mode.
    #[inline]
    pub fn new_with_mode(mode: StatsMergeMode) -> Self {
        Self {
            total: Arc::new(AtomicU64::new(0)),
            speed: Arc::new(AtomicU64::new(0)),
            last_total: Arc::new(AtomicU64::new(0)),
            current: Arc::new(AtomicI64::new(0)),
            max: Arc::new(AtomicI64::new(0)),
            mode,
        }
    }

    /// Creates an independent deep copy (new atomics) with the same values.
    #[inline]
    pub fn snapshot(&self) -> Self {
        Self {
            total: Arc::new(AtomicU64::new(self.total())),
            speed: Arc::new(AtomicU64::new(f64::to_bits(self.speed()))),
            last_total: Arc::new(AtomicU64::new(self.total())),
            current: Arc::new(AtomicI64::new(self.current())),
            max: Arc::new(AtomicI64::new(self.max())),
            mode: self.mode.clone(),
        }
    }

    /// Increments total, current, and potentially updates max by 1.
    #[inline]
    pub fn inc(&self) {
        self.total.fetch_add(1, Ordering::Relaxed);
        let old = self.current.fetch_add(1, Ordering::Relaxed);
        self.max.fetch_max(old + 1, Ordering::Relaxed);
    }

    /// Increments total, current, and potentially updates max by `n`.
    #[inline]
    pub fn incs(&self, n: u64) {
        self.total.fetch_add(n, Ordering::Relaxed);
        let old = self.current.fetch_add(n as i64, Ordering::Relaxed);
        self.max.fetch_max(old + n as i64, Ordering::Relaxed);
    }

    /// Returns the cumulative total count.
    #[inline]
    pub fn total(&self) -> u64 {
        self.total.load(Ordering::Relaxed)
    }

    /// Returns the per-second rate computed by the most recent [`tick`].
    ///
    /// Returns `0.0` if [`tick`] has never been called.
    #[inline]
    pub fn speed(&self) -> f64 {
        f64::from_bits(self.speed.load(Ordering::Relaxed))
    }

    /// Computes the per-second rate: `speed = (total - last_total) / interval`.
    ///
    /// Call this periodically at a known `interval` (e.g. every 3 s) to get
    /// a true per-second throughput, regardless of the sampling duration.
    #[inline]
    pub fn tick(&self, interval: Duration) {
        let curr = self.total.load(Ordering::Relaxed);
        let prev = self.last_total.swap(curr, Ordering::Relaxed);
        let delta = curr.wrapping_sub(prev) as f64;
        let rate = delta / interval.as_secs_f64();
        self.speed.store(f64::to_bits(rate), Ordering::Relaxed);
    }

    /// Resets all counters to zero.
    #[inline]
    pub fn reset(&self) {
        self.total.store(0, Ordering::Relaxed);
        self.speed.store(f64::to_bits(0.0), Ordering::Relaxed);
        self.last_total.store(0, Ordering::Relaxed);
        self.current.store(0, Ordering::Relaxed);
        self.max.store(0, Ordering::Relaxed);
    }

    // ── current (in-flight) counter ──

    /// Returns the current (in-flight / active) count.
    #[inline]
    pub fn current(&self) -> i64 {
        self.current.load(Ordering::Relaxed)
    }

    /// Returns the peak (historical maximum) of the current count.
    #[inline]
    pub fn max(&self) -> i64 {
        self.max.load(Ordering::Relaxed)
    }

    /// Converts the rate counter to JSON format.
    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "total": self.total(),
            "speed": self.speed(),
            "current": self.current(),
            "max": self.max(),
        })
    }

    // ── Aggregation helpers (mirroring Counter) ──

    /// Sums `total`, `current` and `max` from `other` into `self`.
    #[inline]
    pub fn add(&self, other: &Self) {
        self.total.fetch_add(other.total(), Ordering::Relaxed);
        self.current.fetch_add(other.current(), Ordering::Relaxed);
        self.max.fetch_add(other.max(), Ordering::Relaxed);
    }

    /// Replaces all fields with the values from `other`.
    #[inline]
    pub fn set(&self, other: &Self) {
        self.total.store(other.total(), Ordering::Relaxed);
        self.speed.store(f64::to_bits(other.speed()), Ordering::Relaxed);
        self.last_total.store(other.total(), Ordering::Relaxed);
        self.current.store(other.current(), Ordering::Relaxed);
        self.max.store(other.max(), Ordering::Relaxed);
    }

    /// Merges `other` into `self` according to [`self.mode`](StatsMergeMode).
    ///
    /// * [`None`](StatsMergeMode::None) — no-op.
    /// * [`Sum`](StatsMergeMode::Sum) — totals are summed,
    ///   `current` / `max` take the larger value.
    /// * [`Max`](StatsMergeMode::Max) / [`Min`](StatsMergeMode::Min) —
    ///   only `current` and `max` are affected.
    #[inline]
    pub fn merge(&self, other: &Self) {
        match self.mode {
            StatsMergeMode::None => {}
            StatsMergeMode::Sum => {
                self.add(other);
            }
            StatsMergeMode::Max => {
                self.current.fetch_max(other.current(), Ordering::Relaxed);
                self.max.fetch_max(other.max(), Ordering::Relaxed);
                self.total.fetch_max(other.total(), Ordering::Relaxed);
            }
            StatsMergeMode::Min => {
                self.current.fetch_min(other.current(), Ordering::Relaxed);
                self.max.fetch_min(other.max(), Ordering::Relaxed);
                self.total.fetch_min(other.total(), Ordering::Relaxed);
            }
            _ => {}
        }
    }

    /// Decrements the current count by 1 (total and max unchanged).
    #[inline]
    pub fn dec(&self) {
        self.current.fetch_sub(1, Ordering::Relaxed);
    }

    /// Decrements the current count by `n` (total and max unchanged).
    #[inline]
    pub fn decs(&self, n: i64) {
        self.current.fetch_sub(n, Ordering::Relaxed);
    }
}

// ── Serde: snapshot-based serialisation ──

/// Helper struct carrying the fields we serialise.
#[derive(Serialize, Deserialize)]
struct RateCounterData {
    total: u64,
    speed: f64,
    current: i64,
    max: i64,
    mode: StatsMergeMode,
}

impl Serialize for RateCounter {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        RateCounterData {
            total: self.total(),
            speed: self.speed(),
            current: self.current(),
            max: self.max(),
            mode: self.mode.clone(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RateCounter {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let data = RateCounterData::deserialize(deserializer)?;
        Ok(RateCounter {
            total: Arc::new(AtomicU64::new(data.total)),
            speed: Arc::new(AtomicU64::new(f64::to_bits(data.speed))),
            last_total: Arc::new(AtomicU64::new(data.total)),
            current: Arc::new(AtomicI64::new(data.current)),
            max: Arc::new(AtomicI64::new(data.max)),
            mode: data.mode,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn new_is_zero() {
        let rc = RateCounter::new();
        assert_eq!(rc.total(), 0);
        assert_eq!(rc.speed(), 0.0);
        assert_eq!(rc.current(), 0);
        assert_eq!(rc.max(), 0);
    }

    #[test]
    fn inc_updates_current_and_max() {
        let rc = RateCounter::new();
        rc.inc();
        assert_eq!(rc.total(), 1);
        assert_eq!(rc.current(), 1);
        assert_eq!(rc.max(), 1);
    }

    #[test]
    fn incs_updates_current_and_max() {
        let rc = RateCounter::new();
        rc.incs(42);
        assert_eq!(rc.total(), 42);
        assert_eq!(rc.current(), 42);
        assert_eq!(rc.max(), 42);
    }

    #[test]
    fn max_tracks_peak() {
        let rc = RateCounter::new();
        rc.incs(10);
        assert_eq!(rc.max(), 10);

        rc.decs(5);
        assert_eq!(rc.current(), 5);
        assert_eq!(rc.max(), 10, "decrease should not lower max");

        rc.incs(20);
        assert_eq!(rc.current(), 25);
        assert_eq!(rc.max(), 25, "new higher peak should update max");
    }

    #[test]
    fn dec_does_not_affect_total_nor_max() {
        let rc = RateCounter::new();
        rc.incs(10);
        assert_eq!(rc.total(), 10);
        assert_eq!(rc.max(), 10);

        rc.dec();
        assert_eq!(rc.total(), 10, "dec should not change total");
        assert_eq!(rc.current(), 9);
        assert_eq!(rc.max(), 10, "dec should not change max");

        rc.decs(5);
        assert_eq!(rc.total(), 10, "decs should not change total");
        assert_eq!(rc.current(), 4);
        assert_eq!(rc.max(), 10, "decs should not change max");
    }

    #[test]
    fn tick_does_not_affect_current_nor_max() {
        let rc = RateCounter::new();
        rc.incs(100);
        assert_eq!(rc.current(), 100);
        assert_eq!(rc.max(), 100);

        rc.tick(Duration::from_secs(1));
        assert!((rc.speed() - 100.0).abs() < f64::EPSILON);
        assert_eq!(rc.total(), 100);
        assert_eq!(rc.current(), 100, "tick should not affect current");
        assert_eq!(rc.max(), 100, "tick should not affect max");
    }

    #[test]
    fn reset_clears_all_including_max() {
        let rc = RateCounter::new();
        rc.incs(100);
        rc.decs(20);
        rc.tick(Duration::from_secs(1));
        rc.reset();
        assert_eq!(rc.total(), 0);
        assert_eq!(rc.speed(), 0.0);
        assert_eq!(rc.current(), 0);
        assert_eq!(rc.max(), 0);

        // tick after reset should yield 0
        rc.tick(Duration::from_secs(1));
        assert_eq!(rc.speed(), 0.0);
    }

    #[test]
    fn concurrent_incs_max() {
        use std::thread;

        let rc = Arc::new(RateCounter::new());
        let mut handles = Vec::new();

        for _ in 0..8 {
            let rc = rc.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..1_000 {
                    rc.inc();
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(rc.total(), 8_000);
        assert_eq!(rc.current(), 8_000);
        // With 8 threads racing, the max should be 8_000 (all incs at once)
        // or less (some threads may have started before others completed decs
        // in other tests, but here there are no decs, so max == current)
        assert_eq!(rc.max(), 8_000);
    }

    #[test]
    fn clone_shares_max() {
        let a = RateCounter::new();
        let b = a.clone();

        a.incs(10);
        assert_eq!(b.total(), 10, "clone should see the same total");
        assert_eq!(b.current(), 10, "clone should see the same current");
        assert_eq!(b.max(), 10, "clone should see the same max");

        b.dec();
        assert_eq!(a.current(), 9, "original should see b's dec");

        a.incs(20);
        assert_eq!(b.current(), 29, "clone should see a's new current");
        assert_eq!(b.max(), 29, "clone should see a's new max");

        a.tick(Duration::from_secs(1));
        assert!((b.speed() - 30.0).abs() < f64::EPSILON, "speed computed on a should also be visible on b");
    }

    #[test]
    fn concurrent_inc_dec_pair_max() {
        use std::thread;

        let rc = Arc::new(RateCounter::new());
        let mut handles = Vec::new();

        // 8 threads each inc and dec 1_000 times → net current should be 0
        for _ in 0..8 {
            let rc = rc.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..1_000 {
                    rc.inc();
                    rc.dec();
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // total should reflect all incs
        assert_eq!(rc.total(), 8_000);
        // net current should be 0
        assert_eq!(rc.current(), 0);
        // max should be > 0 (at some point there were concurrent incs)
        assert!(rc.max() > 0, "concurrent inc/dec should have produced a peak");
    }

    #[test]
    fn to_json_includes_all_fields() {
        let rc = RateCounter::new();
        rc.incs(100);
        rc.decs(20);
        rc.tick(Duration::from_secs(5));

        let json = rc.to_json();
        let obj = json.as_object().expect("to_json should return an object");

        assert_eq!(obj["total"].as_u64(), Some(100));
        assert!((obj["speed"].as_f64().unwrap() - 20.0).abs() < f64::EPSILON);
        assert_eq!(obj["current"].as_i64(), Some(80));
        assert_eq!(obj["max"].as_i64(), Some(100));
    }
}
