use std::fmt;
use std::sync::atomic::{AtomicIsize, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::json;

type Current = AtomicIsize;
type Max = AtomicIsize;

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
    #[inline]
    pub fn new() -> Self {
        Counter(AtomicIsize::new(0), AtomicIsize::new(0), StatsMergeMode::None)
    }

    #[inline]
    pub fn new_with(c: isize, max: isize, m: StatsMergeMode) -> Self {
        Counter(AtomicIsize::new(c), AtomicIsize::new(max), m)
    }

    #[inline]
    pub fn inc(&self) {
        self.incs(1);
    }

    #[inline]
    pub fn incs(&self, c: isize) {
        let prev = self.0.fetch_add(c, Ordering::SeqCst);
        self.1.fetch_max(prev + c, Ordering::SeqCst);
    }

    #[inline]
    pub fn current_inc(&self) {
        self.current_incs(1);
    }

    #[inline]
    pub fn current_incs(&self, c: isize) {
        self.0.fetch_add(c, Ordering::SeqCst);
    }

    #[inline]
    pub fn current_set(&self, c: isize) {
        self.0.store(c, Ordering::SeqCst);
    }

    #[inline]
    pub fn sets(&self, c: isize) {
        self.current_set(c);
        self.1.fetch_max(c, Ordering::SeqCst);
    }

    #[inline]
    pub fn dec(&self) {
        self.decs(1)
    }

    #[inline]
    pub fn decs(&self, c: isize) {
        self.0.fetch_sub(c, Ordering::SeqCst);
    }

    #[inline]
    pub fn count_min(&self, count: isize) {
        self.0.fetch_min(count, Ordering::SeqCst);
    }

    #[inline]
    pub fn count_max(&self, count: isize) {
        self.0.fetch_max(count, Ordering::SeqCst);
    }

    #[inline]
    pub fn max_max(&self, max: isize) {
        self.1.fetch_max(max, Ordering::SeqCst);
    }

    #[inline]
    pub fn max_min(&self, max: isize) {
        self.1.fetch_min(max, Ordering::SeqCst);
    }

    #[inline]
    pub fn count(&self) -> isize {
        self.0.load(Ordering::SeqCst)
    }

    #[inline]
    pub fn max(&self) -> isize {
        self.1.load(Ordering::SeqCst)
    }

    #[inline]
    pub fn add(&self, other: &Self) {
        self.0.fetch_add(other.0.load(Ordering::SeqCst), Ordering::SeqCst);
        self.1.fetch_add(other.1.load(Ordering::SeqCst), Ordering::SeqCst);
    }

    #[inline]
    pub fn set(&self, other: &Self) {
        self.0.store(other.0.load(Ordering::SeqCst), Ordering::SeqCst);
        self.1.store(other.1.load(Ordering::SeqCst), Ordering::SeqCst);
    }

    #[inline]
    pub fn merge(&self, other: &Self) {
        stats_merge(&self.2, self, other);
    }

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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum StatsMergeMode {
    None,    // Represents no merging;
    Sum,     // Represents summing the data;
    Average, // Represents averaging the data;
    Max,     // Represents taking the maximum value of the data;
    Min,     // Represents taking the minimum value of the data;
}
