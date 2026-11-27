//! Which `ClientConnect` handlers see a refusal — MQTT 5.0
//! (rmqtt-flapping + rmqtt-counter)
//!
//! `flapping_v5.rs` pins *what* the gate does — the threshold refusal, its
//! `$SYS` notice, the lapse, the management API. This module pins *who sees
//! the refusal*: the handlers the gate must not hide it from.
//!
//! The `ClientConnect` chain is dispatched highest priority first, and a
//! handler that returns `proceed = false` short-circuits every handler below
//! it (`rmqtt/src/hook.rs`). `rmqtt-counter` and `rmqtt-web-hook` register at
//! `Priority::MAX` and always return `(true, acc)`: they exist to observe, so
//! a gate that runs *before* them erases the very event they count. Two
//! things follow, and both are asserted here through the counter, which
//! publishes its tally as `scx.metrics.client_connect`:
//!
//! 1. the gate must register strictly below `Priority::MAX`, and
//! 2. every CONNECT must reach the counter — including the one the gate
//!    refuses.
//!
//! `flapping_observer_order_v5` walks a ClientId over the threshold
//! (`max_count = 4`: three accepted, the fourth refused) and reads the node
//! metrics back over the HTTP API. The delta of `client_connect` has to be
//! `max_count`, refused CONNECT included.
//!
//! # Why this case owns a broker
//!
//! `Metrics` is a *node-wide* accumulator, so "the delta is exactly the number
//! of CONNECTs this case sent" is only decidable on a broker nobody else
//! talks to. The cases of `flapping_v5.rs` share one broker process — two of
//! them may even run in parallel — and their CONNECTs would land in the same
//! counter. Hence `configs/flapping-order/`: sub-suites are split by
//! (config, addr), so a config of its own buys a broker process of its own,
//! on ports of its own (1903 MQTT / 5380 gRPC / 6067 HTTP), and its counters
//! start at zero. The harness health probe is a bare TCP connect
//! (`src/broker/healthcheck.rs`), which never produces a CONNECT, and
//! the baseline is read before the first attempt anyway.
//!
//! # What a red result means, and its one blind spot
//!
//! A delta short of `max_count` means some CONNECT never reached the counter:
//! the gate was dispatched first and short-circuited it. `client_connack` is
//! read alongside and reported in the failure message as corroboration — the
//! refusal still emits a CONNACK, so an observer that missed the CONNECT has
//! the counters drift apart.
//!
//! The blind spot is that ties are broken by a randomly generated handler id
//! (`DefaultHookManager::add`), and the order is fixed for the life of a
//! broker process. Registering the gate back at `Priority::MAX` therefore
//! makes *this* case red only about half the time — one coin flip per process,
//! which sending more refusals does not improve. The deterministic guard
//! against that regression is the unit test next to the constant in
//! `rmqtt-plugins/rmqtt-flapping/src/lib.rs`; this case is the end-to-end
//! counterpart that proves the arrangement works through a real transport,
//! with the real counter plugin.
//!
//! The attempts are spaced by `ATTEMPT_GAP` instead of reusing
//! `flapping_v5::cross_threshold`: that helper's 30ms gap was measured to be
//! too tight on Windows, where the broker occasionally answers the *next*
//! handshake with CONNACK 0x88 — it has not reclaimed the previous session
//! yet, the takeover `kick` fails, and the connection ends with "server
//! unavailable". That race predates the gate and is tolerated here (see the
//! constant), whereas `cross_threshold`'s stricter "every attempt below the
//! threshold is 0x00" would turn it into a spurious failure.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::anyhow;

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::tests::functional::connack_return_codes_v311::connect_return_code;
use crate::tests::functional::flapping_v5::{
    build_connect_v5, http_request, run, ACCEPTED, MAX_COUNT, REASON_BANNED_V5, SUITE_V5,
};

/// MQTT address of `configs/flapping-order/rmqtt.toml`. This fixture owns its
/// ports instead of sharing the harness-wide `--addr`, so the case advertises
/// this through `broker_addr()` — the harness health-probes 1903 while the
/// `functional_v5@flapping-order` sub-suite runs — and connects here itself.
/// Must match `listener.tcp.external.addr` in that file.
const BROKER_ADDR: &str = "127.0.0.1:1903";
/// HTTP port of `configs/flapping-order/plugins/rmqtt-http-api.toml`, where
/// the node metrics are read from. Must match `http_laddr` in that file.
const HTTP_ADDR: &str = "127.0.0.1:6067";
/// The path that returns this node's own `Metrics` (the fixture runs one node,
/// `node.id = 1`).
const METRICS_PATH: &str = "/api/v1/metrics/1";
/// The ClientId the case crosses the threshold with. Only one case runs
/// against this fixture, so there is no window to leave behind for another
/// one — but the key stays named for the failure messages.
const VICTIM: &str = "flapping-victim-order";
/// CONNACK 0x88 Server unavailable — what the takeover race below answers with.
const REASON_SERVER_UNAVAILABLE_V5: u8 = 0x88;
/// Pause between two consecutive attempts that reuse one ClientId.
///
/// `flapping_v5::SETTLE` uses 30ms; on Windows that proved too tight here. Each
/// attempt drops its socket right after the CONNACK and the broker has yet to
/// reclaim that session, so the next handshake's takeover `kick` fails and the
/// broker answers CONNACK 0x88 instead of finishing the takeover — a race that
/// predates the gate (documented at `flapping_v5::SETTLE`) and has nothing to
/// do with the dispatch order asserted here. The gate counts CONNECT packets
/// rather than live sessions, so waiting longer costs the assertion nothing,
/// and 0x88 stays tolerated below for the same reason.
const ATTEMPT_GAP: Duration = Duration::from_millis(250);

/// Reads this node's `(client_connect, client_connack)` counters, in absolute
/// values for the life of the broker process.
///
/// The response is `{"node":{"id":1,...},"metrics":{...,"client.connect":N,...}}`
/// (`api.rs::_build_metrics` wraps what `Metrics::to_json` produces), and the
/// keys are dotted: the derive macro turns the field's underscores into dots
/// (`rmqtt-macros/src/metrics.rs`), so `client_connect` is read as
/// `client.connect`.
async fn read_counts(addr: &str) -> anyhow::Result<(u64, u64)> {
    let (status, body) = http_request(addr, "GET", METRICS_PATH).await?;
    if status != 200 {
        return Err(anyhow!("GET {METRICS_PATH} answered {status}: {body:.200}"));
    }
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| anyhow!("GET {METRICS_PATH} did not answer JSON ({e}): {body:.200}"))?;
    let metrics = &json["metrics"];
    let counter = |name: &str| -> anyhow::Result<u64> {
        metrics[name]
            .as_u64()
            .ok_or_else(|| anyhow!("GET {METRICS_PATH} carries no `{name}` counter: {body:.300}"))
    };
    Ok((counter("client.connect")?, counter("client.connack")?))
}

/// Every CONNECT reaches the counter that observes the `ClientConnect` chain —
/// the refused one included.
///
/// The counters are node-wide and this fixture's broker is the case's alone, so
/// the assertion is an exact delta rather than a lower bound: the case sends
/// `max_count` CONNECTs and expects exactly `max_count` counted.
pub struct FlappingObserverOrderV5Test;

impl TestCase for FlappingObserverOrderV5Test {
    fn name(&self) -> &str {
        "flapping_observer_order_v5"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        Some(crate::tests::config_path("flapping-order"))
    }

    fn broker_addr(&self) -> Option<&'static str> {
        Some(BROKER_ADDR)
    }

    fn timeout(&self) -> Duration {
        // Broker start + the attempts + two HTTP reads.
        Duration::from_secs(45)
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        run(SUITE_V5, self.name(), async {
            // Read the baseline first: a broker that just started should sit at
            // zero, but the assertion must not depend on that.
            let (connects_before, connacks_before) = read_counts(HTTP_ADDR).await?;

            // clean_start = 1, no username: the fixture allows anonymous
            // access, so nothing but the gate can refuse this CONNECT.
            let packet = build_connect_v5(0x02, VICTIM, None, None);
            let mut codes = Vec::with_capacity(MAX_COUNT);
            for attempt in 1..=MAX_COUNT {
                let code = connect_return_code(BROKER_ADDR, &packet)?.ok_or_else(|| {
                    anyhow!(
                        "{VICTIM}: attempt {attempt}/{MAX_COUNT} got no CONNACK — the broker \
                         closed the connection"
                    )
                })?;
                codes.push(code);
                if attempt < MAX_COUNT {
                    std::thread::sleep(ATTEMPT_GAP);
                }
            }

            // The window counts CONNECT packets, so the `max_count`-th one is
            // the refusal — whatever the earlier attempts were answered with.
            let refused = codes[MAX_COUNT - 1];
            if refused != REASON_BANNED_V5 {
                return Err(anyhow!(
                    "{VICTIM}: the attempt that reaches max_count = {MAX_COUNT} must be refused \
                     with CONNACK 0x{REASON_BANNED_V5:02x} (Banned), got 0x{refused:02x} \
                     (all codes: {codes:02x?})"
                ));
            }
            // Below the threshold the gate has no business refusing anything.
            // 0x88 is tolerated because it is the takeover race described on
            // `ATTEMPT_GAP`, not the gate; anything else non-zero here would
            // mean the threshold fires early, which the delta check below could
            // no longer distinguish from a correct run.
            if let Some((index, code)) = codes[..MAX_COUNT - 1]
                .iter()
                .enumerate()
                .find(|(_, code)| !matches!(**code, ACCEPTED | REASON_SERVER_UNAVAILABLE_V5))
            {
                return Err(anyhow!(
                    "{VICTIM}: attempt {} of {} must be accepted while the window is below \
                     max_count = {MAX_COUNT}, got CONNACK 0x{code:02x} (all codes: {codes:02x?})",
                    index + 1,
                    MAX_COUNT - 1,
                ));
            }

            // The counters are incremented inside the `ClientConnect` hook,
            // i.e. before the CONNACK that ended the last attempt is sent, so
            // both reads are settled by now — no sleep, no retry.
            let (connects_after, connacks_after) = read_counts(HTTP_ADDR).await?;
            let connects = connects_after.saturating_sub(connects_before);
            let connacks = connacks_after.saturating_sub(connacks_before);
            if connects != MAX_COUNT as u64 {
                return Err(anyhow!(
                    "{VICTIM}: sent {MAX_COUNT} CONNECTs, the last one refused with \
                     0x{REASON_BANNED_V5:02x}, but `client.connect` (the `client_connect` \
                     counter) moved by {connects} ({connects_before} -> {connects_after}): the \
                     CONNECT that the gate refused never reached rmqtt-counter. A refused \
                     CONNECT must still be dispatched to the observers that register at \
                     `Priority::MAX`, so the gate has to stay below them — see \
                     CONNECT_GATE_PRIORITY in rmqtt-plugins/rmqtt-flapping/src/lib.rs. For \
                     reference `client.connack` moved by {connacks} ({connacks_before} -> \
                     {connacks_after}), which is what an observer-blind refusal looks like: the \
                     CONNACK is counted, the CONNECT is not. (all codes: {codes:02x?})"
                ));
            }

            Ok(())
        })
    }
}
