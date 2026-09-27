//! Connection flapping gate — MQTT 5.0 (rmqtt-flapping)
//!
//! Background: `ClientConnect` is the only hook every connection reaches.
//! `client_authenticate` short-circuits without running its chain when the
//! CONNECT carries no username and the listener allows anonymous access
//! (`rmqtt/src/hook.rs`), so a per-ClientId or per-IP gate has nowhere else to
//! live. The `rmqtt-flapping` plugin counts attempts in a sliding window and
//! refuses the one that reaches the threshold — before authentication, for the
//! whole duration of a ban, with the protocol's own reason code.
//!
//! These cases pin that behaviour through real transports, against the broker
//! `configs/flapping/` describes. The harness owns it (`broker_config()`), but
//! the fixture keeps its *own* ports rather than sharing the harness-wide
//! `--addr`: the cases declare `broker_addr()` so the health probe follows the
//! config, and connect to `BROKER_ADDR` themselves:
//!
//! - `flapping_ban_v5` — the first `max_count - 1` attempts are accepted, the
//!   one that reaches `max_count` is refused with CONNACK 0x8A (Banned), the
//!   ban is announced on `$SYS`, and it lapses on its own after `ban_time`;
//! - `flapping_api_unban_v5` — the management API lists the ban, reporting the
//!   gate as `available: true` (`GET /api/v1/features` carries the same state
//!   as `flapping: true`), and lifts it, after which the very same CONNECT is
//!   accepted again although the ban would still have been in force, and a
//!   second lift answers 404.
//!
//! Two details of the design are load-bearing for the assertions:
//!
//! 1. Anonymous access is enabled in the fixture, so the gate is the *only*
//!    possible source of a refusal — a non-zero CONNACK here cannot come from
//!    the authentication chain.
//! 2. Creating a ban **discards the window**: otherwise `ban_time <
//!    window_time` would re-ban the client the moment its ban lapses, turning
//!    a temporary ban into a permanent one. `flapping_ban_v5` therefore ends by
//!    asserting that a single attempt right after `ban_time` is accepted,
//!    which is exactly what the discarded window buys.
//!
//! `flapping_ban_v311.rs` covers the 3.1.1 side of the same gate (0x05), using
//! the helpers below. `flapping_order_v5.rs` pins *which* handlers see a
//! refusal; it needs a broker no other case talks to and therefore lives in
//! `configs/flapping-order/`.

use std::future::Future;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::anyhow;

use crate::framework::context::{TestConfig, TestContext};
use crate::framework::testcase::{TestCase, TestResult};
use crate::mqtt::common::QoS;
use crate::mqtt::v311::MqttV311Client;
use crate::tests::functional::connack_return_codes_v311::connect_return_code;

/// `by_clientid.max_count` in `configs/flapping/plugins/rmqtt-flapping.toml`.
/// The attempt that reaches this count is the one refused.
pub(crate) const MAX_COUNT: usize = 4;
/// `by_clientid.ban_time` in the same fixture.
pub(crate) const BAN_TIME: Duration = Duration::from_secs(5);
/// CONNACK 0x00: accepted.
pub(crate) const ACCEPTED: u8 = 0x00;
/// CONNACK 0x8A Banned — what `ConnectRefuse::Banned.to_v5()` maps to.
pub(crate) const REASON_BANNED_V5: u8 = 0x8A;
/// CONNACK 0x05 Not authorized — what `ConnectRefuse::Banned.to_v3()` maps to
/// (MQTT 3.1.1 defines no ban code, so a policy refusal becomes this one).
pub(crate) const REASON_BANNED_V311: u8 = 0x05;
/// The ban topic filter, with a wildcard where the node id goes: the test does
/// not know the harness broker's node id, and the fixture's template resolves
/// `{node}` to it.
pub(crate) const SYS_BAN_FILTER: &str = "$SYS/brokers/+/flapping/banned";
/// The manual-unban topic filter (`notify_on_unban = true` in the fixture).
pub(crate) const SYS_UNBAN_FILTER: &str = "$SYS/brokers/+/flapping/unbanned";
/// MQTT address of `configs/flapping/rmqtt.toml`. The fixture owns its ports
/// instead of sharing the harness-wide `--addr`, so the cases advertise this
/// through `broker_addr()` — the harness health-probes 1902 while the
/// sub-suite runs — and connect here themselves. Must match
/// `listener.tcp.external.addr` in that file.
pub(crate) const BROKER_ADDR: &str = "127.0.0.1:1902";
/// HTTP port of `configs/flapping/plugins/rmqtt-http-api.toml`. The harness
/// knows nothing about the plugin's own port, so it is pinned here and must
/// match that file.
pub(crate) const HTTP_ADDR: &str = "127.0.0.1:6066";
/// The ClientId each case crosses the threshold with. Every case needs a key of
/// its own: all of them share one broker process — the fixture config and
/// `broker_addr()` are the same for every case, so the scheduler never restarts
/// the broker between the `@flapping` sub-suites — and a window left behind by
/// one case would be spent by the next one, which then trips the gate early.
pub(crate) const VICTIM_BAN: &str = "flapping-victim-ban";
pub(crate) const VICTIM_API: &str = "flapping-victim-api";
/// `$SYS` watcher ClientIds, one per case for the same reason — and because the
/// two cases of `functional_v5@flapping` may run in parallel, where a shared
/// ClientId would have the second connection kick the first.
pub(crate) const WATCHER_BAN: &str = "flapping-watcher-ban";
pub(crate) const WATCHER_API: &str = "flapping-watcher-api";
/// Pause between two consecutive attempts that reuse one ClientId.
///
/// Each attempt drops its socket right after the CONNACK and the broker then has
/// to reclaim that session. Reconnecting before it does can hit a takeover race
/// that predates this gate: the old session is already gone, so the `kick` the
/// new handshake issues fails and the broker answers CONNACK 0x88 instead of
/// finishing the takeover. The gate counts CONNECT packets, not live sessions,
/// so the pause costs the assertions nothing.
const SETTLE: Duration = Duration::from_millis(30);

pub(crate) const SUITE_V5: &str = "functional_v5";
pub(crate) const SUITE_V311: &str = "functional_v311";

/// The fixture broker this module's cases are grouped under, so the harness
/// starts it and the scheduler switches config at the sub-suite boundary.
pub(crate) fn flapping_config() -> Option<PathBuf> {
    Some(crate::tests::config_path("flapping"))
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Runs an async body on a fresh runtime and maps the outcome onto a verdict
/// of `suite`. The future may borrow the `TestContext` (`block_on` does not
/// require `'static`).
pub(crate) fn run<'a>(
    suite: &str,
    name: &str,
    fut: impl Future<Output = anyhow::Result<()>> + 'a,
) -> TestResult {
    let start = Instant::now();
    let rt = tokio::runtime::Runtime::new().unwrap();
    match rt.block_on(fut) {
        Ok(()) => TestResult::passed(name, suite, start.elapsed()),
        Err(e) => TestResult::failed(name, suite, start.elapsed(), e.to_string()),
    }
}

/// Builds an MQTT 5.0 CONNECT with no properties.
///
/// The v3.1.1 builder next door hard-codes protocol level 4, so the two
/// versions cannot share one.
pub(crate) fn build_connect_v5(
    connect_flags: u8,
    client_id: &str,
    username: Option<&str>,
    password: Option<&str>,
) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&[0x00, 0x04]);
    body.extend_from_slice(b"MQTT");
    body.push(5); // level 5: MQTT 5.0
    body.push(connect_flags);
    body.extend_from_slice(&[0x00, 0x3C]); // keep alive 60s
    body.push(0x00); // properties: none
    body.extend_from_slice(&(client_id.len() as u16).to_be_bytes());
    body.extend_from_slice(client_id.as_bytes());
    if let Some(username) = username {
        body.extend_from_slice(&(username.len() as u16).to_be_bytes());
        body.extend_from_slice(username.as_bytes());
    }
    if let Some(password) = password {
        body.extend_from_slice(&(password.len() as u16).to_be_bytes());
        body.extend_from_slice(password.as_bytes());
    }

    let mut packet = vec![0x10];
    packet.extend_from_slice(&encode_varint(body.len() as u32));
    packet.extend_from_slice(&body);
    packet
}

/// MQTT remaining-length encoding (variable byte integer, MQTT-1.5.5).
fn encode_varint(mut value: u32) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (value % 128) as u8;
        value /= 128;
        if value > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            return out;
        }
    }
}

/// Sends `times` identical CONNECTs, each on its own connection, and returns
/// the CONNACK code of every one.
///
/// Every attempt drops its socket right after the CONNACK: the gate counts
/// CONNECT packets, not live sessions, and an abrupt close is what a client
/// retrying in a loop actually does. Attempts are spaced by `SETTLE` so that the
/// next one does not race the previous connection's teardown.
fn connect_attempts(addr: &str, packet: &[u8], times: usize) -> anyhow::Result<Vec<u8>> {
    let mut codes = Vec::with_capacity(times);
    for attempt in 1..=times {
        match connect_return_code(addr, packet)? {
            Some(code) => codes.push(code),
            None => {
                return Err(anyhow!(
                    "attempt {attempt}/{times} got no CONNACK — the broker closed the connection"
                ));
            }
        }
        std::thread::sleep(SETTLE);
    }
    Ok(codes)
}

/// Walks a ClientId up to the threshold: asserts that the first
/// `max_count - 1` attempts are accepted, and returns the code the attempt
/// that reaches `max_count` was refused with.
pub(crate) fn cross_threshold(addr: &str, packet: &[u8], label: &str) -> anyhow::Result<u8> {
    let below = MAX_COUNT - 1;
    let codes = connect_attempts(addr, packet, below)?;
    if let Some((index, code)) = codes.iter().enumerate().find(|(_, code)| **code != ACCEPTED) {
        return Err(anyhow!(
            "{label}: attempt {} of {below} must be accepted while the window is below \
             max_count = {MAX_COUNT}, got CONNACK 0x{code:02x} (all codes: {codes:02x?})",
            index + 1,
        ));
    }
    connect_return_code(addr, packet)?
        .ok_or_else(|| anyhow!("{label}: the attempt that reaches max_count got no CONNACK"))
}

/// Connects a v3.1.1 watcher under `client_id` and subscribes to a `$SYS`
/// filter, draining anything stale so the assertions below cannot read a
/// leftover message.
///
/// Connects to `BROKER_ADDR`, not `cfg.broker_addr`: the fixture owns its own
/// port, so the harness-wide `--addr` is not where this broker listens. The
/// ClientId is a parameter rather than a constant because the callers share one
/// broker and may run in parallel.
pub(crate) async fn sys_watcher(
    cfg: &TestConfig,
    client_id: &str,
    filter: &str,
) -> anyhow::Result<MqttV311Client> {
    let mut watcher = MqttV311Client::connect(BROKER_ADDR, client_id, cfg.connect_timeout).await?;
    watcher.subscribe(filter, QoS::AtMostOnce).await?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    while watcher.recv_message_timeout(Duration::from_millis(100)).await.is_some() {}
    Ok(watcher)
}

/// Waits for the `event` notice about `key`, skipping unrelated `$SYS` traffic.
/// Returns the payload as text.
pub(crate) async fn await_sys_event(
    watcher: &mut MqttV311Client,
    event: &str,
    key: &str,
    timeout: Duration,
) -> Option<String> {
    let needle_event = format!("\"event\":\"{event}\"");
    let needle_key = format!("\"key\":\"{key}\"");
    let start = Instant::now();
    while start.elapsed() < timeout {
        if let Some(msg) = watcher.recv_message_timeout(Duration::from_millis(200)).await {
            let body = String::from_utf8_lossy(&msg.payload).into_owned();
            if body.contains(&needle_event) && body.contains(&needle_key) {
                return Some(body);
            }
        }
    }
    None
}

/// Minimal HTTP/1.1 request (no external HTTP client dependency). Returns the
/// status code and the response body.
///
/// Shared with `flapping_order_v5.rs`, which reads the node metrics over the
/// same API.
pub(crate) async fn http_request(addr: &str, method: &str, path: &str) -> anyhow::Result<(u16, String)> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::TcpStream::connect(addr).await?;
    let req =
        format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;

    let text = String::from_utf8_lossy(&buf).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| anyhow!("malformed HTTP response: {text:.80}"))?;
    let body = text.split_once("\r\n\r\n").map_or(String::new(), |(_, body)| body.to_string());
    Ok((status, body))
}

// ---------------------------------------------------------------------------
// Cases
// ---------------------------------------------------------------------------

/// The threshold refusal, its `$SYS` announcement, and the lapse of the ban.
pub struct FlappingBanV5Test;

impl TestCase for FlappingBanV5Test {
    fn name(&self) -> &str {
        "flapping_ban_v5"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        flapping_config()
    }

    fn broker_addr(&self) -> Option<&'static str> {
        Some(BROKER_ADDR)
    }

    fn timeout(&self) -> Duration {
        // Broker start + the attempts + BAN_TIME + headroom.
        Duration::from_secs(45)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(SUITE_V5, self.name(), async {
            let addr = BROKER_ADDR;
            // clean_start = 1, no username: the fixture allows anonymous
            // access, so nothing but the gate can refuse this CONNECT.
            let packet = build_connect_v5(0x02, VICTIM_BAN, None, None);
            let mut watcher = sys_watcher(&ctx.config, WATCHER_BAN, SYS_BAN_FILTER).await?;

            let refused = cross_threshold(addr, &packet, VICTIM_BAN)?;
            if refused != REASON_BANNED_V5 {
                return Err(anyhow!(
                    "{VICTIM_BAN}: the attempt that reaches max_count = {MAX_COUNT} must be refused \
                     with CONNACK 0x{REASON_BANNED_V5:02x} (Banned), got 0x{refused:02x}"
                ));
            }

            let notice = await_sys_event(&mut watcher, "banned", VICTIM_BAN, Duration::from_secs(10))
                .await
                .ok_or_else(|| {
                anyhow!(
                    "no `banned` notice for {VICTIM_BAN} arrived on {SYS_BAN_FILTER} within 10s, \
                         although the CONNECT was refused with 0x{refused:02x}"
                )
            })?;
            // The payload has to carry what an operator acts on: which key was
            // banned and on how many attempts.
            let expected_count = format!("\"count\":{MAX_COUNT}");
            if !notice.contains("\"dimension\":\"clientid\"") || !notice.contains(&expected_count) {
                return Err(anyhow!(
                    "the `banned` notice does not identify the ClientId dimension and \
                     {expected_count}: {notice}"
                ));
            }

            // The ban lapses on its own. One attempt must be enough: the window
            // was discarded when the ban was created, so this client starts
            // from a full budget instead of immediately re-tripping the gate.
            tokio::time::sleep(BAN_TIME + Duration::from_millis(700)).await;
            let after = connect_return_code(addr, &packet)?
                .ok_or_else(|| anyhow!("{VICTIM_BAN}: no CONNACK after the ban lapsed"))?;
            if after != ACCEPTED {
                return Err(anyhow!(
                    "{VICTIM_BAN}: the ban is still in force {:?} after it was created, \
                     although ban_time = {BAN_TIME:?} — CONNACK 0x{after:02x}",
                    BAN_TIME + Duration::from_millis(700),
                ));
            }

            watcher.disconnect().await?;
            Ok(())
        })
    }
}

/// The management API: list a ban, lift it by hand, and see the gate follow
/// immediately (`rmqtt::flapping::Flapping` installed into the core's
/// extension slot by the plugin's `init`).
///
/// The list also has to say *that a gate is screening at all*, not only what it
/// bans: an empty table and an absent gate are otherwise indistinguishable, and
/// neither the list nor `/api/v1/features` may report one as the other.
pub struct FlappingApiUnbanV5Test;

impl TestCase for FlappingApiUnbanV5Test {
    fn name(&self) -> &str {
        "flapping_api_unban_v5"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        flapping_config()
    }

    fn broker_addr(&self) -> Option<&'static str> {
        Some(BROKER_ADDR)
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        run(SUITE_V5, self.name(), async {
            let addr = BROKER_ADDR;
            let packet = build_connect_v5(0x02, VICTIM_API, None, None);

            let refused = cross_threshold(addr, &packet, VICTIM_API)?;
            if refused != REASON_BANNED_V5 {
                return Err(anyhow!(
                    "{VICTIM_API}: expected CONNACK 0x{REASON_BANNED_V5:02x} once max_count = \
                     {MAX_COUNT} is reached, got 0x{refused:02x}"
                ));
            }

            // Subscribe before lifting, so the announcement cannot be missed.
            let mut watcher = sys_watcher(&ctx.config, WATCHER_API, SYS_UNBAN_FILTER).await?;
            let path = format!("/api/v1/flapping/banned?dimension=clientid&key={VICTIM_API}");

            let (status, body) = http_request(HTTP_ADDR, "GET", &path).await?;
            if status != 200 {
                return Err(anyhow!("GET {path} answered {status}: {body:.200}"));
            }
            // `available` is what tells a gate that is not screening apart from
            // one that is screening and simply has nothing banned — the two read
            // the same `banned_count`, so the flag is the only thing separating
            // them. This fixture installs the gate with `by_clientid` enabled,
            // so it must read `true` here.
            if !body.contains("\"available\":true") {
                return Err(anyhow!(
                    "GET {path} did not report the gate as installed and screening: {body:.300}"
                ));
            }
            if !body.contains(VICTIM_API) || !body.contains("\"dimension\":\"clientid\"") {
                return Err(anyhow!("GET {path} did not list the ban on {VICTIM_API}: {body:.300}"));
            }

            // The same state through the feature summary, so an operator that
            // asks once is not left guessing whether `flapping_banned: 0` means
            // "nothing is banned" or "no gate is running".
            let (status, body) = http_request(HTTP_ADDR, "GET", "/api/v1/features").await?;
            if status != 200 {
                return Err(anyhow!("GET /api/v1/features answered {status}: {body:.200}"));
            }
            if !body.contains("\"flapping\":true") {
                return Err(anyhow!(
                    "GET /api/v1/features did not report `flapping: true`, although the \
                     fixture's gate is screening: {body:.300}"
                ));
            }

            let (status, body) = http_request(HTTP_ADDR, "DELETE", &path).await?;
            if status != 200 {
                return Err(anyhow!("DELETE {path} answered {status}: {body:.200}"));
            }
            if !body.contains("\"unbanned\":true") {
                return Err(anyhow!("DELETE {path} did not report the lift: {body:.200}"));
            }

            // The ban would still be in force for several seconds, so an
            // accepted CONNECT now can only mean the lift reached the gate.
            let after = connect_return_code(addr, &packet)?
                .ok_or_else(|| anyhow!("{VICTIM_API}: no CONNACK after the ban was lifted"))?;
            if after != ACCEPTED {
                return Err(anyhow!(
                    "{VICTIM_API}: still refused with CONNACK 0x{after:02x} right after the \
                     management API reported the ban as lifted — the gate and the API disagree"
                ));
            }

            let notice = await_sys_event(&mut watcher, "unbanned", VICTIM_API, Duration::from_secs(10))
                .await
                .ok_or_else(|| {
                    anyhow!(
                        "no `unbanned` notice for {VICTIM_API} arrived on {SYS_UNBAN_FILTER} within \
                         10s, although the ban was lifted"
                    )
                })?;
            if !notice.contains("\"reason\":\"manual\"") {
                return Err(anyhow!("the `unbanned` notice does not mark a manual lift: {notice}"));
            }

            // Lifting twice is a lookup miss, not a second lift: 404.
            let (status, _) = http_request(HTTP_ADDR, "DELETE", &path).await?;
            if status != 404 {
                return Err(anyhow!(
                    "a second DELETE of the same ban answered {status}, expected 404 — \
                     there was nothing left to lift"
                ));
            }

            watcher.disconnect().await?;
            Ok(())
        })
    }
}
