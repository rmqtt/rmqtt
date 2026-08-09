//! TCP keepalive / transport-layer dead-peer tests (GitHub issue #465)
//!
//! Issue: RMQTT never sets `SO_KEEPALIVE` on accepted MQTT/TCP connections
//! (`rmqtt-net/src/builder.rs` `accept_tcp()` only called `set_nodelay`), so
//! the kernel never probes idle peers. Under cellular / CGNAT "NAT black
//! holes" the peer silently drops FIN/RST, the MQTT layer cannot finish the
//! TCP teardown, and ESTABLISHED / FIN-WAIT-1 connections pile up while the
//! MQTT session count stays near the real device count. Tuning the sysctls
//! `net.ipv4.tcp_keepalive_*` had no effect because the socket option was
//! never set.
//!
//! Fix (see `designs/issue-465-tcp-keepalive-fix.md`): accepted connections
//! now enable SO_KEEPALIVE by default, with optional per-listener probe
//! parameters `{ idle, interval, probes }`.
//!
//! These tests cover the testable halves of the issue:
//!
//! 1. `tcp_keepalive_socket_option` — Linux-gated root-cause assertion: after
//!    a normal MQTT connect, inspect the broker-side accepted socket with
//!    `ss -o` and check whether it shows a keepalive timer
//!    (`timer:(keepalive,...)`). Requires `ss` (iproute2); skipped with a
//!    note when unavailable, when not on Linux, or when `ss` returns no
//!    output. Judgment is based on the matched connection row plus the few
//!    following lines (iproute2 omits the trailing `timer:(...)` entirely for
//!    idle sockets with no pending timer, and `state established` filtering
//!    drops the State column), so the failure message also dumps the full `ss`
//!    output for diagnosis.
//! 2. `tcp_keepalive_custom_params` — Linux-gated: with the explicit probe
//!    parameters of `configs/tcp-keepalive/` (`idle = 1s`), the keepalive
//!    timer on the accepted socket must be in the seconds range (the kernel
//!    default would be minutes/hours), proving the per-listener parameters
//!    are applied. Runs in the `functional_v5@tcp-keepalive` sub-suite.
//! 3. `mqtt_keepalive_timeout_reclaims_tcp` — portable behavioural baseline:
//!    with a short MQTT keepalive the broker must close the *TCP* connection
//!    (raw read returns EOF) after the keep-alive window (1.5x) elapses. This
//!    documents that the MQTT-layer defence works; it is exactly the case
//!    TCP keepalive must cover when MQTT keepalive cannot (keep_alive = 0 or
//!    a black hole that swallows the teardown FIN).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};

/// Suite these tests report under (part of the regular functional_v5 suite).
const SUITE: &str = "functional_v5";

/// Build a raw MQTT v5 CONNECT ("MQTT" / level 5, clean start, no props)
/// with the given keep-alive value.
fn raw_connect_v5(client_id: &str, keep_alive: u16) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&[0x00, 0x04]);
    body.extend_from_slice(b"MQTT");
    body.push(5); // level
    body.push(0x02); // clean start
    body.extend_from_slice(&keep_alive.to_be_bytes());
    body.push(0x00); // property length = 0
    let cid = client_id.as_bytes();
    body.extend_from_slice(&(cid.len() as u16).to_be_bytes());
    body.extend_from_slice(cid);

    let mut pkt = vec![0x10];
    let mut len = body.len();
    loop {
        let mut b = (len % 128) as u8;
        len /= 128;
        if len > 0 {
            b |= 0x80;
        }
        pkt.push(b);
        if len == 0 {
            break;
        }
    }
    pkt.extend_from_slice(&body);
    pkt
}

/// Read one full MQTT packet (fixed header + remaining length) from the stream.
fn read_full_packet(stream: &mut TcpStream) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut b = [0u8; 1];
    let n = stream.read(&mut b)?;
    if n == 0 {
        return Err(anyhow::anyhow!("connection closed"));
    }
    buf.push(b[0]);

    let mut remaining: u32 = 0;
    let mut shift = 0u32;
    loop {
        let n = stream.read(&mut b)?;
        if n == 0 {
            return Err(anyhow::anyhow!("connection closed mid-header"));
        }
        buf.push(b[0]);
        remaining |= ((b[0] & 0x7F) as u32) << shift;
        if b[0] & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift > 21 {
            return Err(anyhow::anyhow!("malformed remaining length"));
        }
    }

    let mut rest = vec![0u8; remaining as usize];
    stream.read_exact(&mut rest)?;
    buf.extend_from_slice(&rest);
    Ok(buf)
}

/// Open a raw TCP connection, send a valid v5 CONNECT, consume the complete
/// CONNACK, and return the stream (with a read timeout already applied).
fn raw_connect(broker_addr: &str, client_id: &str, keep_alive: u16) -> anyhow::Result<TcpStream> {
    let mut stream = TcpStream::connect(broker_addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let pkt = raw_connect_v5(client_id, keep_alive);
    stream.write_all(&pkt)?;
    stream.flush()?;
    let connack = read_full_packet(&mut stream)?;
    if connack.len() < 4 || connack[0] != 0x20 || connack[3] != 0 {
        return Err(anyhow::anyhow!("CONNECT refused: {:02x?}", &connack[..connack.len().min(8)]));
    }
    Ok(stream)
}

/// Extract the TCP port from a `host:port` broker address.
fn broker_port(broker_addr: &str) -> anyhow::Result<u16> {
    broker_addr
        .rsplit_once(':')
        .and_then(|(_, p)| p.parse().ok())
        .ok_or_else(|| anyhow::anyhow!("cannot parse broker port from {broker_addr}"))
}

/// Probe whether the `ss` binary (iproute2) is available.
fn ss_available() -> bool {
    Command::new("ss").arg("-V").output().is_ok()
}

/// Run `ss -tno state established "( sport = :PORT )"` and return the output.
///
/// `-o` (timers) is the key flag: for a socket with SO_KEEPALIVE the line ends
/// with `timer:(keepalive,<remaining>,<retries>)`. Note that with
/// `state established` filtering iproute2 drops the State column, and idle
/// sockets with no pending timer show no `timer:(...)` at all — the absence of
/// `timer:(keepalive,` is therefore the expected signature of a socket without
/// SO_KEEPALIVE.
fn run_ss(port: u16) -> anyhow::Result<String> {
    let filter = format!("( sport = :{port} )");
    let out = Command::new("ss")
        .args(["-tno", "state", "established", &filter])
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run `ss`: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Connect to the broker, let the connection go idle for `idle_secs`, run
/// `ss -o` against the broker port, and return `(ss_output, client_port,
/// broker_port)`.
fn probe_ss(ctx: &TestContext, keep_alive: u16, idle_secs: u64) -> anyhow::Result<(String, u16, u16)> {
    let uid = uuid::Uuid::new_v4().simple().to_string();
    let stream = raw_connect(&ctx.config.broker_addr, &format!("ka-{uid}"), keep_alive)?;
    let client_port = stream.local_addr()?.port();
    let port = broker_port(&ctx.config.broker_addr)?;
    // Idle so a keepalive timer (if enabled) becomes visible in `ss -o`.
    std::thread::sleep(Duration::from_secs(idle_secs));
    let output = run_ss(port)?;
    Ok((output, client_port, port))
}

/// Find the `ss` row for our accepted connection (local = broker port, peer =
/// our client port) and check the row plus the few following lines for a
/// keepalive timer. Returns the matching line if found. `state established`
/// filtering makes iproute2 drop the State column, and some ss versions omit
/// the trailing `timer:(...)` entirely when the socket has no pending timer,
/// so a strict single-line match would be unreliable.
fn find_keepalive_timer_line(output: &str, client_port: u16, port: u16) -> Option<String> {
    let idx = output
        .lines()
        .position(|l| l.contains(&format!(":{client_port}")) && l.contains(&format!(":{port}")))?;
    output.lines().skip(idx).take(3).find(|l| l.contains("timer:(keepalive,")).map(String::from)
}

/// Parse the keepalive timer remaining time (seconds) from an `ss -o` line
/// like `... timer:(keepalive,1.000s,0)`, `timer:(keepalive,004ms,0)` or
/// `timer:(keepalive,120.000min,0)`. The unit may be `us`/`ms`/`s`/`min`/`h`.
///
/// An empty value (`timer:(keepalive,,0)`) means the keepalive timer just
/// fired and is being reset — ss reports zero remaining time — which is
/// treated as 0s. A bare number with no unit (`timer:(keepalive,0,0)`, as some
/// ss versions emit when the value is zero) is treated as seconds.
fn keepalive_timer_seconds(line: &str) -> Option<f64> {
    let needle = "timer:(keepalive,";
    let start = line.find(needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find(',')?;
    let num_unit = &rest[..end];
    if num_unit.is_empty() {
        // Timer just fired / is being reset: 0s remaining.
        return Some(0.0);
    }
    // Split the leading number from the trailing unit. All digits and the
    // decimal point are ASCII, so byte indices are safe.
    let num_len = num_unit.bytes().take_while(|b| b.is_ascii_digit() || *b == b'.').count();
    if num_len == 0 {
        return None;
    }
    let num: f64 = num_unit[..num_len].parse().ok()?;
    let unit = &num_unit[num_len..];
    Some(match unit {
        "us" => num / 1_000_000.0,
        "ms" => num / 1_000.0,
        "s" | "" => num,
        "min" => num * 60.0,
        "h" => num * 3_600.0,
        _ => return None,
    })
}

/// Root-cause assertion for GitHub issue #465 (Linux-gated).
///
/// After a successful MQTT connect the broker-side accepted socket must have
/// SO_KEEPALIVE enabled. On Linux this is observable via the `ss -o` timer
/// column: with SO_KEEPALIVE set, an idle accepted socket shows
/// `timer:(keepalive,...)`. Before the fix this test failed — reproducing the
/// issue; after the fix it passes as a regression guard.
pub struct TcpKeepAliveSocketOptionTest;

impl TestCase for TcpKeepAliveSocketOptionTest {
    fn name(&self) -> &str {
        "tcp_keepalive_socket_option"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        if cfg!(not(target_os = "linux")) {
            return TestResult::skipped(
                self.name(),
                SUITE,
                start.elapsed(),
                "SO_KEEPALIVE inspection needs Linux `ss -o`; skipped on this platform",
            );
        }
        if !ss_available() {
            return TestResult::skipped(
                self.name(),
                SUITE,
                start.elapsed(),
                "`ss` (iproute2) not available; cannot inspect socket timers",
            );
        }

        let (output, client_port, port) = match probe_ss(ctx, 60, 3) {
            Err(e) => return TestResult::failed(self.name(), SUITE, start.elapsed(), e.to_string()),
            Ok(v) => v,
        };

        if output.trim().is_empty() {
            return TestResult::skipped(
                self.name(),
                SUITE,
                start.elapsed(),
                "`ss` returned no output for the broker port; cannot inspect socket timers",
            );
        }

        let verdict = match find_keepalive_timer_line(&output, client_port, port) {
            Some(_) => Ok(()),
            None => Err(anyhow::anyhow!(
                "accepted MQTT/TCP socket has NO SO_KEEPALIVE (no `timer:(keepalive,...)` in \
                 `ss -o` output) — reproduces GitHub issue #465 (dead peers are never probed; \
                 connections pile up under cellular/CGNAT NAT black holes).\n`ss` output:\n{output}"
            )),
        };

        match verdict {
            Ok(()) => TestResult::passed(self.name(), SUITE, start.elapsed()),
            Err(e) => TestResult::failed(self.name(), SUITE, start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// Phase-2 verification: explicit per-listener probe parameters take effect
/// (Linux-gated).
///
/// Runs against `rmqtt-test/configs/tcp-keepalive/` where
/// `tcp_keepalive = { idle = "1s", interval = "1s", probes = 2 }`. After a
/// normal connect, the `ss -o` keepalive timer on the accepted socket must be
/// in the seconds range — the kernel default (`tcp_keepalive_time`, typically
/// 7200s) would show minutes/hours — proving the configured parameters are
/// applied. A silent loopback peer answers keepalive probes (its kernel is
/// alive), so this verifies parameter application, not dead-peer reclamation
/// (which cannot be simulated without root-level packet drops).
pub struct TcpKeepaliveCustomParamsTest;

impl TestCase for TcpKeepaliveCustomParamsTest {
    fn name(&self) -> &str {
        "tcp_keepalive_custom_params"
    }

    fn broker_config(&self) -> Option<PathBuf> {
        Some(crate::tests::config_path("tcp-keepalive"))
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        if cfg!(not(target_os = "linux")) {
            return TestResult::skipped(
                self.name(),
                SUITE,
                start.elapsed(),
                "SO_KEEPALIVE inspection needs Linux `ss -o`; skipped on this platform",
            );
        }
        if !ss_available() {
            return TestResult::skipped(
                self.name(),
                SUITE,
                start.elapsed(),
                "`ss` (iproute2) not available; cannot inspect socket timers",
            );
        }

        let (output, client_port, port) = match probe_ss(ctx, 60, 2) {
            Err(e) => return TestResult::failed(self.name(), SUITE, start.elapsed(), e.to_string()),
            Ok(v) => v,
        };

        if output.trim().is_empty() {
            return TestResult::skipped(
                self.name(),
                SUITE,
                start.elapsed(),
                "`ss` returned no output for the broker port; cannot inspect socket timers",
            );
        }

        let verdict = (|| -> anyhow::Result<()> {
            let line = find_keepalive_timer_line(&output, client_port, port).ok_or_else(|| {
                anyhow::anyhow!(
                    "no `timer:(keepalive,...)` for the accepted connection — SO_KEEPALIVE or \
                     per-listener parameters not applied.\n`ss` output:\n{output}"
                )
            })?;
            let secs = keepalive_timer_seconds(&line).ok_or_else(|| {
                anyhow::anyhow!("cannot parse keepalive timer from `ss` line: {line}\n`ss` output:\n{output}")
            })?;
            if secs < 60.0 {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "keepalive timer {secs:.1}s indicates kernel defaults were used — the \
                     configured `{{ idle = \"1s\", interval = \"1s\", probes = 2 }}` parameters \
                     were not applied.\n`ss` line: {line}\n`ss` output:\n{output}"
                ))
            }
        })();

        match verdict {
            Ok(()) => TestResult::passed(self.name(), SUITE, start.elapsed()),
            Err(e) => TestResult::failed(self.name(), SUITE, start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// Behavioural baseline: with a short MQTT keep-alive, the broker must close
/// the *TCP* connection once the keep-alive window (1.5x) elapses.
///
/// Uses a raw socket so the assertion is on the wire (read returns EOF), not
/// on client-side bookkeeping. This is the MQTT-layer defence that works; the
/// TCP keepalive option (issue #465) is what must additionally cover the
/// cases this cannot: keep_alive = 0, or a black hole that swallows the FIN.
pub struct MqttKeepaliveTimeoutReclaimsTcpTest;

impl TestCase for MqttKeepaliveTimeoutReclaimsTcpTest {
    fn name(&self) -> &str {
        "mqtt_keepalive_timeout_reclaims_tcp"
    }

    fn execute(&self, ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let uid = uuid::Uuid::new_v4().simple().to_string();
        let client_id = format!("kat-{uid}");

        let result = (|| -> anyhow::Result<()> {
            let mut stream = raw_connect(&ctx.config.broker_addr, &client_id, 5)?; // keep-alive 5s

            // 1. Shortly after CONNACK the connection must still be alive:
            //    a read with a short timeout must time out (no EOF).
            stream.set_read_timeout(Some(Duration::from_secs(3)))?;
            let mut probe = [0u8; 1];
            match stream.read(&mut probe) {
                Err(_) => {} // alive: read timed out, no data, no EOF
                Ok(0) => {
                    return Err(anyhow::anyhow!(
                        "broker closed the TCP connection before the keep-alive window elapsed"
                    ));
                }
                Ok(n) => {
                    return Err(anyhow::anyhow!(
                        "unexpected {n} bytes from broker right after CONNACK: {:02x?}",
                        &probe[..n]
                    ));
                }
            }

            // 2. Stay silent past the keep-alive window: timeout = 1.5 * 5 = 7.5s.
            std::thread::sleep(Duration::from_secs(10));

            // 3. The broker must now have closed the connection: read -> EOF (0).
            stream.set_read_timeout(Some(Duration::from_secs(3)))?;
            let mut buf = [0u8; 1];
            match stream.read(&mut buf) {
                Ok(0) => Ok(()), // EOF: TCP connection reclaimed
                Ok(n) => {
                    Err(anyhow::anyhow!("expected TCP EOF after MQTT keep-alive timeout, got {n} bytes"))
                }
                Err(e) => Err(anyhow::anyhow!(
                    "TCP connection still open {e:?} after MQTT keep-alive timeout — not reclaimed"
                )),
            }
        })();

        match result {
            Ok(()) => TestResult::passed(self.name(), SUITE, start.elapsed()),
            Err(e) => TestResult::failed(self.name(), SUITE, start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(40)
    }
}

#[cfg(test)]
mod keepalive_timer_seconds_tests {
    use super::keepalive_timer_seconds;

    #[test]
    fn parses_milliseconds() {
        // ss emits `ms` for sub-second timers, e.g. right after a probe fires.
        let line = "0      0  127.0.0.1:1883  127.0.0.1:58480 timer:(keepalive,004ms,0)";
        let secs = keepalive_timer_seconds(line).expect("004ms must parse");
        assert!((secs - 0.004).abs() < 1e-9, "got {secs}s");
    }

    #[test]
    fn empty_value_is_zero() {
        // ss shows an empty value in the instant the keepalive timer fires and
        // resets (observed intermittently with idle = 1s).
        let line = "0      0  127.0.0.1:1883  127.0.0.1:44512 timer:(keepalive,,0)";
        let secs = keepalive_timer_seconds(line).expect("empty value must parse as 0s");
        assert_eq!(secs, 0.0, "got {secs}s");
    }

    #[test]
    fn bare_zero_without_unit_is_zero() {
        // Some ss versions emit a bare `0` with no unit when the value is zero.
        let line = "0      0  127.0.0.1:1883  127.0.0.1:44512 timer:(keepalive,0,0)";
        let secs = keepalive_timer_seconds(line).expect("bare 0 must parse as 0s");
        assert_eq!(secs, 0.0, "got {secs}s");
    }

    #[test]
    fn parses_seconds() {
        let line = "ESTAB 0 0 127.0.0.1:1883 127.0.0.1:58480 timer:(keepalive,1.000s,0)";
        let secs = keepalive_timer_seconds(line).expect("1.000s must parse");
        assert!((secs - 1.0).abs() < 1e-9, "got {secs}s");
    }

    #[test]
    fn parses_minutes() {
        // Kernel default tcp_keepalive_time = 7200s shows as minutes.
        let line = "ESTAB 0 0 127.0.0.1:1883 127.0.0.1:58480 timer:(keepalive,120.000min,0)";
        let secs = keepalive_timer_seconds(line).expect("120.000min must parse");
        assert!((secs - 7200.0).abs() < 1e-9, "got {secs}s");
    }

    #[test]
    fn rejects_non_keepalive_timer() {
        assert!(keepalive_timer_seconds("timer:(on,000ms,0)").is_none());
    }

    #[test]
    fn rejects_garbage() {
        assert!(keepalive_timer_seconds("timer:(keepalive,abc,0)").is_none());
    }
}
