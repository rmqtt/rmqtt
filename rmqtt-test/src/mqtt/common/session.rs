//! Shared packet ID counter

use std::num::NonZeroU16;
use std::sync::atomic::{AtomicU16, Ordering};

/// Thread-safe packet ID counter that wraps around, never returning 0
pub struct PacketIdCounter {
    next: AtomicU16,
}

impl PacketIdCounter {
    pub fn new() -> Self {
        Self { next: AtomicU16::new(1) }
    }

    pub fn next(&self) -> NonZeroU16 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        NonZeroU16::new(if id == 0 { 1 } else { id }).unwrap()
    }
}

impl Default for PacketIdCounter {
    fn default() -> Self {
        Self::new()
    }
}
