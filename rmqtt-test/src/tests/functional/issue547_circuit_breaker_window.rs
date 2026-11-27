//! Issue #547 — the default `TimeBased` circuit-breaker window on the gRPC
//! forward path costs a **full window scan per call**, under a single mutex.
//!
//! # The report
//!
//! A three-node `rmqtt-cluster-raft` deployment at 12 500 msg/s cluster-wide
//! (QoS 2, 150 KiB payloads, a fixed ring so every message crosses one broker)
//! never completes its 7.5 M messages under the default circuit-breaker
//! configuration: the publish/receive rate decays, the backlog grows and a
//! broker OOMs at 32 GiB. Switching only `circuit_breaker.sliding_window_type`
//! to `"CountBased"` makes the same load pass at 12 491 msg/s with a 446 MiB
//! peak. The reporter's own microbenchmark blamed
//! `tower-resilience-circuitbreaker 0.10.1`, the version rmqtt pins.
//!
//! # What the dependency actually does (verified in the pinned source)
//!
//! * `Circuit::call_records` is an **unbounded** `VecDeque<CallRecord>`
//!   (`circuit.rs`); `cleanup_old_records` only evicts by
//!   `sliding_window_duration`, so the window holds **one record per call made
//!   in the last 45 s** — `sliding_window_size` is not a cap. (`rmqtt.toml`
//!   claims the opposite: "For TimeBased, this value sets the max tracked
//!   calls". In the dependency it is only the fallback for
//!   `permitted_calls_in_half_open`, so the operator-facing comment is wrong.)
//! * Every call ends in `record_success` / `record_failure` →
//!   `evaluate_window` → `time_based_stats`, which **walks the whole window**
//!   (`for record in &self.call_records`) to recompute total / failures /
//!   slow — on every single call, while holding
//!   `Arc<tokio::sync::Mutex<Circuit>>` (`lib.rs`, `call()`).
//! * `poll_ready` takes the same mutex with `try_lock`; when it fails it
//!   returns `Pending` behind a 1 ms sleep, so contention turns into latency
//!   on top of the scan.
//! * `GrpcClient` (`rmqtt/src/grpc.rs`) wraps the *mailbox push* — a nearly
//!   free operation — in exactly this layer, so the breaker's cost is the
//!   forward path's cost. Both `send_message` and `notify` are covered, which
//!   is why the reporter saw the same behaviour after switching between them.
//!
//! Measured on the pinned 0.10.1 with the shipped defaults (Windows, release,
//! 8 workers, one noop service behind the layer — i.e. the same shape as
//! `GrpcNotifyService`):
//!
//! | records in window | TimeBased (45 s) | CountBased (20) |
//! |---|---|---|
//! | 5 000 | 1.45 µs/call | 0.37 µs/call |
//! | 20 000 | 5.74 µs/call | 0.37 µs/call |
//! | 50 000 | 15.41 µs/call | 0.37 µs/call |
//! | 100 000 | 33.30 µs/call | 0.37 µs/call |
//! | 200 000 | 98.63 µs/call | 0.37 µs/call |
//!
//! 16 concurrent callers on one breaker: 60 k calls/s in the first second,
//! 13.5 k at 5 s, 7.1 k at 10 s, ~6 k once the window saturates — against a
//! flat 1.3 M calls/s for `CountBased`. That is the whole incident: the
//! scheme is self-limiting, because the steady-state rate solves
//! `rate = 1 / (a · duration · rate)` (a ≈ 3.3e-10 s per record), i.e.
//! `rate ≈ 1 / √(a · duration)` — about **8 k calls/s per gRPC link** at the
//! default 45 s window, no matter how fast the link or the machine is. The
//! reporter asks for 12 500 msg/s on a path that crosses one broker, so the
//! 45 s window would hold ~187 500 records and the forward path can never
//! keep up; the two-broker ring forwards 4 166 msg/s per link, which is why
//! `CountBased` (flat 0.15 µs) passes and `TimeBased` does not.
//!
//! Upstream has not fixed it: `main` (0.11 – 0.14) still keeps per-call
//! records and still rescans them in `time_based_stats`. Upgrading the
//! dependency is therefore **not** a fix; the choices are a different default
//! window, a bucketed/incremental breaker of our own, or replacing the layer.
//!
//! # Why the reproduction is in-process, not a cluster run
//!
//! The decay is driven by *records in the window*, so it becomes observable
//! after roughly `1 / (a · rate²)` seconds of load: ~120 s at 5 000 msg/s,
//! ~30 s at 10 000, ~8 s at 20 000. A single developer machine cannot offer
//! 12 500 msg/s (let alone per path, with 150 KiB QoS 2 payloads), and any
//! e2e attempt would report "no decay" — which is indistinguishable from
//! "the defect is gone". The cases below therefore drive the **production
//! layer** directly, where the call rate is controlled and the effect is
//! unambiguous, and they measure **ratios inside one run**, so a slow or
//! loaded machine cannot flip the verdict.
//!
//! # The four cases — one arm each, so the report reads as an A/B
//!
//! Both window types get the **same** load shape; only the window type differs.
//! The `TimeBased` pair reproduces the defect (declared `Normal`, so it FAILS
//! until the breaker stops rescanning its window); the `CountBased` pair is the
//! positive control and must stay green — a red control means the harness, the
//! dependency or the machine changed, and is itself the signal.
//!
//! | Case | Arm | Load | Assertion |
//! |---|---|---|---|
//! | `issue547_timebased_window_scan_cost` | TimeBased 45 s | sequential, warm-up + 10 k → 135 k records | cross-arm cost at that occupancy must stay < 20× (secondary: growth < 5×) |
//! | `issue547_countbased_window_scan_cost` | CountBased 20 | **same** sequential shape | per-call cost must not grow > 2× |
//! | `issue547_timebased_concurrent_ceiling` | TimeBased 45 s | 16 callers, 20 s | throughput must stay ≥ 0.5× of the early rate |
//! | `issue547_countbased_concurrent_ceiling` | CountBased 20 | 16 callers, 5 s | throughput must stay ≥ 0.6× of the early rate |
//!
//! A `TimeBased` case additionally measures a tiny `CountBased` reference block
//! (its cost does not depend on occupancy, so 20 k calls are enough) and reports
//! the **cross-arm ratio** at the same occupancy — the one number a machine that
//! slowed down mid-run cannot distort. That reference block is also the guard
//! against attributing a machine hiccup to the window type: the case reports an
//! `Error`, never a reproduction, when the reference itself degraded by more
//! than 5 µs/call.
//!
//! One caveat about the thresholds: the growth ratio is smaller in a debug
//! build than in the table above (the fixed per-call cost is larger, so the scan
//! is a smaller share), and the absolute µs are larger. Measured in `dev` on the
//! same machine: 52 µs/call at 20 000 records → 450 µs/call at 200 000, growth
//! 8.7× (release: 5.74 → 98.63, 17×). The 5× ceiling clears both builds, and
//! every case reports the numbers it actually measured.
//!
//! # Running
//!
//! The suite is deliberately outside the default full run — a timing case
//! must not share the machine with the functional suites' parallel workers:
//!
//! ```text
//! cargo build -p rmqtt-test
//! ./target/debug/mqtt_harness --workspace . --no-broker --suites issue547 --workers 1
//! ```
//!
//! No broker is spawned and no port is bound. `-t issue547_timebased` runs the
//! two reproducing cases, `-t issue547_countbased` the two controls, and
//! `-t issue547_countbased_window` a single one.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tower::{Layer, Service, ServiceExt};
use tower_resilience_circuitbreaker::{
    CircuitBreaker, CircuitBreakerLayer, DefaultClassifier, SlidingWindowType,
};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Suite name; excluded from the default full run (see module header).
const SUITE: &str = "issue547";

// ---------------------------------------------------------------------------
// The shipped defaults, mirrored
// ---------------------------------------------------------------------------
//
// `rmqtt-conf` defaults (rmqtt-conf/src/lib.rs, `impl Default for
// CircuitBreaker`) mapped into `CircuitBreakerConfig` by
// `rmqtt-bin/src/server.rs` and into the Tower layer by
// `build_circuit_breaker_layer` (rmqtt/src/grpc.rs) — reproduced here field by
// field. [`shipped_defaults_conflict`] fails the cases if the broker's side
// moves, so this copy cannot silently drift away from what ships.

const FAILURE_RATE_THRESHOLD: f64 = 0.35;
const SLIDING_WINDOW_SIZE: usize = 20;
const SLIDING_WINDOW_DURATION: Duration = Duration::from_secs(45);
const MINIMUM_NUMBER_OF_CALLS: usize = 10;
const WAIT_DURATION_IN_OPEN: Duration = Duration::from_secs(30);
const SLOW_CALL_DURATION_THRESHOLD: Duration = Duration::from_secs(2);
const SLOW_CALL_RATE_THRESHOLD: f64 = 1.0;

// ---------------------------------------------------------------------------
// Load shape and verdict thresholds
// ---------------------------------------------------------------------------

/// Discarded block before the first measurement: it warms the layer's code
/// paths and the allocator, so the control arm does not report a "growth" that
/// is only cold-start (a debug build otherwise starts at 3 us/call and settles
/// near 1 us — a 3x ratio at a sub-microsecond cost, i.e. noise). Small enough
/// not to matter for the 150 000-record window the TimeBased arm needs.
const WARMUP_CALLS: usize = 5_000;
/// Sequential arm: the block whose per-call cost is the "small window"
/// baseline. Kept small because it is not the interesting part.
const FIRST_BLOCK_CALLS: usize = 10_000;
/// Ramp between the two measured blocks; it is what fills the window.
const RAMP_CALLS: usize = 120_000;
/// Block measured at the far end of the ramp.
const FINAL_BLOCK_CALLS: usize = 20_000;
/// Records in the window when the second block starts (warm-up + first block +
/// ramp = 135 000 — the same order of magnitude as the ~187 500 records a
/// 4 166 msg/s forward link accumulates in the default 45 s window). The ramp
/// dominates the runtime, which is why it is not taken all the way to 200 000:
/// in a debug build every TimeBased ramp call pays the unoptimised scan.
const WINDOW_RECORDS: usize = WARMUP_CALLS + FIRST_BLOCK_CALLS + RAMP_CALLS;

/// TimeBased: per-call cost with a `WINDOW_RECORDS`-record window may not
/// exceed this multiple of the cost at `FIRST_BLOCK_CALLS` records. A healthy
/// breaker stays at ~1× (the window is not on the call path); the pinned
/// dependency measures 17 – 30×.
const SCAN_GROWTH_CEILING: f64 = 5.0;
/// CountBased (positive control): running counters only, so the same load shape
/// must stay flat. Anything above this means the control itself changed.
const CONTROL_GROWTH_CEILING: f64 = 2.0;
/// A ratio between two sub-microsecond blocks is noise, not a measurement: in a
/// release build the control arm costs ~0.35 µs/call, where cache state, CPU
/// boost and a neighbouring process move the ratio past 2× without the breaker
/// doing anything different. The reference is only considered degraded when its
/// absolute per-call cost also grew by more than this.
const NOISE_FLOOR_US: f64 = 5.0;
/// Cross-arm ceiling: at the far-end occupancy a TimeBased call may cost at most
/// this much more than a CountBased one. Measured: 157 – 290× release, 455×
/// debug. Both values come from the same occupancy, so a machine that slowed
/// down mid-run scales them together.
const CROSS_ARM_CEILING: f64 = 20.0;

/// Concurrent callers driven against one breaker (one `GrpcClient` per peer
/// node is what the broker's forward path contends on).
const CALLERS: usize = 16;
/// Early rate is sampled once the callers have started, before the window has
/// meaningfully filled.
const EARLY_SAMPLE_AT: Duration = Duration::from_millis(500);
const SAMPLE_WINDOW: Duration = Duration::from_secs(1);
/// Late rate for a `TimeBased` arm: 20 s of load at these rates puts hundreds of
/// thousands of records in a 45 s window, well before it starts sliding (so this
/// measures accumulation, not eviction).
const TIME_BASED_LATE_SAMPLE_AT: Duration = Duration::from_secs(20);
/// The `CountBased` arm is flat by construction — 5 s is plenty.
const COUNT_BASED_LATE_SAMPLE_AT: Duration = Duration::from_secs(5);
/// Throughput at a saturated TimeBased window must keep at least this share of
/// the early rate. Measured today: 0.15.
const DECAY_FLOOR: f64 = 0.5;
/// The CountBased control must keep this share of its early rate. Deliberately
/// generous: in a debug build the 16 spinning callers plus the sampler share
/// few workers and the late sample lands around 0.85 even though nothing in the
/// breaker changed — the criterion only has to separate "no collapse" from the
/// TimeBased arm's 0.15 – 0.25.
const CONTROL_DECAY_FLOOR: f64 = 0.6;
/// Below this early rate a sample says nothing about the breaker (the runtime
/// never got going), so the case reports an Error instead of a verdict.
const MIN_EARLY_RATE_TIME_BASED: f64 = 2_000.0;
const MIN_EARLY_RATE_COUNT_BASED: f64 = 50_000.0;

// ---------------------------------------------------------------------------
// The service under the breaker
// ---------------------------------------------------------------------------

/// Stand-in for `GrpcNotifyService` (`rmqtt/src/grpc.rs`): `poll_ready` is
/// always `Ready` and `call` resolves immediately, so what the measurement
/// sees is the breaker's own overhead, not the wrapped work. This is faithful:
/// the wrapped work on the forward path is a mailbox push.
#[derive(Clone)]
struct NoopService;

impl Service<()> for NoopService {
    type Response = ();
    type Error = anyhow::Error;
    type Future = std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), anyhow::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, _req: ()) -> Self::Future {
        Box::pin(async { Ok(()) })
    }
}

/// Which window shape the breaker under test uses — the only difference between
/// the four cases.
#[derive(Clone, Copy, PartialEq, Eq)]
enum WindowShape {
    TimeBased,
    CountBased,
}

impl WindowShape {
    fn label(self) -> &'static str {
        match self {
            Self::TimeBased => "TimeBased 45s",
            Self::CountBased => "CountBased 20",
        }
    }

    fn window_clause(self) -> &'static str {
        match self {
            Self::TimeBased => {
                "`sliding_window_duration = 45s`, which keeps one `CallRecord` per call in the last 45s"
            }
            Self::CountBased => "`sliding_window_size = 20`, which only updates running counters",
        }
    }
}

/// Builds the same Tower layer `GrpcClient::new` builds
/// (`build_circuit_breaker_layer`), field for field.
fn breaker(shape: WindowShape) -> CircuitBreaker<NoopService, DefaultClassifier> {
    let mut builder = CircuitBreakerLayer::builder()
        .failure_rate_threshold(FAILURE_RATE_THRESHOLD)
        .minimum_number_of_calls(MINIMUM_NUMBER_OF_CALLS)
        .wait_duration_in_open(WAIT_DURATION_IN_OPEN)
        .slow_call_duration_threshold(SLOW_CALL_DURATION_THRESHOLD)
        .slow_call_rate_threshold(SLOW_CALL_RATE_THRESHOLD)
        .name("grpc")
        .sliding_window_type(match shape {
            WindowShape::TimeBased => SlidingWindowType::TimeBased,
            WindowShape::CountBased => SlidingWindowType::CountBased,
        })
        .sliding_window_size(SLIDING_WINDOW_SIZE);

    if shape == WindowShape::TimeBased {
        builder = builder.sliding_window_duration(SLIDING_WINDOW_DURATION);
    }

    builder.build().layer(NoopService)
}

/// Compares this module's mirrored defaults with `rmqtt-conf`'s, so a change to
/// the shipped configuration shows up as an Error instead of as a reproduction
/// of a configuration that no longer exists.
fn shipped_defaults_conflict() -> Option<String> {
    let d = rmqtt_conf::CircuitBreaker::default();
    let mut diffs = Vec::new();
    if d.sliding_window_type.as_str() != "TimeBased" {
        diffs.push(format!(
            "sliding_window_type: shipped {:?}, mirrored \"TimeBased\"",
            d.sliding_window_type
        ));
    }
    if d.sliding_window_size != SLIDING_WINDOW_SIZE {
        diffs.push(format!(
            "sliding_window_size: shipped {}, mirrored {SLIDING_WINDOW_SIZE}",
            d.sliding_window_size
        ));
    }
    if d.sliding_window_duration != SLIDING_WINDOW_DURATION {
        diffs.push(format!(
            "sliding_window_duration: shipped {:?}, mirrored {:?}",
            d.sliding_window_duration, SLIDING_WINDOW_DURATION
        ));
    }
    if d.minimum_number_of_calls != MINIMUM_NUMBER_OF_CALLS {
        diffs.push(format!(
            "minimum_number_of_calls: shipped {}, mirrored {MINIMUM_NUMBER_OF_CALLS}",
            d.minimum_number_of_calls
        ));
    }
    if d.wait_duration_in_open != WAIT_DURATION_IN_OPEN {
        diffs.push(format!(
            "wait_duration_in_open: shipped {:?}, mirrored {:?}",
            d.wait_duration_in_open, WAIT_DURATION_IN_OPEN
        ));
    }
    if d.slow_call_duration_threshold != SLOW_CALL_DURATION_THRESHOLD {
        diffs.push(format!(
            "slow_call_duration_threshold: shipped {:?}, mirrored {:?}",
            d.slow_call_duration_threshold, SLOW_CALL_DURATION_THRESHOLD
        ));
    }
    if (d.failure_rate_threshold - FAILURE_RATE_THRESHOLD).abs() > 1e-9 {
        diffs.push(format!(
            "failure_rate_threshold: shipped {}, mirrored {FAILURE_RATE_THRESHOLD}",
            d.failure_rate_threshold
        ));
    }
    if (d.slow_call_rate_threshold - SLOW_CALL_RATE_THRESHOLD).abs() > 1e-9 {
        diffs.push(format!(
            "slow_call_rate_threshold: shipped {}, mirrored {SLOW_CALL_RATE_THRESHOLD}",
            d.slow_call_rate_threshold
        ));
    }
    (!diffs.is_empty()).then(|| {
        format!(
            "the circuit-breaker defaults shipped by `rmqtt-conf` changed, so these cases no longer \
             measure the shipped TimeBased window: {}. Update the constants in this module (and the \
             issue #547 analysis) to the new defaults. Also note `sliding_window_size` has no upper-bound \
             effect on the TimeBased window in the dependency — it is not a record cap.",
            diffs.join("; ")
        )
    })
}

// ---------------------------------------------------------------------------
// Measurement primitives
// ---------------------------------------------------------------------------

/// Runs `calls` sequential calls through the breaker and reports the elapsed
/// time plus how many calls the breaker rejected (a rejection means the
/// circuit opened, which would make the timing meaningless).
async fn drive(svc: &CircuitBreaker<NoopService, DefaultClassifier>, calls: usize) -> (Duration, usize) {
    let start = Instant::now();
    let mut rejected = 0usize;
    for _ in 0..calls {
        // `ready()` never fails for this layer and NoopService cannot fail, so
        // the only way `call()` errors is the breaker refusing (OpenCircuit).
        let outcome =
            svc.clone().ready().await.expect("the circuit-breaker layer is always ready").call(()).await;
        if outcome.is_err() {
            rejected += 1;
        }
    }
    (start.elapsed(), rejected)
}

/// One sequential measurement: per-call cost over the first block, per-call cost
/// over the final block, and the growth between them.
struct SequentialArm {
    first_us: f64,
    late_us: f64,
    growth: f64,
}

impl SequentialArm {
    fn describe(&self, shape: WindowShape) -> String {
        format!(
            "{}: {:.2} us/call over the first {FIRST_BLOCK_CALLS} -> {:.2} us/call over the last \
             {FINAL_BLOCK_CALLS} ({WINDOW_RECORDS} records in the window), growth {:.1}x",
            shape.label(),
            self.first_us,
            self.late_us,
            self.growth
        )
    }
}

/// Drives one arm through the full sequential load shape on a fresh breaker.
///
/// `Err` carries an `Error`-class reason (not a verdict): the breaker refused
/// calls, which means it opened and the timings are not about the window scan.
fn sequential_arm(rt: &tokio::runtime::Runtime, shape: WindowShape) -> Result<SequentialArm, String> {
    let svc = breaker(shape);
    let total = FIRST_BLOCK_CALLS + RAMP_CALLS + FINAL_BLOCK_CALLS;

    let elapsed = |calls: usize, block: &str| -> Result<Duration, String> {
        let (took, rejected) = rt.block_on(drive(&svc, calls));
        if rejected > 0 {
            return Err(format!(
                "the {} arm had {rejected} calls refused by the breaker (circuit Open) during the \
                 {block} block of {calls} calls, so its timings are not a measurement of the window \
                 scan. This case needs an unloaded machine; rerun with --workers 1.",
                shape.label()
            ));
        }
        Ok(took)
    };

    let _warmup = elapsed(WARMUP_CALLS, "warm-up")?;
    let first = elapsed(FIRST_BLOCK_CALLS, "first")?;
    let _ramp = elapsed(RAMP_CALLS, "ramp")?;
    let last = elapsed(FINAL_BLOCK_CALLS, "final")?;

    let first_us = first.as_secs_f64() * 1e6 / FIRST_BLOCK_CALLS as f64;
    let late_us = last.as_secs_f64() * 1e6 / FINAL_BLOCK_CALLS as f64;
    tracing::debug!("{total} calls measured on {}", shape.label());
    Ok(SequentialArm { first_us, late_us, growth: late_us / first_us.max(f64::MIN_POSITIVE) })
}

/// A cheap `CountBased` reference: two 10 000-call blocks and **no ramp**, since
/// its cost does not depend on how many records the window holds. Used by the
/// `TimeBased` cases for the cross-arm ratio and as the stability guard.
struct Reference {
    first_us: f64,
    second_us: f64,
}

fn count_based_reference(rt: &tokio::runtime::Runtime) -> Result<Reference, String> {
    let svc = breaker(WindowShape::CountBased);
    let block = 10_000usize;
    let measure = || -> Result<f64, String> {
        let (took, rejected) = rt.block_on(drive(&svc, block));
        if rejected > 0 {
            return Err(format!(
                "the CountBased reference block had {rejected} calls refused by the breaker (circuit \
                 Open), so it cannot serve as the {WINDOW_RECORDS}-record baseline."
            ));
        }
        Ok(took.as_secs_f64() * 1e6 / block as f64)
    };
    let first_us = measure()?;
    let second_us = measure()?;
    Ok(Reference { first_us, second_us })
}

impl Reference {
    fn delta_us(&self) -> f64 {
        self.second_us - self.first_us
    }

    fn growth(&self) -> f64 {
        self.second_us / self.first_us.max(f64::MIN_POSITIVE)
    }

    /// `Some(reason)` when the reference itself moved enough to make the run
    /// unattributable — a ratio at ~0.5 µs/call is noise unless the absolute
    /// cost moved too.
    fn degradation(&self) -> Option<String> {
        (self.growth() > CONTROL_GROWTH_CEILING && self.delta_us() > NOISE_FLOOR_US).then(|| {
            format!(
                "the CountBased reference degraded {:.1}x on its own ({:.2} -> {:.2} us/call, +{:.1} us, \
                 floor {NOISE_FLOOR_US:.0} us per call), so this run cannot attribute a TimeBased slowdown \
                 to the window type — the machine was not stable",
                self.growth(),
                self.first_us,
                self.second_us,
                self.delta_us()
            )
        })
    }
}

/// Concurrent arm: spawns [`CALLERS`] tasks that call the breaker in a loop and
/// samples the completed-call rate once early and once late.
///
/// Times are relative to the start of the arm. Returns `(early, late, rejected)`
/// where `late` is the **higher** of two consecutive one-second samples: an
/// outside disturbance can only push a sample down, so taking the better of two
/// keeps the controls green without weakening the decay criterion (the TimeBased
/// arm measures 0.15 even at its best late sample).
fn concurrent_rates(rt: &tokio::runtime::Runtime, shape: WindowShape, late_at: Duration) -> (f64, f64, u64) {
    let calls = Arc::new(AtomicU64::new(0));
    let rejected = Arc::new(AtomicU64::new(0));

    rt.block_on(async move {
        let t0 = Instant::now();
        // ONE breaker, cloned into every task: the clones share the same
        // `Arc<Mutex<Circuit>>`, which is exactly the contention the broker's
        // forward path has on a single peer link.
        let svc = breaker(shape);
        let mut tasks = Vec::with_capacity(CALLERS);
        for _ in 0..CALLERS {
            let svc = svc.clone();
            let calls = calls.clone();
            let rejected = rejected.clone();
            tasks.push(tokio::spawn(async move {
                loop {
                    let outcome = svc.clone().ready().await.expect("always ready").call(()).await;
                    if outcome.is_err() {
                        rejected.fetch_add(1, Ordering::Relaxed);
                        tokio::task::yield_now().await;
                        continue;
                    }
                    calls.fetch_add(1, Ordering::Relaxed);
                }
            }));
        }

        let early = sample_rate(&calls, t0, EARLY_SAMPLE_AT).await;
        let late = sample_rate(&calls, t0, late_at).await;
        let late_again = sample_rate(&calls, t0, late_at + SAMPLE_WINDOW).await;

        for t in tasks {
            t.abort();
            let _ = t.await;
        }
        (early, late.max(late_again), rejected.load(Ordering::Relaxed))
    })
}

/// Waits until `at` (measured from `t0`) and returns the call rate over the next
/// [`SAMPLE_WINDOW`].
async fn sample_rate(calls: &Arc<AtomicU64>, t0: Instant, at: Duration) -> f64 {
    let now = t0.elapsed();
    if at > now {
        tokio::time::sleep(at - now).await;
    }
    let before = calls.load(Ordering::Relaxed);
    tokio::time::sleep(SAMPLE_WINDOW).await;
    (calls.load(Ordering::Relaxed).saturating_sub(before)) as f64 / SAMPLE_WINDOW.as_secs_f64()
}

fn runtime(workers: usize) -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
        .expect("failed to build the measurement runtime")
}

/// Worker threads for a concurrent arm: the callers are yield-heavy, so a couple
/// of them plus a scheduler slot is enough. Clamped so a container with a low
/// `nproc` still gets a usable runtime.
fn worker_count() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 8)
}

// ---------------------------------------------------------------------------
// Shared verdict pieces
// ---------------------------------------------------------------------------

/// The occluded-defect wording shared by the two reproducing cases.
fn defect_note() -> &'static str {
    "the default TimeBased circuit-breaker window puts an O(n) scan on every gRPC forward call \
     (tower-resilience-circuitbreaker 0.10.1 `time_based_stats()` walks the whole \
     `VecDeque<CallRecord>` under the circuit mutex), so per-call cost grows with the records the \
     window holds — at 45s and a 4166 msg/s forward link that is ~187500 records. Note \
     `circuit_breaker.sliding_window_size` is NOT a cap for TimeBased (the dependency only uses it \
     as the half-open fallback), so raising it does not bound the window"
}

// ---------------------------------------------------------------------------
// Case 1 — TimeBased, sequential scan cost
// ---------------------------------------------------------------------------

/// Per-call cost must not grow with the number of records in the window.
pub struct Issue547TimeBasedWindowScanCostTest;

impl TestCase for Issue547TimeBasedWindowScanCostTest {
    fn name(&self) -> &str {
        "issue547_timebased_window_scan_cost"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        if let Some(msg) = shipped_defaults_conflict() {
            return TestResult::error(self.name(), SUITE, start.elapsed(), msg);
        }

        let rt = runtime(2);
        let arm = match sequential_arm(&rt, WindowShape::TimeBased) {
            Ok(arm) => arm,
            Err(msg) => return TestResult::error(self.name(), SUITE, start.elapsed(), msg),
        };
        let reference = match count_based_reference(&rt) {
            Ok(r) => r,
            Err(msg) => return TestResult::error(self.name(), SUITE, start.elapsed(), msg),
        };

        let cross_ratio = arm.late_us / reference.second_us.max(f64::MIN_POSITIVE);
        let evidence = format!(
            "{}; CountBased reference {:.2} us/call (occupancy-independent, delta {:+.2} us across the run); \
             TimeBased/CountBased at {WINDOW_RECORDS} records: {cross_ratio:.0}x",
            arm.describe(WindowShape::TimeBased),
            reference.second_us,
            reference.delta_us()
        );

        if let Some(reason) = reference.degradation() {
            return TestResult::error(self.name(), SUITE, start.elapsed(), format!("{reason}. {evidence}"));
        }

        // The cross-arm ratio alone is decisive: both cost values come from the
        // same window occupancy, so no machine-wide slowdown can produce it.
        // Growth without a cross-arm gap is ambiguous and handled below.
        if cross_ratio >= CROSS_ARM_CEILING {
            let detail = format!(
                "issue #547 reproduced: {}. At {WINDOW_RECORDS} records a TimeBased call may cost at most \
                 {CROSS_ARM_CEILING:.0}x a CountBased call; measured {cross_ratio:.0}x, with per-call cost \
                 growing {:.1}x across the window (ceiling {SCAN_GROWTH_CEILING:.1}x). {evidence}.",
                defect_note(),
                arm.growth
            );
            tracing::warn!("{detail}");
            return TestResult::failed(self.name(), SUITE, start.elapsed(), detail);
        }

        if arm.growth >= SCAN_GROWTH_CEILING {
            // Growth without a cross-arm gap: the window scan is not what got
            // slower, so this is not a reproduction of #547.
            return TestResult::error(
                self.name(),
                SUITE,
                start.elapsed(),
                format!(
                    "the TimeBased arm grew {:.1}x but the cross-arm ratio stayed at {cross_ratio:.1}x \
                     (ceiling {CROSS_ARM_CEILING:.0}x), so the slowdown cannot be attributed to the window \
                     scan — the machine changed during the run. {evidence}",
                    arm.growth
                ),
            );
        }

        let note = format!(
            "the TimeBased breaker does not rescan its window per call (growth {:.1}x, ceiling \
             {SCAN_GROWTH_CEILING:.1}x; cross-arm {cross_ratio:.0}x, ceiling {CROSS_ARM_CEILING:.0}x). {evidence}",
            arm.growth
        );
        tracing::info!("{note}");
        TestResult::passed_with_note(self.name(), SUITE, start.elapsed(), &note)
    }

    fn timeout(&self) -> Duration {
        // ~150k calls per arm; the debug build's ~450 us/call at the far end
        // dominates, so allow for a slow machine.
        Duration::from_secs(300)
    }
}

// ---------------------------------------------------------------------------
// Case 2 — CountBased, same sequential load (positive control)
// ---------------------------------------------------------------------------

/// The control for case 1: **identical** sequential load shape, only the window
/// type differs. Running counters stay flat, so this case must pass — if it ever
/// fails, the harness or the dependency changed, not the machine.
pub struct Issue547CountBasedWindowScanCostTest;

impl TestCase for Issue547CountBasedWindowScanCostTest {
    fn name(&self) -> &str {
        "issue547_countbased_window_scan_cost"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        if let Some(msg) = shipped_defaults_conflict() {
            return TestResult::error(self.name(), SUITE, start.elapsed(), msg);
        }

        let rt = runtime(2);
        let arm = match sequential_arm(&rt, WindowShape::CountBased) {
            Ok(arm) => arm,
            Err(msg) => return TestResult::error(self.name(), SUITE, start.elapsed(), msg),
        };
        let evidence = format!(
            "{}; the same {WINDOW_RECORDS}-record occupancy costs a TimeBased call 17-30x more (see \
             issue547_timebased_window_scan_cost)",
            arm.describe(WindowShape::CountBased)
        );

        if arm.growth > CONTROL_GROWTH_CEILING && (arm.late_us - arm.first_us) > NOISE_FLOOR_US {
            return TestResult::failed(
                self.name(),
                SUITE,
                start.elapsed(),
                format!(
                    "the CountBased control regressed: per-call cost grew {:.1}x (ceiling \
                     {CONTROL_GROWTH_CEILING:.1}x) with an absolute change of {:+.2} us/call, while it only \
                     updates running counters and must be independent of the window occupancy. {evidence}",
                    arm.growth,
                    arm.late_us - arm.first_us
                ),
            );
        }

        // At ~0.3 us/call a ratio says nothing, so report *why* a growth above
        // the ceiling is still a pass instead of printing a contradiction.
        let delta_us = arm.late_us - arm.first_us;
        let flatness = if arm.growth > CONTROL_GROWTH_CEILING {
            format!(
                "growth {:.1}x is {delta_us:+.2} us/call of sub-microsecond noise, below the \
                 {NOISE_FLOOR_US:.0} us floor that would make it real",
                arm.growth
            )
        } else {
            format!("growth {:.1}x vs the {:.1}x ceiling", arm.growth, CONTROL_GROWTH_CEILING)
        };

        let note = format!(
            "control: the CountBased breaker stays flat under the same load — {flatness}. {evidence}"
        );
        tracing::info!("{note}");
        TestResult::passed_with_note(self.name(), SUITE, start.elapsed(), &note)
    }

    fn timeout(&self) -> Duration {
        // ~150k calls at a sub-microsecond cost: seconds, even in a debug build.
        Duration::from_secs(120)
    }
}

// ---------------------------------------------------------------------------
// Case 3 — TimeBased, concurrent ceiling
// ---------------------------------------------------------------------------

/// Throughput must not decay as the window fills.
///
/// This is the incident's actual signature: a breaker whose per-call cost grows
/// with its own window is self-limiting, so once the window holds the records of
/// the last `duration` at rate `r`, the achievable rate collapses to
/// `1 / √(a · duration)` regardless of how much load is offered — ~8 k calls/s
/// per gRPC link at the default 45 s. In the report that ceiling sat below the
/// 12 500 msg/s the deployment needed, the backlog grew, and a broker OOMed.
pub struct Issue547TimeBasedConcurrentCeilingTest;

impl TestCase for Issue547TimeBasedConcurrentCeilingTest {
    fn name(&self) -> &str {
        "issue547_timebased_concurrent_ceiling"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        if let Some(msg) = shipped_defaults_conflict() {
            return TestResult::error(self.name(), SUITE, start.elapsed(), msg);
        }

        let rt = runtime(worker_count());
        let (early, late, rejected) =
            concurrent_rates(&rt, WindowShape::TimeBased, TIME_BASED_LATE_SAMPLE_AT);
        let evidence = format!(
            "{CALLERS} concurrent callers on one breaker, calls/s: TimeBased 45s {early:.0} at {:.1}s -> \
             {late:.0} at {late_at:?} (ratio {:.2}; the CountBased control holds ~1.4M calls/s under the \
             identical load, see issue547_countbased_concurrent_ceiling)",
            EARLY_SAMPLE_AT.as_secs_f64() + SAMPLE_WINDOW.as_secs_f64() / 2.0,
            late / early.max(1.0),
            late_at = TIME_BASED_LATE_SAMPLE_AT + SAMPLE_WINDOW
        );

        if rejected > 0 {
            return TestResult::error(
                self.name(),
                SUITE,
                start.elapsed(),
                format!(
                    "{rejected} calls were refused by the breaker (circuit Open), so the rates are not a \
                     measurement of the window cost. {evidence}"
                ),
            );
        }
        if early < MIN_EARLY_RATE_TIME_BASED {
            return TestResult::error(
                self.name(),
                SUITE,
                start.elapsed(),
                format!(
                    "the callers only reached {early:.0} calls/s before the window filled (floor \
                     {MIN_EARLY_RATE_TIME_BASED:.0}/s), so this run cannot show a decay. {evidence}"
                ),
            );
        }

        let ratio = late / early;
        if ratio < DECAY_FLOOR {
            let detail = format!(
                "issue #547 reproduced: throughput through the default TimeBased circuit breaker decays as \
                 its window fills, because every call rescans the window under the circuit mutex \
                 (tower-resilience-circuitbreaker 0.10.1) — the scheme caps itself at ~1/sqrt(a*window) \
                 rather than serving the offered rate, which is how a 12500 msg/s cluster forward path ends \
                 up with a growing backlog and an OOM. Throughput may fall to at most {DECAY_FLOOR:.2} of the \
                 early rate; measured {ratio:.2}. {evidence}"
            );
            tracing::warn!("{detail}");
            return TestResult::failed(self.name(), SUITE, start.elapsed(), detail);
        }

        let note = format!(
            "throughput stays at {ratio:.2} of its early rate (floor {DECAY_FLOOR:.2}) while the window \
             fills. {evidence}"
        );
        tracing::info!("{note}");
        TestResult::passed_with_note(self.name(), SUITE, start.elapsed(), &note)
    }

    fn timeout(&self) -> Duration {
        // 20 s arm + sampling and teardown.
        Duration::from_secs(180)
    }
}

// ---------------------------------------------------------------------------
// Case 4 — CountBased, same concurrent load (positive control)
// ---------------------------------------------------------------------------

/// The control for case 3: the same 16-caller load on a breaker that only keeps
/// running counters. It must stay flat; a red result here means the load
/// generator or the machine is the problem, not the window type.
pub struct Issue547CountBasedConcurrentCeilingTest;

impl TestCase for Issue547CountBasedConcurrentCeilingTest {
    fn name(&self) -> &str {
        "issue547_countbased_concurrent_ceiling"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        if let Some(msg) = shipped_defaults_conflict() {
            return TestResult::error(self.name(), SUITE, start.elapsed(), msg);
        }

        let rt = runtime(worker_count());
        let (early, late, rejected) =
            concurrent_rates(&rt, WindowShape::CountBased, COUNT_BASED_LATE_SAMPLE_AT);
        let evidence = format!(
            "{CALLERS} concurrent callers on one breaker, calls/s: CountBased 20 {early:.0} at {:.1}s -> \
             {late:.0} at {late_at:?} (ratio {:.2})",
            EARLY_SAMPLE_AT.as_secs_f64() + SAMPLE_WINDOW.as_secs_f64() / 2.0,
            late / early.max(1.0),
            late_at = COUNT_BASED_LATE_SAMPLE_AT + SAMPLE_WINDOW
        );

        if rejected > 0 {
            return TestResult::error(
                self.name(),
                SUITE,
                start.elapsed(),
                format!("{rejected} calls were refused by the breaker (circuit Open). {evidence}"),
            );
        }
        if early < MIN_EARLY_RATE_COUNT_BASED {
            return TestResult::error(
                self.name(),
                SUITE,
                start.elapsed(),
                format!(
                    "the callers only reached {early:.0} calls/s (floor {MIN_EARLY_RATE_COUNT_BASED:.0}/s), \
                     so this run says nothing about the breaker. {evidence}"
                ),
            );
        }

        let ratio = late / early;
        if ratio < CONTROL_DECAY_FLOOR {
            return TestResult::failed(
                self.name(),
                SUITE,
                start.elapsed(),
                format!(
                    "the CountBased control decayed to {ratio:.2} of its early rate (floor \
                     {CONTROL_DECAY_FLOOR:.2}) although it only updates running counters — the load \
                     generator or the machine, not the window type, is the limit. {evidence}"
                ),
            );
        }

        let note = format!(
            "control: the CountBased breaker holds {ratio:.2} of its early rate (floor \
             {CONTROL_DECAY_FLOOR:.2}) under the same load. {evidence}"
        );
        tracing::info!("{note}");
        TestResult::passed_with_note(self.name(), SUITE, start.elapsed(), &note)
    }

    fn timeout(&self) -> Duration {
        // 5 s arm + sampling and teardown.
        Duration::from_secs(120)
    }
}
