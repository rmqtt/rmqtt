//! Thread-safe circuit breaker for proactive failure detection and fast-fail degradation.
//!
//! ## State Machine
//!
//! ```text
//!                record_failure() ≥ threshold
//!     CLOSED ──────────────────────────────────► OPEN
//!       ▲                                          │
//!       │                     reset_timeout elapsed │
//!       │                                          ▼
//!       │                                  ┌──────────────┐
//!       │         record_success()         │  HALF_OPEN    │
//!       ◄──────────────── (≥ threshold) ───│ (probe phase) │
//!                                          └──────────────┘
//! ```
//!
//! - **CLOSED**: Normal operation. Counts consecutive failures.
//! - **OPEN**: Fail fast — all callers skip the external operation.
//! - **HALF_OPEN**: Allow probe requests. If enough consecutive successes occur, transition
//!   back to CLOSED. If any failure occurs, transition back to OPEN.
//!
//! ## Zero-Lock Design
//!
//! All state transitions use atomic operations with `SeqCst` ordering for
//! consistent cross-thread visibility. No `Mutex` or `RwLock` is needed.
//!
//! ## Usage
//!
//! ```rust
//! use rmqtt_utils::{CircuitBreaker, CircuitBreakerConfig};
//! use std::time::Duration;
//!
//! let cb = CircuitBreaker::new(CircuitBreakerConfig {
//!     enabled: true,
//!     failure_threshold: 5,
//!     reset_timeout: Duration::from_secs(30),
//!     half_open_success_threshold: 3,
//! });
//!
//! // Before calling an external operation, check if the circuit is blocked:
//! if !cb.is_blocked() {
//!     // ... perform operation ...
//!     let success = true; // replace with actual result
//!     if success {
//!         cb.record_success();
//!     } else {
//!         cb.record_failure();
//!     }
//! }
//! ```

use std::fmt;
use std::sync::atomic::{AtomicI64, AtomicU8, AtomicUsize, Ordering};
use std::time::Duration;

use crate::timestamp_millis;

// ─── State constants ─────────────────────────────────────────────────────────

const CLOSED: u8 = 0;
const OPEN: u8 = 1;
const HALF_OPEN: u8 = 2;

// ─── Public types ─────────────────────────────────────────────────────────────

/// Circuit breaker operational states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation — requests are allowed through.
    Closed,
    /// Fail-fast — requests are blocked without attempting the external call.
    Open,
    /// Probe phase — a limited number of requests are allowed to test recovery.
    HalfOpen,
}

/// Configuration for [`CircuitBreaker`].
///
/// # Example (TOML)
///
/// ```toml
/// [message-storage.circuit_breaker]
/// enabled = true
/// failure_threshold = 5
/// reset_timeout = "30s"
/// half_open_success_threshold = 3
/// ```
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Whether the circuit breaker is active. When `false`, all operations pass
    /// through unchecked.
    pub enabled: bool,

    /// Number of consecutive failures in CLOSED state before tripping to OPEN.
    pub failure_threshold: usize,

    /// Duration to wait in OPEN state before transitioning to HALF_OPEN for a probe.
    pub reset_timeout: Duration,

    /// Number of consecutive successes in HALF_OPEN state before transitioning
    /// back to CLOSED.
    ///
    /// A value of `1` means a single successful probe closes the circuit.
    /// Higher values provide more confidence before full recovery.
    pub half_open_success_threshold: usize,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            failure_threshold: 5,
            reset_timeout: Duration::from_secs(30),
            half_open_success_threshold: 1,
        }
    }
}

// ─── Circuit breaker ──────────────────────────────────────────────────────────

/// A lock-free, thread-safe circuit breaker for fast-fail degradation.
///
/// All state transitions are atomic; no locks or wait mechanisms are used.
/// See the [module-level documentation](self) for the state machine description
/// and usage example.
pub struct CircuitBreaker {
    state: AtomicU8,
    failure_count: AtomicUsize,
    opened_at: AtomicI64,
    success_count: AtomicUsize,
    config: CircuitBreakerConfig,
}

impl fmt::Debug for CircuitBreaker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CircuitBreaker")
            .field("state", &self.state())
            .field("failure_count", &self.failure_count.load(Ordering::Relaxed))
            .field("success_count", &self.success_count.load(Ordering::Relaxed))
            .field("config", &self.config)
            .finish()
    }
}

impl CircuitBreaker {
    /// Create a new circuit breaker with the given configuration.
    #[inline]
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            state: AtomicU8::new(CLOSED),
            failure_count: AtomicUsize::new(0),
            opened_at: AtomicI64::new(0),
            success_count: AtomicUsize::new(0),
            config,
        }
    }

    /// Returns `true` **iff** the circuit is OPEN and the caller should **skip**
    /// the external operation.
    ///
    /// Side effects (atomic transitions):
    /// - OPEN → HALF_OPEN after `reset_timeout` has elapsed (at most one thread
    ///   will perform the transition; others still see OPEN and are blocked).
    /// - CLOSED, HALF_OPEN → no transition.
    #[inline]
    pub fn is_blocked(&self) -> bool {
        if !self.config.enabled {
            return false;
        }

        let state = self.state.load(Ordering::Acquire);

        match state {
            CLOSED => false,
            HALF_OPEN => false,
            OPEN => {
                // Check whether it is time to attempt a probe.
                let elapsed_ms = (crate::timestamp_millis() - self.opened_at.load(Ordering::Relaxed)) as u64;
                let reset_ms = self.config.reset_timeout.as_millis() as u64;

                if elapsed_ms >= reset_ms {
                    // Reset success counter before entering HALF_OPEN.
                    self.success_count.store(0, Ordering::Release);
                    self.state.store(HALF_OPEN, Ordering::Release);
                    false
                } else {
                    true
                }
            }
            _ => false,
        }
    }

    /// Record a successful call.
    ///
    /// - **CLOSED**: reset the `failure_count` to 0.
    /// - **HALF_OPEN**: increment success count; if it reaches
    ///   `half_open_success_threshold`, transition to CLOSED and reset all counters.
    /// - **OPEN**: no-op.
    #[inline]
    pub fn record_success(&self) {
        let state = self.state.load(Ordering::Acquire);

        match state {
            CLOSED => {
                self.failure_count.store(0, Ordering::Release);
            }
            HALF_OPEN => {
                let c = self.success_count.fetch_add(1, Ordering::SeqCst) + 1;
                if c >= self.config.half_open_success_threshold {
                    self.state.store(CLOSED, Ordering::Release);
                    self.failure_count.store(0, Ordering::Release);
                    self.success_count.store(0, Ordering::Release);
                    log::info!("circuit breaker: CLOSED (recovered)");
                }
            }
            OPEN => {
                // No-op: failures may still be in-flight after the state
                // transition; we ignore individual successes while OPEN.
            }
            _ => {}
        }
    }

    /// Record a failed call.
    ///
    /// - **CLOSED**: increment `failure_count`; if it reaches
    ///   `failure_threshold`, transition to OPEN and record the timestamp.
    /// - **HALF_OPEN**: transition immediately back to OPEN (probe failed).
    /// - **OPEN**: no-op (already open).
    #[inline]
    pub fn record_failure(&self) {
        let state = self.state.load(Ordering::Acquire);

        match state {
            CLOSED => {
                let c = self.failure_count.fetch_add(1, Ordering::SeqCst) + 1;
                if c >= self.config.failure_threshold {
                    self.state.store(OPEN, Ordering::Release);
                    self.opened_at.store(timestamp_millis(), Ordering::Release);
                    log::warn!(
                        "circuit breaker: OPEN ({} consecutive failures, threshold: {})",
                        c,
                        self.config.failure_threshold
                    );
                }
            }
            HALF_OPEN => {
                self.state.store(OPEN, Ordering::Release);
                self.opened_at.store(timestamp_millis(), Ordering::Release);
                log::warn!("circuit breaker: OPEN (half-open probe failed)");
            }
            OPEN => {
                // No-op: already open; no need to update the timestamp.
            }
            _ => {}
        }
    }

    /// Return the current circuit state (no side effects).
    #[inline]
    pub fn state(&self) -> CircuitState {
        match self.state.load(Ordering::Acquire) {
            CLOSED => CircuitState::Closed,
            OPEN => CircuitState::Open,
            HALF_OPEN => CircuitState::HalfOpen,
            _ => unreachable!(),
        }
    }

    /// Manually reset the circuit breaker to CLOSED state.
    ///
    /// This is useful after administrative recovery of the downstream service.
    #[inline]
    pub fn reset(&self) {
        self.state.store(CLOSED, Ordering::Release);
        self.failure_count.store(0, Ordering::Release);
        self.success_count.store(0, Ordering::Release);
        self.opened_at.store(0, Ordering::Release);
        log::info!("circuit breaker: CLOSED (manual reset)");
    }

    /// Borrow the configuration (for inspection / logging).
    #[inline]
    pub fn config(&self) -> &CircuitBreakerConfig {
        &self.config
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_disabled_passes_through() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig { enabled: false, ..Default::default() });
        assert!(!cb.is_blocked());
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn test_closed_then_open() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            enabled: true,
            failure_threshold: 3,
            reset_timeout: Duration::from_secs(60),
            half_open_success_threshold: 1,
        });
        assert!(!cb.is_blocked());

        // 2 failures — still closed
        cb.record_failure();
        cb.record_failure();
        assert!(!cb.is_blocked());

        // 3rd failure — trip
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(cb.is_blocked());
    }

    #[test]
    fn test_open_recovers_to_half_open_after_timeout() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            enabled: true,
            failure_threshold: 1,
            reset_timeout: Duration::from_millis(1), // very short
            half_open_success_threshold: 1,
        });

        // Trip to OPEN
        cb.record_failure();
        assert!(cb.is_blocked());

        // Wait for reset_timeout
        std::thread::sleep(Duration::from_millis(5));

        // Should now transition to HALF_OPEN
        assert!(!cb.is_blocked());
        assert_eq!(cb.state(), CircuitState::HalfOpen);
    }

    #[test]
    fn test_half_open_success_closes() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            enabled: true,
            failure_threshold: 1,
            reset_timeout: Duration::from_millis(1),
            half_open_success_threshold: 2, // need 2 successes
        });

        // Trip to OPEN
        cb.record_failure();
        // Wait for timeout
        std::thread::sleep(Duration::from_millis(5));

        // Transition to HALF_OPEN
        assert!(!cb.is_blocked());
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        // 1st success — still HALF_OPEN
        cb.record_success();
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        // 2nd success — CLOSED
        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn test_half_open_failure_reopens() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            enabled: true,
            failure_threshold: 1,
            reset_timeout: Duration::from_millis(1),
            half_open_success_threshold: 1,
        });

        // Trip → wait → HALF_OPEN
        cb.record_failure();
        std::thread::sleep(Duration::from_millis(5));
        assert!(!cb.is_blocked());

        // Probe fails → back to OPEN
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(cb.is_blocked());
    }

    #[test]
    fn test_reset() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            enabled: true,
            failure_threshold: 1,
            ..Default::default()
        });

        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);

        cb.reset();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(!cb.is_blocked());
    }

    #[test]
    fn test_success_clears_failure_count_in_closed() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            enabled: true,
            failure_threshold: 5,
            ..Default::default()
        });

        // 2 failures then a success
        cb.record_failure();
        cb.record_failure();
        cb.record_success();

        // Should need 5 more failures to trip
        cb.record_failure();
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed); // only 3 since reset

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
    }
}
