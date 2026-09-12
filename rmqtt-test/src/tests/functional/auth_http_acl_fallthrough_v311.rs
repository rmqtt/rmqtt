//! Issue #501 — auth-http 'ignore' × rmqtt-acl final-rule fallthrough tests
//!
//! Background (GitHub issue #501 + code audit, rmqtt-auth-http/src/lib.rs
//! `response_result()` L307-309): when the HTTP auth service replies with a
//! non-2xx status code (404/500/502, ...), the request itself succeeded, so
//! `deny_if_error` does NOT apply (it only covers transport-layer failures).
//! The plugin yields `ignore` and the auth chain continues.
//!
//! The built-in `rmqtt-acl` plugin also hooks `ClientAuthenticate`
//! (rmqtt-acl/src/lib.rs L189-220) and its rules are then evaluated for the
//! CONNECT. The outcome depends ENTIRELY on the last ACL rule:
//!
//! - `["allow", "all"]` (the shipped default): the omitted action column
//!   resolves to ALL operations including CONNECT
//!   (rmqtt-acl/src/config.rs `Control::try_from(None) => Control::All`,
//!   `User::All` matches everyone), so the connection is EXPLICITLY allowed
//!   — even with `allow_anonymous = false`. This is the issue #501
//!   fail-open: a misconfigured route / reverse-proxy error page turns into
//!   a successful authentication.
//! - `["deny", "all"]` (documented hardening): the connection is rejected
//!   with `NotAuthorized` → CONNACK 0x05 — fail-closed.
//!
//! Each test spawns its OWN self-managed broker (the default harness broker
//! on 1883 stays untouched) from `configs/auth-http-acl-fallthrough/`, with
//! an in-test mock auth server that ALWAYS replies HTTP 404, and the acl
//! plugin's final rule rewritten per test:
//!
//! - allow-all case: MQTT 1896 / gRPC 5376 → expects CONNACK 0x00
//! - deny-all case:  MQTT 1900 / gRPC 5377 → expects CONNACK 0x05
//!
//! `http_acl_req` is intentionally omitted from the plugin config so that
//! subscribe/publish ACL decisions also fall through to rmqtt-acl (the
//! assertions here are connect-phase, matching the issue report).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::tests::functional::connack_return_codes_v311::{
    build_connect, connect_return_code, copy_dir_recursive, spawn_auth_broker_with_config,
};

/// MQTT port of the allow-all broker (issue #501 reproduction).
const ACL_ALLOWALL_ADDR: &str = "127.0.0.1:1896";
/// gRPC port of the allow-all broker.
const ACL_ALLOWALL_RPC: &str = "0.0.0.0:5376";
/// MQTT port of the deny-all broker (fail-closed verification).
const ACL_DENYALL_ADDR: &str = "127.0.0.1:1900";
/// gRPC port of the deny-all broker.
const ACL_DENYALL_RPC: &str = "0.0.0.0:5377";
const NODE_START_TIMEOUT: Duration = Duration::from_secs(20);
const SUITE: &str = "functional_v311";

/// Generate the rmqtt-acl plugin config with the given FINAL rule. The first
/// three rules mirror the shipped defaults (they do not match CONNECT);
/// the final rule alone decides the connect-phase outcome.
fn acl_rules_toml(final_rule: &str) -> String {
    format!(
        "rules = [\n    \
         [\"allow\", {{ user = \"dashboard\" }}, \"subscribe\", [\"$SYS/#\"]],\n    \
         [\"allow\", {{ ipaddr = \"127.0.0.1\" }}, \"pubsub\", [\"$SYS/#\", \"#\"]],\n    \
         [\"deny\", \"all\", \"subscribe\", [\"$SYS/#\", {{ eq = \"#\" }}]],\n    \
         {final_rule}\n]\n"
    )
}

/// Copy `configs/auth-http-acl-fallthrough/` into
/// `target/auth-http-acl-<label>-<uid>/` and rewrite:
/// 1. the mock auth port (9099 → ephemeral port of the in-test mock),
/// 2. the MQTT listener port (base 1896 → per-test port),
/// 3. the gRPC port (base 5376 → per-test port),
/// 4. `plugins.dir` to an ABSOLUTE path (removes the broker-CWD dependence),
/// 5. the rmqtt-acl FINAL rule (`allow` / `deny` variant).
///
/// Returns the rmqtt.toml FILE path (`-f` must point at the file — a
/// directory is silently ignored by rmqtt-conf and defaults get used).
fn prepare_fallthrough_config(
    label: &str,
    final_rule: &str,
    mock_port: u16,
    mqtt_addr: &str,
    rpc_addr: &str,
) -> anyhow::Result<PathBuf> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("configs").join("auth-http-acl-fallthrough");
    let uid = uuid::Uuid::new_v4().simple();
    let dst = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join(format!("auth-http-acl-{label}-{uid}"));
    copy_dir_recursive(&src, &dst)?;

    // 1) auth plugin: point at the ephemeral mock port.
    let plugin_cfg = dst.join("plugins").join("rmqtt-auth-http.toml");
    let content = std::fs::read_to_string(&plugin_cfg)?;
    std::fs::write(&plugin_cfg, content.replace("9099", &mock_port.to_string()))?;

    // 2) acl plugin: install the final rule under test.
    let acl_cfg = dst.join("plugins").join("rmqtt-acl.toml");
    std::fs::write(&acl_cfg, acl_rules_toml(final_rule))?;

    // 3) main config: per-test MQTT/gRPC ports + absolute plugins.dir.
    let main_cfg = dst.join("rmqtt.toml");
    let content = std::fs::read_to_string(&main_cfg)?;
    let abs_plugins_dir = std::fs::canonicalize(dst.join("plugins"))?.to_string_lossy().replace('\\', "/");
    let updated = content
        .replace("rmqtt-test/configs/auth-http-acl-fallthrough/plugins/", &format!("{abs_plugins_dir}/"))
        .replace("127.0.0.1:1896", mqtt_addr)
        .replace("0.0.0.0:5376", rpc_addr);
    std::fs::write(&main_cfg, updated)?;
    Ok(main_cfg)
}

/// Start the in-test mock auth server that ALWAYS replies HTTP 404 (empty
/// body) — the non-2xx failure mode from issue #501 (misconfigured route /
/// reverse-proxy error page). The transport itself is healthy, so the
/// plugin's `deny_if_error` must not fire; the expected outcome is a clean
/// 'ignore' and a continued auth chain.
///
/// Ephemeral port + readiness probe, same rationale as
/// `connack_return_codes_v311::spawn_mock_auth_server`: a real HTTP exchange
/// guarantees the accept loop is serving before the broker's first CONNECT,
/// so a transport error can never be misread as an auth decision.
async fn spawn_mock_404_auth_server() -> anyhow::Result<(tokio::task::JoinHandle<()>, u16)> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let mock_port = listener.local_addr()?.port();
    let addr = format!("127.0.0.1:{mock_port}");
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { break };
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 1024];
                loop {
                    buf.clear();
                    // read until the header terminator (or EOF)
                    let head_end = loop {
                        match sock.read(&mut tmp).await {
                            Ok(0) => return,
                            Ok(n) => {
                                buf.extend_from_slice(&tmp[..n]);
                                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                    break i + 4;
                                }
                            }
                            Err(_) => return,
                        }
                    };
                    let first_line = String::from_utf8_lossy(&buf[..head_end]).into_owned();
                    let first_line = first_line.lines().next().unwrap_or("").to_string();
                    eprintln!("[mock-404-auth] {first_line} -> 404");
                    let body = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    if sock.write_all(body.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });

    // Readiness probe: a real HTTP exchange before the broker starts.
    {
        let mut probe = tokio::net::TcpStream::connect(&addr).await?;
        probe.write_all(b"GET /readiness HTTP/1.1\r\nHost: probe\r\n\r\n").await?;
        let mut buf = [0u8; 64];
        let n = tokio::time::timeout(Duration::from_secs(3), probe.read(&mut buf))
            .await
            .map_err(|_| anyhow::anyhow!("mock 404 auth server did not become ready on {addr}"))??;
        if n == 0 {
            return Err(anyhow::anyhow!("mock 404 auth server closed the readiness probe"));
        }
        if !buf.starts_with(b"HTTP/1.1 404") {
            return Err(anyhow::anyhow!("mock 404 auth server replied unexpectedly: {buf:?}"));
        }
    }

    Ok((handle, mock_port))
}

/// Shared execution skeleton: mock 404 server → prepared config (final ACL
/// rule) → self-managed broker → single raw CONNECT → returned CONNACK code.
async fn run_fallthrough_scenario(
    label: &str,
    final_rule: &str,
    mqtt_addr: &str,
    rpc_addr: &str,
    client_id: &str,
) -> anyhow::Result<Option<u8>> {
    let (mock, mock_port) = spawn_mock_404_auth_server().await?;
    // Run the assertions inside a block so the mock listener is always
    // stopped (abort + wait) even when an assertion fails early.
    let check = async {
        let config = prepare_fallthrough_config(label, final_rule, mock_port, mqtt_addr, rpc_addr)?;
        let (_node, _binary) = spawn_auth_broker_with_config(config, label, mqtt_addr)?;
        // flags = clean (0x02) | user name (0x80) | password (0x40)
        let pkt = build_connect(0xC2, client_id, Some("intruder"), Some("whatever"));
        connect_return_code(mqtt_addr, &pkt)
    };
    let outcome = check.await;
    mock.abort();
    let _ = mock.await;
    outcome
}

// ---------------------------------------------------------------------------
// Issue #501 reproduction: acl ["allow", "all"] promotes 'ignore' to 'allow'
// ---------------------------------------------------------------------------

/// Reproduction of issue #501 (fail-open): with `rmqtt-auth-http` enabled and
/// `allow_anonymous = false`, a CONNECT whose auth request is answered with
/// HTTP 404 (no auth decision → plugin yields 'ignore') is EXPLICITLY ALLOWED
/// by the default rmqtt-acl final rule `["allow", "all"]` → CONNACK 0x00.
pub struct AuthHttpIgnoreAllowAllAclV311Test;

impl TestCase for AuthHttpIgnoreAllowAllAclV311Test {
    fn name(&self) -> &str {
        "auth_http_ignore_allow_all_acl_v311"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(run_fallthrough_scenario(
            "auth-acl-allowall",
            "[\"allow\", \"all\"]",
            ACL_ALLOWALL_ADDR,
            ACL_ALLOWALL_RPC,
            "acl-allowall",
        ));

        match result {
            // The fail-open behavior: allowed despite allow_anonymous = false.
            Ok(Some(0x00)) => TestResult::passed(self.name(), SUITE, start.elapsed()),
            Ok(Some(code)) => TestResult::failed(
                self.name(),
                SUITE,
                start.elapsed(),
                format!(
                    "expected CONNACK 0x00 (issue #501 fail-open: rmqtt-acl [\"allow\", \"all\"] \
                     promotes an auth-http 'ignore' — non-2xx auth response — into a successful \
                     connection), got 0x{code:02x}"
                ),
            ),
            Ok(None) => TestResult::failed(
                self.name(),
                SUITE,
                start.elapsed(),
                "broker closed the connection without a CONNACK (issue #501 fail-open scenario)".into(),
            ),
            Err(e) => TestResult::failed(self.name(), SUITE, start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        // Must exceed NODE_START_TIMEOUT (20s) + mock probe + cleanup.
        Duration::from_secs(45)
    }
}

// ---------------------------------------------------------------------------
// Fail-closed hardening: acl ["deny", "all"] rejects the 'ignore' connection
// ---------------------------------------------------------------------------

/// Fail-closed verification of the issue #501 guidance: with the rmqtt-acl
/// final rule `["deny", "all"]`, the same HTTP-404 'ignore' connection is
/// rejected → CONNACK 0x05 (Not authorized).
pub struct AuthHttpIgnoreDenyAllAclV311Test;

impl TestCase for AuthHttpIgnoreDenyAllAclV311Test {
    fn name(&self) -> &str {
        "auth_http_ignore_deny_all_acl_v311"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(run_fallthrough_scenario(
            "auth-acl-denyall",
            "[\"deny\", \"all\"]",
            ACL_DENYALL_ADDR,
            ACL_DENYALL_RPC,
            "acl-denyall",
        ));

        match result {
            // The fail-closed behavior: rejected by the ACL backstop rule.
            Ok(Some(0x05)) => TestResult::passed(self.name(), SUITE, start.elapsed()),
            Ok(Some(code)) => TestResult::failed(
                self.name(),
                SUITE,
                start.elapsed(),
                format!(
                    "expected CONNACK 0x05 (Not authorized; fail-closed: rmqtt-acl \
                     [\"deny\", \"all\"] backstop rejects the auth-http 'ignore' from a non-2xx \
                     auth response), got 0x{code:02x}"
                ),
            ),
            Ok(None) => TestResult::failed(
                self.name(),
                SUITE,
                start.elapsed(),
                "broker closed the connection without a CONNACK (issue #501 fail-closed scenario)".into(),
            ),
            Err(e) => TestResult::failed(self.name(), SUITE, start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(45)
    }
}
