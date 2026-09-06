//! MQTT v3.1.1 CONNACK return-code tests (G13, MQTT-3.2.2.3)
//!
//! The default broker config never produces the non-zero CONNACK codes, so
//! each test spawns its OWN self-managed broker (the default harness broker
//! on 1883 stays untouched) with a dedicated auth-enabled config:
//!
//! - `configs/auth-denied/`     — `rmqtt-auth-http`, anonymous disabled:
//!   the in-test mock HTTP auth server replies "allow" → CONNACK 0x00, or
//!   "deny" → CONNACK 0x04 (Bad user name or password).
//! - `configs/auth-jwt-denied/` — `rmqtt-auth-jwt`, anonymous disabled:
//!   an invalid JWT password → CONNACK 0x05 (Not authorized).
//!
//! 0x03 (Server unavailable) is a broker-internal condition that cannot be
//! triggered deterministically from the protocol side; it is left uncovered.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::broker::healthcheck::port_free_sync;
use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::tests::functional::cluster_session_restart::{rmqttd_binary, ClusterNode};

/// MQTT ports of the self-managed auth brokers (avoid the harness 1883, the
/// cluster tests 1886-1890, the session-expiry-cleanup 1884, rl-boundary 1891).
const AUTH_HTTP_ADDR: &str = "127.0.0.1:1892";
const AUTH_JWT_ADDR: &str = "127.0.0.1:1893";
const AUTH_NODE_START_TIMEOUT: Duration = Duration::from_secs(20);

/// Spawn a self-managed broker from an explicit config FILE (the `rmqtt.toml`
/// inside the prepared config directory).
///
/// NOTE: `-f` must point at the FILE, not the directory. rmqtt-conf loads the
/// `-f` path with `File::with_name(..).required(false)`, so a directory (or
/// any unreadable path) is SILENTLY skipped and the broker falls back to
/// built-in defaults — http-api on 6060, gRPC on 5363, MQTT on 1883 — which
/// then collides with the harness broker (WSAEADDRINUSE 10048) and the
/// intended auth config never takes effect.
fn spawn_auth_broker_with_config(
    config: PathBuf,
    label: &str,
    addr: &str,
) -> Result<(ClusterNode, PathBuf), anyhow::Error> {
    let binary = rmqttd_binary();
    if !binary.exists() {
        return Err(anyhow::anyhow!(
            "rmqttd binary not found at {:?}; build it first (cargo build -p rmqttd)",
            binary
        ));
    }
    if !config.is_file() {
        return Err(anyhow::anyhow!(
            "{label}: config path {:?} is not a file; `-f` must point at rmqtt.toml \
             (a directory is silently ignored by the broker and defaults get used)",
            config
        ));
    }
    // Pre-flight check: fail fast when the MQTT port is already occupied by a
    // residual broker / unrelated process, instead of burning the full start
    // timeout on a broker that can never bind.
    if !port_free_sync(addr) {
        return Err(anyhow::anyhow!(
            "{label}: MQTT address {addr} is already in use by another process; \
             a residual broker may still be running"
        ));
    }
    let log_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("target");
    let log_file = log_dir.join(format!("{label}-node.log"));
    let mut node = ClusterNode::new(config, addr, log_file);
    node.spawn(&binary)?;
    if !node.wait_healthy(AUTH_NODE_START_TIMEOUT) {
        return Err(anyhow::anyhow!(
            "{label} broker did not become healthy; log tail:\n{}",
            node.log_tail(30).unwrap_or_else(|| "<log file unreadable>".into())
        ));
    }
    Ok((node, binary))
}

/// Recursively copy a directory tree.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// Copy `configs/auth-denied/` into `target/auth-denied-<port>/` with the
/// auth-http mock port rewritten, so the mock can bind an ephemeral port
/// instead of a fixed 9099 (which external/residual processes can occupy).
fn prepare_auth_config(mock_port: u16) -> anyhow::Result<PathBuf> {
    // NOTE: config_path() returns the rmqtt.toml FILE — here we need the
    // config DIRECTORY to copy.
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("configs").join("auth-denied");
    let dst = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join(format!("auth-denied-{mock_port}"));
    copy_dir_recursive(&src, &dst)?;
    // Point the plugin config at the ephemeral mock port.
    let plugin_cfg = dst.join("plugins").join("rmqtt-auth-http.toml");
    let content = std::fs::read_to_string(&plugin_cfg)?;
    let updated = content.replace("9099", &mock_port.to_string());
    std::fs::write(&plugin_cfg, updated)?;
    // The main config's plugins.dir is a RELATIVE path (resolved against the
    // broker process CWD). Rewrite it to the ABSOLUTE copy path so the broker
    // always loads the rewritten plugin config regardless of where the
    // harness was launched from.
    let main_cfg = dst.join("rmqtt.toml");
    let content = std::fs::read_to_string(&main_cfg)?;
    let abs_plugins_dir = std::fs::canonicalize(dst.join("plugins"))?.to_string_lossy().replace('\\', "/"); // TOML-friendly separators on Windows too
    let updated = content.replace("rmqtt-test/configs/auth-denied/plugins/", &format!("{abs_plugins_dir}/"));
    std::fs::write(&main_cfg, updated)?;
    // Return the FILE, not the directory: `-f <dir>` is silently ignored by
    // rmqtt-conf (required(false)) and the broker would run on defaults.
    Ok(main_cfg)
}

/// Copy `configs/auth-jwt-denied/` into `target/auth-jwt-denied-<uid>/` with
/// its `plugins.dir` rewritten to an absolute path (removes the broker-CWD
/// dependence; no mock port to rewrite here).
fn prepare_jwt_config() -> anyhow::Result<PathBuf> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("configs").join("auth-jwt-denied");
    let uid = uuid::Uuid::new_v4().simple();
    let dst = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join(format!("auth-jwt-denied-{uid}"));
    copy_dir_recursive(&src, &dst)?;
    let main_cfg = dst.join("rmqtt.toml");
    let content = std::fs::read_to_string(&main_cfg)?;
    let abs_plugins_dir = std::fs::canonicalize(dst.join("plugins"))?.to_string_lossy().replace('\\', "/");
    let updated =
        content.replace("rmqtt-test/configs/auth-jwt-denied/plugins/", &format!("{abs_plugins_dir}/"));
    std::fs::write(&main_cfg, updated)?;
    // Return the FILE, not the directory (same reason as prepare_auth_config).
    Ok(main_cfg)
}

/// Build a raw v3.1.1 CONNECT with optional username / password.
/// Connect flags: clean session (0x02) | user name flag (0x80) |
/// password flag (0x40), as passed by the caller.
fn build_connect(
    connect_flags: u8,
    client_id: &str,
    username: Option<&str>,
    password: Option<&str>,
) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    body.extend_from_slice(&[0x00, 0x04]);
    body.extend_from_slice(b"MQTT");
    body.push(4); // level 4 (3.1.1)
    body.push(connect_flags);
    body.extend_from_slice(&[0x00, 0x3C]); // keep alive 60
    let cid = client_id.as_bytes();
    body.extend_from_slice(&(cid.len() as u16).to_be_bytes());
    body.extend_from_slice(cid);
    if let Some(u) = username {
        let u = u.as_bytes();
        body.extend_from_slice(&(u.len() as u16).to_be_bytes());
        body.extend_from_slice(u);
    }
    if let Some(p) = password {
        let p = p.as_bytes();
        body.extend_from_slice(&(p.len() as u16).to_be_bytes());
        body.extend_from_slice(p);
    }

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

/// Send a raw CONNECT and return the CONNACK return code, or `None` when the
/// broker closed the connection without a CONNACK.
fn connect_return_code(broker_addr: &str, packet: &[u8]) -> anyhow::Result<Option<u8>> {
    let mut stream = TcpStream::connect(broker_addr)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(packet)?;
    stream.flush()?;
    let mut buf = [0u8; 8];
    match stream.read(&mut buf) {
        Ok(n) if n >= 4 && buf[0] == 0x20 => Ok(Some(buf[3])),
        Ok(_) => Ok(None),
        Err(_) => Ok(None),
    }
}

/// Simple form-urlencoded field extraction (ASCII values only).
fn form_param(body: &str, key: &str) -> String {
    body.split('&')
        .find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            if k == key {
                Some(v.replace("%20", " ").replace('+', " "))
            } else {
                None
            }
        })
        .unwrap_or_default()
}

/// Start the in-test mock HTTP auth server on an EPHEMERAL port (the auth
/// config is rewritten to point at it, so no fixed port such as 9099 can be
/// occupied by external/residual processes).
///
/// Behaviour:
/// - POST /mqtt/auth — reply "allow" when `username == "good"`, else "deny"
/// - POST /mqtt/acl  — reply "allow" (never block pub/sub in these tests)
///
/// Each connection is served in a loop (keep-alive) so that reqwest's
/// connection pool can safely reuse a socket. The accept loop is cancellable:
/// aborting the returned handle closes the listener so the port is not
/// leaked into retries / later runs.
///
/// After spawning the accept loop this performs a readiness probe (a real
/// HTTP exchange) so the broker's first CONNECT can never race an
/// un-scheduled tokio accept task — a connection failure there would be
/// turned into a spurious 0x04 by `deny_if_error`.
///
/// Returns the join handle and the bound port (needed to rewrite the auth
/// config before the broker starts).
async fn spawn_mock_auth_server() -> anyhow::Result<(tokio::task::JoinHandle<()>, u16)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let mock_port = listener.local_addr()?.port();
    let addr = format!("127.0.0.1:{mock_port}");
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { break };
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = Vec::new();
                let mut tmp = [0u8; 1024];
                // Serve every request on this connection until it closes.
                loop {
                    buf.clear();
                    // read until the header terminator (or EOF = connection closed)
                    let head_end = loop {
                        match sock.read(&mut tmp).await {
                            Ok(0) => return, // connection closed -> stop serving
                            Ok(n) => {
                                buf.extend_from_slice(&tmp[..n]);
                                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                    break i + 4;
                                }
                            }
                            Err(_) => return,
                        }
                    };
                    // copy the head so we can still mutate `buf` below
                    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
                    // read the request body according to Content-Length
                    let content_length: usize = head
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    while buf.len() < head_end + content_length {
                        match sock.read(&mut tmp).await {
                            Ok(0) => break,
                            Ok(n) => buf.extend_from_slice(&tmp[..n]),
                            Err(_) => break,
                        }
                    }
                    let first_line = head.lines().next().unwrap_or("").to_string();
                    // Parse form fields from the request BODY only. The
                    // plugin serializes its params HashMap, so the field
                    // order varies per broker process; when `username`
                    // happens to be the first field it would otherwise be
                    // glued to the HTTP head ("...\r\n\r\nusername") and
                    // never match, turning the accepted user into a
                    // spurious deny -> CONNACK 0x04.
                    let body_only = String::from_utf8_lossy(&buf[head_end..]).into_owned();
                    // deny only auth requests whose username != "good"; ACL
                    // requests and accepted users are allowed
                    let uname = form_param(&body_only, "username");
                    let response =
                        if !first_line.contains("/mqtt/acl") && uname != "good" { "deny" } else { "allow" };
                    // Diagnostic: surface what the mock actually saw, so a
                    // spurious 0x04 can be traced without guessing.
                    eprintln!(
                        "[mock-auth] {} | username={:?} | decision={} | cl={}",
                        first_line, uname, response, content_length
                    );
                    let body = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
                        response.len(),
                        response
                    );
                    if sock.write_all(body.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });

    // Readiness probe: exchange a real request/response so the accept loop is
    // guaranteed to be scheduled and serving before the broker's first auth
    // call. Without this, a slow tokio scheduler could leave the broker's
    // connection unanswered, and `deny_if_error = true` would turn it into a
    // spurious CONNACK 0x04 for the accepted user.
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut probe = tokio::net::TcpStream::connect(&addr).await?;
        probe.write_all(b"GET /readiness HTTP/1.1\r\nHost: probe\r\n\r\n").await?;
        let mut buf = [0u8; 32];
        let n = tokio::time::timeout(Duration::from_secs(3), probe.read(&mut buf))
            .await
            .map_err(|_| anyhow::anyhow!("mock auth server did not become ready on {addr}"))??;
        if n == 0 {
            return Err(anyhow::anyhow!("mock auth server closed the readiness probe"));
        }
    }

    Ok((handle, mock_port))
}

// ---------------------------------------------------------------------------
// CONNACK 0x00 / 0x04 via rmqtt-auth-http (config: auth-denied)
// ---------------------------------------------------------------------------

/// Positive: with `rmqtt-auth-http` and anonymous access disabled, a CONNECT
/// the mock server accepts yields CONNACK 0x00; one the mock server rejects
/// yields CONNACK 0x04 (Bad user name or password). [MQTT-3.2.2.3]
pub struct ConnackReturnCodesAuthHttpV311Test;

impl TestCase for ConnackReturnCodesAuthHttpV311Test {
    fn name(&self) -> &str {
        "connack_return_codes_auth_http_v311"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let result = rt.block_on(async {
            // Start the mock first (ephemeral port), then rewrite the auth
            // config to point at that port, then start the broker — the
            // broker never uses a fixed mock port, so external/residual
            // processes occupying 9099 cannot break this test.
            let (mock, mock_port) = spawn_mock_auth_server().await?;
            let config = prepare_auth_config(mock_port)?;
            let (_node, _binary) = spawn_auth_broker_with_config(config, "auth-denied", AUTH_HTTP_ADDR)?;

            // Run the assertions inside a block so the mock listener is always
            // stopped (abort + wait) even when an assertion fails early —
            // otherwise a leaked listener keeps port 9099 occupied and the
            // next run fails with WSAEADDRINUSE (10048).
            let check = async {
                // 1) accepted username → 0x00
                let pkt_ok = build_connect(0xC2, "rc-ok", Some("good"), Some("pw"));
                match connect_return_code(AUTH_HTTP_ADDR, &pkt_ok)? {
                    Some(0) => {}
                    other => {
                        return Err(anyhow::anyhow!(
                            "expected CONNACK 0x00 for accepted user, got {:?}",
                            other
                        ))
                    }
                }

                // 2) rejected username → 0x04 (BadUserNameOrPassword)
                let pkt_bad = build_connect(0xC2, "rc-bad", Some("bad"), Some("pw"));
                match connect_return_code(AUTH_HTTP_ADDR, &pkt_bad)? {
                    Some(0x04) => {}
                    other => {
                        return Err(anyhow::anyhow!(
                            "expected CONNACK 0x04 (Bad user name or password) for denied user, got {:?}",
                            other
                        ))
                    }
                }
                Ok::<(), anyhow::Error>(())
            };

            let outcome = check.await;
            // Stop the accept loop and WAIT for it to finish, so the listener
            // is dropped and port 9099 is actually released (abort() alone is
            // async cancellation).
            mock.abort();
            let _ = mock.await;
            outcome
        });

        match result {
            Ok(()) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        // Must exceed AUTH_NODE_START_TIMEOUT (20s) + mock probe + cleanup,
        // otherwise the harness masks the real failure as "(timeout)".
        Duration::from_secs(45)
    }
}

// ---------------------------------------------------------------------------
// CONNACK 0x05 via rmqtt-auth-jwt (config: auth-jwt-denied)
// ---------------------------------------------------------------------------

/// Positive: with `rmqtt-auth-jwt` and anonymous access disabled, a CONNECT
/// carrying an invalid JWT in the password field yields CONNACK 0x05
/// (Not authorized). [MQTT-3.2.2.3]
pub struct ConnackNotAuthorizedV311Test;

impl TestCase for ConnackNotAuthorizedV311Test {
    fn name(&self) -> &str {
        "connack_not_authorized_v311"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();

        let verdict = (|| -> anyhow::Result<Option<u8>> {
            let config = prepare_jwt_config()?;
            let (_node, _binary) = spawn_auth_broker_with_config(config, "auth-jwt-denied", AUTH_JWT_ADDR)?;
            // flags = clean (0x02) | user name (0x80) | password (0x40)
            let pkt = build_connect(0xC2, "rc-jwt", Some("anyone"), Some("not-a-valid-jwt"));
            connect_return_code(AUTH_JWT_ADDR, &pkt)
        })();

        match verdict {
            Ok(Some(0x05)) => TestResult::passed(self.name(), "functional_v311", start.elapsed()),
            Ok(Some(code)) => TestResult::failed(
                self.name(),
                "functional_v311",
                start.elapsed(),
                format!("expected CONNACK 0x05 (Not authorized) for invalid JWT, got 0x{code:02x}"),
            ),
            Ok(None) => TestResult::failed(
                self.name(),
                "functional_v311",
                start.elapsed(),
                "broker closed the connection without a CONNACK for invalid JWT".into(),
            ),
            Err(e) => TestResult::failed(self.name(), "functional_v311", start.elapsed(), e.to_string()),
        }
    }

    fn timeout(&self) -> Duration {
        // Must exceed AUTH_NODE_START_TIMEOUT (20s) + cleanup, otherwise the
        // harness masks the real failure as "(timeout)".
        Duration::from_secs(45)
    }
}
