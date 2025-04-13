/// # Example: Using `Counter`
/// ```
/// use rmqtt_utils::{Counter, StatsMergeMode};
///
/// let counter = Counter::new();
/// counter.inc(); // increment by 1
/// counter.incs(5); // increment by 5
/// println!("Current: {}, Max: {}", counter.count(), counter.max());
///
/// let other = Counter::new_with(10, 20, StatsMergeMode::Max);
/// counter.merge(&other);
/// println!("After merge: {:?}", counter.to_json());
/// ```
use std::fmt;
use std::sync::atomic::{AtomicIsize, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::json;

type Current = AtomicIsize;
type Max = AtomicIsize;

/// A counter with current and maximum tracking, and optional merging behavior.
#[derive(Serialize, Deserialize)]
pub struct Counter(Current, Max, StatsMergeMode);

impl Clone for Counter {
    fn clone(&self) -> Self {
        Counter(
            AtomicIsize::new(self.0.load(Ordering::SeqCst)),
            AtomicIsize::new(self.1.load(Ordering::SeqCst)),
            self.2.clone(),
        )
    }
}

impl fmt::Debug for Counter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, r#"{{ "count":{}, "max":{} }}"#, self.count(), self.max())
    }
}

impl Default for Counter {
    fn default() -> Self {
        Self::new()
    }
}

impl Counter {
    /// Creates a new `Counter` with zeroed values and `StatsMergeMode::None`.
    ///
    /// # Example
    /// ```
    /// let c = rmqtt_utils::Counter::new();
    /// ```
    #[inline]
    pub fn new() -> Self {
        Counter(AtomicIsize::new(0), AtomicIsize::new(0), StatsMergeMode::None)
    }

    /// Creates a new `Counter` with specified values.
    ///
    /// # Example
    /// ```
    /// let c = rmqtt_utils::Counter::new_with(5, 10, rmqtt_utils::StatsMergeMode::Sum);
    /// ```
    #[inline]
    pub fn new_with(c: isize, max: isize, m: StatsMergeMode) -> Self {
        Counter(AtomicIsize::new(c), AtomicIsize::new(max), m)
    }

    /// Increments current count by 1 and updates max if needed.
    ///
    /// # Example
    /// ```
    /// let c = rmqtt_utils::Counter::new();
    /// c.inc();
    /// ```
    #[inline]
    pub fn inc(&self) {
        self.incs(1);
    }

    /// Increments current count by `c` and updates max if needed.
    ///
    /// # Example
    /// ```
    /// let c = rmqtt_utils::Counter::new();
    /// c.incs(5);
    /// ```
    #[inline]
    pub fn incs(&self, c: isize) {
        let prev = self.0.fetch_add(c, Ordering::SeqCst);
        self.1.fetch_max(prev + c, Ordering::SeqCst);
    }

    /// Increments current count by 1 without affecting max.
    ///
    /// # Example
    /// ```
    /// let c = rmqtt_utils::Counter::new();
    /// c.current_inc();
    /// ```
    #[inline]
    pub fn current_inc(&self) {
        self.current_incs(1);
    }

    /// Increments current count by `c` without affecting max.
    ///
    /// # Example
    /// ```
    /// let c = rmqtt_utils::Counter::new();
    /// c.current_incs(3);
    /// ```
    #[inline]
    pub fn current_incs(&self, c: isize) {
        self.0.fetch_add(c, Ordering::SeqCst);
    }

    /// Sets the current count directly, does not affect max.
    ///
    /// # Example
    /// ```
    /// let c = rmqtt_utils::Counter::new();
    /// c.current_set(100);
    /// ```
    #[inline]
    pub fn current_set(&self, c: isize) {
        self.0.store(c, Ordering::SeqCst);
    }

    /// Sets the current count and possibly updates the max.
    ///
    /// # Example
    /// ```
    /// let c = rmqtt_utils::Counter::new();
    /// c.sets(20);
    /// ```
    #[inline]
    pub fn sets(&self, c: isize) {
        self.current_set(c);
        self.1.fetch_max(c, Ordering::SeqCst);
    }

    /// Decrements current count by 1.
    ///
    /// # Example
    /// ```
    /// let c = rmqtt_utils::Counter::new();
    /// c.dec();
    /// ```
    #[inline]
    pub fn dec(&self) {
        self.decs(1)
    }

    /// Decrements current count by `c`.
    ///
    /// # Example
    /// ```
    /// let c = rmqtt_utils::Counter::new();
    /// c.decs(4);
    /// ```
    #[inline]
    pub fn decs(&self, c: isize) {
        self.0.fetch_sub(c, Ordering::SeqCst);
    }

    /// Sets current count to the minimum of its current value and `count`.
    ///
    /// # Example
    /// ```
    /// let c = rmqtt_utils::Counter::new();
    /// c.count_min(5);
    /// ```
    #[inline]
    pub fn count_min(&self, count: isize) {
        self.0.fetch_min(count, Ordering::SeqCst);
    }

    /// Sets current count to the maximum of its current value and `count`.
    ///
    /// # Example
    /// ```
    /// let c = rmqtt_utils::Counter::new();
    /// c.count_max(10);
    /// ```
    #[inline]
    pub fn count_max(&self, count: isize) {
        self.0.fetch_max(count, Ordering::SeqCst);
    }

    /// Sets max to the maximum of its current value and `max`.
    ///
    /// # Example
    /// ```
    /// let c = rmqtt_utils::Counter::new();
    /// c.max_max(50);
    /// ```
    #[inline]
    pub fn max_max(&self, max: isize) {
        self.1.fetch_max(max, Ordering::SeqCst);
    }

    /// Sets max to the minimum of its current value and `max`.
    ///
    /// # Example
    /// ```
    /// let c = rmqtt_utils::Counter::new();
    /// c.max_min(30);
    /// ```
    #[inline]
    pub fn max_min(&self, max: isize) {
        self.1.fetch_min(max, Ordering::SeqCst);
    }

    /// Returns the current count value.
    ///
    /// # Example
    /// ```
    /// let c = rmqtt_utils::Counter::new();
    /// let v = c.count();
    /// ```
    #[inline]
    pub fn count(&self) -> isize {
        self.0.load(Ordering::SeqCst)
    }

    /// Returns the current max value.
    ///
    /// # Example
    /// ```
    /// let c = rmqtt_utils::Counter::new();
    /// let m = c.max();
    /// ```
    #[inline]
    pub fn max(&self) -> isize {
        self.1.load(Ordering::SeqCst)
    }

    /// Adds values from another counter.
    ///
    /// # Example
    /// ```
    /// let a = rmqtt_utils::Counter::new_with(3, 10, rmqtt_utils::StatsMergeMode::None);
    /// let b = rmqtt_utils::Counter::new_with(2, 5, rmqtt_utils::StatsMergeMode::None);
    /// a.add(&b);
    /// ```
    #[inline]
    pub fn add(&self, other: &Self) {
        self.0.fetch_add(other.0.load(Ordering::SeqCst), Ordering::SeqCst);
        self.1.fetch_add(other.1.load(Ordering::SeqCst), Ordering::SeqCst);
    }

    /// Replaces internal values with those from another counter.
    ///
    /// # Example
    /// ```
    /// let a = rmqtt_utils::Counter::new();
    /// let b = rmqtt_utils::Counter::new_with(5, 100, rmqtt_utils::StatsMergeMode::None);
    /// a.set(&b);
    /// ```
    #[inline]
    pub fn set(&self, other: &Self) {
        self.0.store(other.0.load(Ordering::SeqCst), Ordering::SeqCst);
        self.1.store(other.1.load(Ordering::SeqCst), Ordering::SeqCst);
    }

    /// Merges another counter into this one using the configured merge mode.
    ///
    /// # Example
    /// ```
    /// let a = rmqtt_utils::Counter::new_with(1, 2, rmqtt_utils::StatsMergeMode::Sum);
    /// let b = rmqtt_utils::Counter::new_with(3, 4, rmqtt_utils::StatsMergeMode::Sum);
    /// a.merge(&b);
    /// ```
    #[inline]
    pub fn merge(&self, other: &Self) {
        stats_merge(&self.2, self, other);
    }

    /// Converts the counter to JSON format.
    ///
    /// # Example
    /// ```
    /// let c = rmqtt_utils::Counter::new();
    /// let json = c.to_json();
    /// ```
    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "count": self.count(),
            "max": self.max()
        })
    }
}

#[inline]
fn stats_merge<'a>(mode: &StatsMergeMode, c: &'a Counter, o: &Counter) -> &'a Counter {
    match mode {
        StatsMergeMode::None => {}
        StatsMergeMode::Sum => {
            c.add(o);
        }
        StatsMergeMode::Max => {
            c.count_max(o.count());
            c.max_max(o.max());
        }
        StatsMergeMode::Min => {
            c.count_min(o.count());
            c.max_min(o.max());
        }
        _ => {
            log::info!("unimplemented!");
        }
    }
    c
}

/// Merge behavior modes for `Counter`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum StatsMergeMode {
    None,    // Represents no merging;
    Sum,     // Represents summing the data;
    Average, // Represents averaging the data;
    Max,     // Represents taking the maximum value of the data;
    Min,     // Represents taking the minimum value of the data;
}
