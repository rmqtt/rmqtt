//! Bridge entry topic FILTER fields are never validated (GitHub issue #540)
//!
//! Issue #451 fixed the topic **name** side of a bridge entry (the field a mapped
//! PUBLISH ends up in). The sibling field of every entry is a topic **filter**,
//! and neither MQTT bridge plugin validates it — see issue #540:
//!
//! | plugin | field | role | validated today |
//! |---|---|---|---|
//! | `rmqtt-bridge-ingress-mqtt` | `remote.topic` | the filter the bridge SUBSCRIBEs with on the peer | **nothing at all** (`Remote::topic` is a plain `String`) |
//! | `rmqtt-bridge-egress-mqtt` | `local.topic_filter` | the filter the bridge matches local publishes against | only when the bridge starts (`Topic::from_str(...)?`), whose failure restarts the whole bridge every 3 s |
//!
//! The consequence is the worst of the available outcomes: the broker starts
//! normally, the bridge connects, the peer refuses the subscription (or drops the
//! connection because the filter is malformed), and the entry silently delivers
//! nothing while the operator sees a healthy-looking bridge. A filter that is
//! perfectly well-formed but simply refused by the peer (an ACL, say) is covered
//! by the runtime half of the issue; THIS suite covers what is decidable while
//! the configuration is loaded.
//!
//! # Cases
//!
//! Every case runs its OWN `rmqttd` process, because the contract under test is
//! "the broker must refuse to serve with this configuration" — which the
//! harness's managed broker cannot express: `BrokerProcess::start()` treats a
//! broker that does not become healthy as a harness error, not as a verdict.
//!
//! The generated configuration is a copy of `configs/bridge-egress-mqtt/` (the
//! #451 fixture: both MQTT bridge plugins, `log.to = "file"`, its own port block)
//! with exactly ONE field injected, so a rejection can never be blamed on some
//! unrelated key being missing. (The runtime case is the exception: it is not a
//! rejection case, and it additionally pins `reconnect_interval`, which is the
//! base of the retry backoff it measures.) Each case owns its own port block so a
//! leaked process cannot masquerade as "refused to start" in the next case.
//!
//! | case | field | value | expected |
//! |---|---|---|---|
//! | `issue540_ingress_remote_topic_filter_invalid`      | ingress `remote.topic` | `up/#/x` (`#` not last) | refuse to start, name `bridges[0].entries[0].remote.topic` |
//! | `issue540_ingress_remote_topic_empty`               | ingress `remote.topic` | omitted (`#[serde(default)]` -> `""`) | refuse to start |
//! | `issue540_egress_topic_filter_invalid`              | egress `local.topic_filter` | `eg/#/x` | refuse to start (today: starts, then 3 s restart loop) |
//! | `issue540_egress_topic_filter_empty`                | egress `local.topic_filter` | omitted (`""`) | refuse to start (today: installs a `[Blank]` trie node that matches nothing) |
//! | `issue540_egress_topic_filter_share_prefix`         | egress `local.topic_filter` | `$share/g/eg/#` | refuse to start (`$share` is peer-facing-only) |
//! | `issue540_egress_topic_filter_limit_prefix`         | egress `local.topic_filter` | `$limit/10/eg/#` | refuse to start (`$limit/<n>/` is peer-facing-only) |
//! | `issue540_egress_topic_filter_exclusive_prefix`     | egress `local.topic_filter` | `$exclusive/eg/#` | refuse to start (`$exclusive/` is peer-facing-only) |
//! | `issue540_ingress_remote_topic_filter_overlong`     | ingress `remote.topic` | `$share/<65535 'g's>x/a` (65545 bytes) | refuse to start (the 65535-byte cap applies to the WHOLE string, §1.5.4) |
//! | `issue540_valid_filters_start`                      | — (positive control)  | all filters legal, incl. ingress `$share/g/up/a/#` and egress `$SYS/#` | start and become healthy |
//! | `issue540_ingress_v311_subscribe_refused`           | runtime (MQTT 3.1.1)  | peer answers SUBACK `0x80` | refusal reported at WARN/ERROR with the reason, retries bounded |
//!
//! `issue540_ingress_v311_subscribe_refused` needs a bare MQTT 3.1.1 peer: the
//! v5 mock of the sibling suite cannot serve a `mqtt_ver = "v4"` bridge, and the
//! two protocol paths of the ingress plugin disagree on the severity of exactly
//! this event (`src/v4.rs` logs `info!` where `src/v5.rs` logs `warn!`), which is
//! part of the issue's narrower ask.
//!
//! # What is NOT pinned
//!
//! * The wording of the rejection beyond the substrings each case declares
//!   (field path, the offending value, and that a topic *filter* is what belongs
//!   there). The fix is free to phrase it as it likes.
//! * The exit code / mechanism: a broker that panics, exits, or hangs without
//!   ever serving is accepted; what is asserted is that it never starts serving
//!   with that configuration, and that it says why.
//! * MQTT 3.1.1's *load-time* rejection: the v4 case is the runtime one.
//!
//! # Running it
//!
//! ```text
//! target/debug/mqtt_harness --binary target/debug/rmqttd --workspace . \
//!     --suites bridge_config_reject --workers 1
//! ```
//!
//! Ports owned by this suite: MQTT 1925-1934 / gRPC 5405-5414 / HTTP 6085-6094,
//! plus 2925 for the MQTT 3.1.1 mock. The harness's own broker is untouched.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use rmqtt_codec::v3::{
    Codec as V3Codec, ConnectAck as ConnectAckV3, ConnectAckReason as ConnectAckReasonV3, Packet as PacketV3,
    SubscribeReturnCode,
};
use tokio::net::TcpListener;
use tokio_util::codec::Framed;

use crate::broker::healthcheck::{health_check_sync, port_free_sync};
use crate::framework::context::TestContext;
use crate::framework::testcase::{TestCase, TestResult};
use crate::tests::functional::bridge_egress_mqtt::warn_or_error_record_mentioning;
use crate::tests::functional::cluster_session_restart::rmqttd_binary;
use crate::tests::functional::connack_return_codes_v311::copy_dir_recursive;

/// Suite name (also the `--suites` selector).
pub(crate) const SUITE: &str = "bridge_config_reject";

/// Directory the per-case configurations are generated into, relative to the
/// repository root (the broker is always started from there).
const CASE_ROOT: &str = "target/bridge-config-reject";

/// First port of this suite's own block; case *i* uses base + *i* on all three
/// listeners. The fixture copied underneath is the #451 one (MQTT 1915 /
/// gRPC 5395 / HTTP 6075), so nothing here collides with it or with the
/// harness-wide 1883 / 5363 / 6060.
const MQTT_PORT_BASE: u16 = 1925;
const GRPC_PORT_BASE: u16 = 5405;
const HTTP_PORT_BASE: u16 = 6085;
/// Port of the bare MQTT 3.1.1 peer used by the runtime case.
const V3_MOCK_PORT: u16 = 2925;

/// How long a valid configuration may take to become healthy.
const START_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a broker may keep us waiting before we call it "started anyway".
const REJECT_TIMEOUT: Duration = Duration::from_secs(15);
/// `reconnect_interval` pinned by the runtime case
/// (`issue540_ingress_v311_subscribe_refused`), which is also the FIRST sleep of
/// the subscription-retry backoff: 5 s, then 10 s, 20 s, 40 s, …
///
/// It cannot inherit the template's 1 s: with a 1 s base the bounded backoff
/// puts three retries (at 1 s, 3 s, 7 s) inside [`RETRY_WINDOW`], and the bound
/// below is calibrated for 5 s.
const V311_RECONNECT_INTERVAL: &str = "5s";
/// Window in which a refused subscription must not be retried on an unbounded
/// fixed cadence. Counting from the first refusal, a bounded exponential backoff
/// starting at [`V311_RECONNECT_INTERVAL`] retries at 5 s, then 15 s, then 35 s —
/// exactly one attempt inside this window — while the fixed 5 s cadence this case
/// replaced would put a second attempt at 10 s, and a fixed `reconnect_interval`
/// cadence would fill the window entirely.
const RETRY_WINDOW: Duration = Duration::from_secs(12);
/// Retries allowed inside [`RETRY_WINDOW`] *after* the first refusal.
const MAX_RETRIES_IN_WINDOW: usize = 1;

// ---------------------------------------------------------------------------
// Config templates
//
// The templates are the fixture's own shape, reduced to one bridge with one
// entry. `@@REMOTE_TOPIC@@` / `@@LOCAL_TOPIC_FILTER@@` are replaced by a whole
// line, so a case can also OMIT the field (what an operator gets by writing
// `[[bridges.entries]]` and only filling in the other side).
// ---------------------------------------------------------------------------

const INGRESS_TEMPLATE: &str = r#"##--------------------------------------------------------------------
## rmqtt-bridge-ingress-mqtt — generated by rmqtt-test (issue #540)
##--------------------------------------------------------------------
## One bridge, one entry. `remote.topic` is what varies between cases — it is the
## topic FILTER the bridge SUBSCRIBEs with on the peer, so a value the peer
## cannot accept makes the entry deliver nothing forever. The runtime case also
## pins `reconnect_interval`, because the retry backoff starts from it and the
## case's retry bound is derived from that value.

[[bridges]]
enable = true
name = "ing"
client_id_prefix = "prefix"
server = "tcp://127.0.0.1:@@SERVER_PORT@@"
username = "rmqtt_u"
password = "public"

concurrent_client_limit = 1
connect_timeout = "3s"
keepalive = "60s"
reconnect_interval = "@@RECONNECT_INTERVAL@@"
expiry_interval = "5m"

mqtt_ver = "@@MQTT_VER@@"
v4.clean_session = true
v5.clean_start = true
v5.session_expiry_interval = "0s"
v5.receive_maximum = 16
v5.maximum_packet_size = "1M"
v5.topic_alias_maximum = 10

[[bridges.entries]]
remote.qos = 0
@@REMOTE_TOPIC@@
local.qos = 0
local.topic = "ing/plain"
local.retain = false
"#;

const EGRESS_TEMPLATE: &str = r#"##--------------------------------------------------------------------
## rmqtt-bridge-egress-mqtt — generated by rmqtt-test (issue #540)
##--------------------------------------------------------------------
## One bridge, one entry plus an optional extra one. Only `local.topic_filter`
## varies between cases (the positive control appends a second entry, `$SYS/#`,
## which must stay legal): it is the topic FILTER the bridge matches local
## publishes against, so a value that is not a filter is either rejected late
## (a 3 s restart loop) or accepted and silently dead.

[[bridges]]
enable = true
name = "eg"
client_id_prefix = "prefix"
server = "tcp://127.0.0.1:@@SERVER_PORT@@"

concurrent_client_limit = 1
connect_timeout = "3s"
keepalive = "60s"
reconnect_interval = "1s"
message_channel_capacity = 100

mqtt_ver = "@@MQTT_VER@@"
v4.clean_session = true
v5.clean_start = true
v5.session_expiry_interval = "0s"
v5.receive_maximum = 16
v5.maximum_packet_size = "1M"

[[bridges.entries]]
@@LOCAL_TOPIC_FILTER@@
remote.qos = 0
remote.topic = "mirror/${local.topic}"
remote.retain = false
remote.skip_levels = 0
@@EXTRA_ENTRY@@
"#;

/// The egress peer is never reached by these cases; point it at a port of this
/// suite's own block that nothing ever binds, so a leaked retry loop cannot
/// disturb another suite.
const EGRESS_PEER_PORT: u16 = 3999;
/// The ingress peer for the load-time cases (unreachable on purpose, same
/// rationale as [`EGRESS_PEER_PORT`]).
const INGRESS_PEER_PORT: u16 = 2999;

/// Render the ingress plugin config (bridge-level `reconnect_interval` = 1s).
fn ingress_config(mqtt_ver: &str, peer_port: u16, remote_topic_line: &str) -> String {
    ingress_config_with_reconnect(mqtt_ver, peer_port, remote_topic_line, "1s")
}

/// Same, with `reconnect_interval` pinned by the caller.
///
/// The plugin's subscription-retry backoff starts at `reconnect_interval`, so a
/// case that asserts on the retry cadence has to pin this value instead of
/// inheriting the template's 1s — otherwise the assertion silently describes a
/// different policy from the one under test.
fn ingress_config_with_reconnect(
    mqtt_ver: &str,
    peer_port: u16,
    remote_topic_line: &str,
    reconnect_interval: &str,
) -> String {
    INGRESS_TEMPLATE
        .replace("@@MQTT_VER@@", mqtt_ver)
        .replace("@@SERVER_PORT@@", &peer_port.to_string())
        .replace("@@REMOTE_TOPIC@@", remote_topic_line)
        .replace("@@RECONNECT_INTERVAL@@", reconnect_interval)
}

/// Render the egress plugin config.
fn egress_config(local_topic_filter_line: &str) -> String {
    egress_config_with_extra(local_topic_filter_line, "")
}

/// Same, with `extra_entry` appended as a second `[[bridges.entries]]` block.
///
/// Used by the positive control to put a `$`-prefixed filter that must stay
/// legal (`$SYS/#`, published locally by the sys-topic plugin) in front of the
/// validator: the rejection of the three peer-facing forms is a policy about
/// those forms, not about `$` in general.
fn egress_config_with_extra(local_topic_filter_line: &str, extra_entry: &str) -> String {
    EGRESS_TEMPLATE
        .replace("@@MQTT_VER@@", "v5")
        .replace("@@SERVER_PORT@@", &EGRESS_PEER_PORT.to_string())
        .replace("@@LOCAL_TOPIC_FILTER@@", local_topic_filter_line)
        .replace("@@EXTRA_ENTRY@@", extra_entry)
}

/// `remote.topic = "<value>"` as a line of the generated config.
fn remote_topic(value: &str) -> String {
    format!("remote.topic = \"{value}\"")
}

/// `local.topic_filter = "<value>"` as a line of the generated config.
fn local_topic_filter(value: &str) -> String {
    format!("local.topic_filter = \"{value}\"")
}

/// A configuration the broker must refuse to serve, and the evidence of why.
struct RejectSpec {
    /// Case name; also the generated directory name.
    case: &'static str,
    /// Index into this suite's port block.
    index: u16,
    ingress: String,
    egress: String,
    /// What the operator wrote (for the failure message).
    injected: String,
    /// Substrings the refusal MUST carry (compared lower-cased, so the fix is
    /// free to choose its own wording beyond these).
    expect: Vec<String>,
}

/// The whole table, so a registered case can look its own spec up by name.
fn reject_specs() -> Vec<RejectSpec> {
    let legal_ingress = ingress_config("v5", INGRESS_PEER_PORT, &remote_topic("up/plain/#"));
    let legal_egress = egress_config(&local_topic_filter("eg/plain/#"));
    // Inner filter (`x/a`) and group name are both legal; it is the TOTAL length
    // that crosses the cap: 7 (`$share/`) + 65535 + 3 (`x/a`) = 65545 bytes.
    let overlong = format!("$share/{}x/a", "g".repeat(u16::MAX as usize));
    vec![
        RejectSpec {
            case: "issue540_ingress_remote_topic_filter_invalid",
            index: 0,
            ingress: ingress_config("v5", INGRESS_PEER_PORT, &remote_topic("up/#/x")),
            egress: legal_egress.clone(),
            injected: "remote.topic = \"up/#/x\"  (`#` is legal only as the last level, MQTT 5.0 §4.7.1.2)"
                .to_string(),
            expect: vec![
                "bridges[0].entries[0].remote.topic".to_string(),
                "topic filter".to_string(),
                "up/#/x".to_string(),
            ],
        },
        RejectSpec {
            case: "issue540_ingress_remote_topic_empty",
            index: 1,
            // No `remote.topic` line at all: `Remote::topic` has
            // `#[serde(default)]`, so the value becomes "" and the bridge
            // SUBSCRIBEs with an empty filter — which no peer can grant
            // ([MQTT-4.7.3-1] requires at least one character).
            ingress: ingress_config("v5", INGRESS_PEER_PORT, ""),
            egress: legal_egress.clone(),
            injected: "remote.topic omitted (`#[serde(default)]` -> \"\")".to_string(),
            expect: vec!["bridges[0].entries[0].remote.topic".to_string(), "topic filter".to_string()],
        },
        RejectSpec {
            case: "issue540_egress_topic_filter_invalid",
            index: 2,
            ingress: legal_ingress.clone(),
            egress: egress_config(&local_topic_filter("eg/#/x")),
            injected: "local.topic_filter = \"eg/#/x\"".to_string(),
            expect: vec![
                "bridges[0].entries[0].local.topic_filter".to_string(),
                "topic filter".to_string(),
                "eg/#/x".to_string(),
            ],
        },
        RejectSpec {
            case: "issue540_egress_topic_filter_empty",
            index: 3,
            ingress: legal_ingress.clone(),
            // Omitted: `Local::topic_filter` has `#[serde(default)]` -> "".
            // `Topic::from_str("")` SUCCEEDS ("" parses to `[Level::Blank]`), so
            // the bridge installs a trie node under the empty level and never
            // matches a real topic — silently, forever.
            egress: egress_config(""),
            injected: "local.topic_filter omitted (`#[serde(default)]` -> \"\")".to_string(),
            expect: vec!["bridges[0].entries[0].local.topic_filter".to_string(), "topic filter".to_string()],
        },
        RejectSpec {
            case: "issue540_egress_topic_filter_share_prefix",
            index: 4,
            ingress: legal_ingress.clone(),
            // `$share/...` is meaningful for a subscription sent to a PEER; on
            // the local side it is matched literally, so such an entry would only
            // fire for a client publishing to a topic *named* `$share/g/eg/...` —
            // almost certainly an ingress/egress mix-up, not a topic it meant.
            egress: egress_config(&local_topic_filter("$share/g/eg/#")),
            injected: "local.topic_filter = \"$share/g/eg/#\"  (peer-facing shared subscription)".to_string(),
            expect: vec![
                "bridges[0].entries[0].local.topic_filter".to_string(),
                "$share".to_string(),
                "$share/g/eg/#".to_string(),
            ],
        },
        RejectSpec {
            case: "issue540_egress_topic_filter_limit_prefix",
            index: 5,
            ingress: legal_ingress.clone(),
            // The second peer-facing form. `$limit/10/eg/#` parses as an ordinary
            // filter (no `$share` prefix), so only the form check catches it, and
            // like `$share/...` it is matched literally here — it would fire only
            // for a topic *named* `$limit/10/eg/...`.
            egress: egress_config(&local_topic_filter("$limit/10/eg/#")),
            injected: "local.topic_filter = \"$limit/10/eg/#\"  (peer-facing limited subscription)"
                .to_string(),
            expect: vec![
                "bridges[0].entries[0].local.topic_filter".to_string(),
                "limited-subscription".to_string(),
                "$limit/10/eg/#".to_string(),
            ],
        },
        RejectSpec {
            case: "issue540_egress_topic_filter_exclusive_prefix",
            index: 6,
            ingress: legal_ingress.clone(),
            // The third peer-facing form, same reasoning: legal as a filter,
            // meaningless against a local trie.
            egress: egress_config(&local_topic_filter("$exclusive/eg/#")),
            injected: "local.topic_filter = \"$exclusive/eg/#\"  (peer-facing exclusive subscription)"
                .to_string(),
            expect: vec![
                "bridges[0].entries[0].local.topic_filter".to_string(),
                "exclusive-subscription".to_string(),
                "$exclusive/eg/#".to_string(),
            ],
        },
        RejectSpec {
            case: "issue540_ingress_remote_topic_filter_overlong",
            index: 7,
            // The 65535-byte cap of a UTF-8 Encoded String (§1.5.4) applies to
            // the WHOLE filter string, not only to the filter behind the
            // ShareName: an early check that measured just the inner filter
            // would let this through, and the two-byte length prefix the SUBSCRIBE
            // packet relies on cannot express the result.
            ingress: ingress_config("v5", INGRESS_PEER_PORT, &remote_topic(&overlong)),
            egress: legal_egress.clone(),
            injected: "remote.topic = \"$share/<65535 'g's>x/a\"  (65545 bytes: over the \
                       65535-byte UTF-8 Encoded String cap, MQTT 5.0 §1.5.4)"
                .to_string(),
            // The value itself is ~64 KB and the message embeds it; assert on the
            // stable parts of the wording instead.
            expect: vec![
                "bridges[0].entries[0].remote.topic".to_string(),
                "topic filter".to_string(),
                "65535".to_string(),
            ],
        },
    ]
}

// ---------------------------------------------------------------------------
// Generated configuration
// ---------------------------------------------------------------------------

/// One generated case configuration and where its output lands.
struct Generated {
    case_dir: PathBuf,
    config: PathBuf,
    /// stdout/stderr of the broker process (`register plugin failed` panics are
    /// written here).
    process_log: PathBuf,
    /// The broker's own log (`log.to = "file"`), i.e. everything tracing wrote.
    broker_log: PathBuf,
    mqtt_addr: String,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Ports of the block owned by case `index`.
fn ports_for(index: u16) -> (u16, u16, u16) {
    (MQTT_PORT_BASE + index, GRPC_PORT_BASE + index, HTTP_PORT_BASE + index)
}

/// Copy the #451 fixture into `target/bridge-config-reject/<case>/`, point it at
/// this case's port block, and overwrite the two bridge configs.
///
/// The copy keeps the fixture's `log.to = "file"` (the ONLY way a case can read
/// what RMQTT logged: the harness and this suite both start the broker with a
/// piped, never-drained stdio) and its `plugins.default_startups`, so the plugin
/// that is NOT under test still loads with a legal config and the rejection can
/// only come from the field the case injected.
///
/// `egress` is `None` when the case does not need the egress plugin at all (the
/// runtime case drives the ingress side only): the plugin is then dropped from
/// `default_startups`, which keeps its "the peer is unreachable" WARNs out of
/// the log the case is about to read.
fn generate(case: &str, index: u16, ingress: &str, egress: Option<&str>) -> anyhow::Result<Generated> {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("configs").join("bridge-egress-mqtt");
    let case_dir = repo_root().join(CASE_ROOT).join(case);
    // A stale copy from an earlier run must not survive: the assertions read
    // this tree's files.
    let _ = std::fs::remove_dir_all(&case_dir);
    copy_dir_recursive(&src, &case_dir)?;

    let logs = case_dir.join("logs");
    std::fs::create_dir_all(&logs)?;
    let abs_plugins = std::fs::canonicalize(case_dir.join("plugins"))?.to_string_lossy().replace('\\', "/");
    let abs_logs = std::fs::canonicalize(&logs)?.to_string_lossy().replace('\\', "/");

    let (mqtt, grpc, http) = ports_for(index);
    let main_cfg = case_dir.join("rmqtt.toml");
    let mut text = std::fs::read_to_string(&main_cfg)?
        .replace("@@PLUGINS_DIR@@", &abs_plugins)
        .replace("@@LOG_DIR@@", &abs_logs)
        .replace("127.0.0.1:1915", &format!("127.0.0.1:{mqtt}"))
        .replace("0.0.0.0:5395", &format!("0.0.0.0:{grpc}"))
        .replace("bridge-egress-mqtt.log", "broker.log");
    if egress.is_none() {
        // Leaving an empty line inside the TOML array is valid.
        text = text.replace("\"rmqtt-bridge-egress-mqtt\",", "");
    }
    std::fs::write(&main_cfg, text)?;

    let http_cfg = case_dir.join("plugins").join("rmqtt-http-api.toml");
    let text = std::fs::read_to_string(&http_cfg)?.replace("0.0.0.0:6075", &format!("0.0.0.0:{http}"));
    std::fs::write(&http_cfg, text)?;

    std::fs::write(case_dir.join("plugins").join("rmqtt-bridge-ingress-mqtt.toml"), ingress)?;
    if let Some(egress) = egress {
        std::fs::write(case_dir.join("plugins").join("rmqtt-bridge-egress-mqtt.toml"), egress)?;
    }

    Ok(Generated {
        config: main_cfg,
        process_log: case_dir.join("process.log"),
        broker_log: logs.join("broker.log"),
        mqtt_addr: format!("127.0.0.1:{mqtt}"),
        case_dir,
    })
}

/// A broker process owned by a case: killed on drop, so an early return cannot
/// leak a process (or keep a port of this suite's block bound).
struct OwnedBroker {
    child: Child,
    case_dir: PathBuf,
}

impl OwnedBroker {
    fn spawn(gen: &Generated) -> anyhow::Result<Self> {
        let binary = rmqttd_binary();
        if !binary.exists() {
            return Err(anyhow::anyhow!(
                "rmqttd binary not found at {binary:?}; build it first (cargo build -p rmqttd)"
            ));
        }
        // A residual process from an earlier run would make the broker fail for
        // a reason that has nothing to do with the configuration under test.
        if !port_free_sync(&gen.mqtt_addr) {
            return Err(anyhow::anyhow!(
                "{} is already in use; a residual broker (or another process) holds this suite's \
                 port block — stop it before re-running",
                gen.mqtt_addr
            ));
        }

        let stdout = std::fs::File::create(&gen.process_log)?;
        let stderr = stdout.try_clone()?;
        // `-f` points at the FILE (a directory is silently ignored by
        // rmqtt-conf, and the built-in defaults would be used instead). The
        // working directory stays the repository root, which is what the
        // fixture's deliberately partial config relies on.
        let child = Command::new(binary)
            .arg("-f")
            .arg(&gen.config)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()?;
        Ok(Self { child, case_dir: gen.case_dir.clone() })
    }

    /// Everything the broker said, wherever it said it: the process output
    /// (panics, tracing's console layer) and the `log.to = "file"` target.
    fn output(&self, gen: &Generated) -> String {
        let mut out = std::fs::read_to_string(&gen.process_log).unwrap_or_default();
        out.push('\n');
        out.push_str(&std::fs::read_to_string(&gen.broker_log).unwrap_or_default());
        out
    }

    fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for OwnedBroker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = &self.case_dir;
    }
}

/// Outcome of watching a freshly started broker.
enum Watch {
    /// It became healthy: the configuration was accepted.
    Healthy,
    /// It stopped before serving.
    Exited(Option<std::process::ExitStatus>),
    /// Neither, within the timeout.
    Silent,
}

/// Wait until the broker either serves or dies.
fn watch_broker(broker: &mut OwnedBroker, mqtt_addr: &str, timeout: Duration) -> Watch {
    let deadline = Instant::now() + timeout;
    loop {
        if !broker.is_alive() {
            return Watch::Exited(broker.child.try_wait().ok().flatten());
        }
        if health_check_sync(mqtt_addr, Duration::from_millis(300)) {
            return Watch::Healthy;
        }
        if Instant::now() >= deadline {
            return Watch::Silent;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Tail of the process output, for failure messages.
fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    all[start..].join("\n")
}

// ---------------------------------------------------------------------------
// Case bodies
// ---------------------------------------------------------------------------

/// A configuration the broker cannot serve must be refused while it is loaded,
/// naming the field and the entry.
///
/// The contract is deliberately mechanism-agnostic: the broker may panic, exit
/// or hang, as long as it never starts serving and says why. Both the process
/// output and the `log.to = "file"` target are searched, so the current
/// `expect("register plugin failed")` path and any future `?`-based one are
/// accepted alike.
fn run_reject_case(spec: &RejectSpec) -> anyhow::Result<()> {
    let gen = generate(spec.case, spec.index, &spec.ingress, Some(&spec.egress))?;
    let mut broker = OwnedBroker::spawn(&gen)?;

    let watch = watch_broker(&mut broker, &gen.mqtt_addr, REJECT_TIMEOUT);
    let output = broker.output(&gen);
    let haystack = output.to_lowercase();

    match watch {
        Watch::Healthy => Err(anyhow::anyhow!(
            "the broker started and served on {} with a bridge entry whose topic FILTER field is \
             not a filter — the entry now delivers nothing forever while the operator sees a \
             healthy bridge (issue #540). Injected configuration: {} ({}). A topic filter must be \
             at least one character long, must be at most 65535 bytes, and `#` is legal only as \
             the last level; the peer-facing `$share/<group>/`, `$limit/<n>/` and `$exclusive/` \
             forms belong in an ingress `remote.topic`, not in a local filter. Nothing in the \
             broker's own output mentions the field: this is what the configuration load is \
             supposed to refuse.",
            gen.mqtt_addr,
            spec.injected,
            spec.case
        )),
        Watch::Exited(_) | Watch::Silent => {
            let missing: Vec<&str> = spec
                .expect
                .iter()
                .filter(|needle| !haystack.contains(&needle.to_lowercase()))
                .map(|s| s.as_str())
                .collect();
            if missing.is_empty() {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "the broker refused to serve with an illegal topic filter ({}), but nothing in \
                     its output names the offending field: missing {missing:?}.\nProcess/broker \
                     output tail:\n{}",
                    spec.injected,
                    tail(&output, 25)
                ))
            }
        }
    }
}

/// The positive control for the whole suite: a configuration whose filters are
/// all legal — including an ingress `$share/<group>/<filter>`, which is what the
/// peer needs and what a filter validator must NOT reject, and an egress
/// `$SYS/#`, which is `$`-prefixed but not peer-facing (`$SYS/...` is published
/// locally by the sys-topic plugin) and therefore must NOT be rejected either —
/// must start and serve. Without it, "the broker always refuses to start" would
/// satisfy every negative case above, and a rule that grew from "the three
/// peer-facing forms" into "anything starting with `$`" would go unnoticed.
fn run_valid_config_case() -> anyhow::Result<()> {
    let specs = reject_specs();
    let index = specs.len() as u16;
    let ingress = ingress_config("v5", INGRESS_PEER_PORT, &remote_topic("$share/g/up/a/#"));
    let egress = egress_config_with_extra(
        &local_topic_filter("eg/plain/#"),
        "[[bridges.entries]]\nlocal.topic_filter = \"$SYS/#\"\nremote.qos = 0\n\
         remote.topic = \"mirror/${local.topic}\"\nremote.retain = false\nremote.skip_levels = 0\n",
    );

    let gen = generate("issue540_valid_filters_start", index, &ingress, Some(&egress))?;
    let mut broker = OwnedBroker::spawn(&gen)?;

    match watch_broker(&mut broker, &gen.mqtt_addr, START_TIMEOUT) {
        Watch::Healthy => Ok(()),
        Watch::Exited(status) => Err(anyhow::anyhow!(
            "a configuration whose topic filters are ALL legal was refused (broker exited: \
             {status:?}): a filter validator must accept `$share/g/up/a/#` (peer-facing, ingress \
             side), `eg/plain/#` and `$SYS/#` (a `$`-prefixed but LOCAL topic, which is NOT one of \
             the peer-facing forms). Output tail:\n{}",
            tail(&broker.output(&gen), 25)
        )),
        Watch::Silent => Err(anyhow::anyhow!(
            "a configuration whose topic filters are ALL legal did not become healthy within \
             {START_TIMEOUT:?}. Output tail:\n{}",
            tail(&broker.output(&gen), 25)
        )),
    }
}

// ---------------------------------------------------------------------------
// Minimal MQTT 3.1.1 peer (the runtime case)
// ---------------------------------------------------------------------------

/// A bare MQTT 3.1.1 broker: answers CONNECT with CONNACK(accepted) and every
/// SUBSCRIBE with `0x80` (Failure), counting the attempts per filter.
///
/// It exists because the ingress plugin's v3.1.1 path (`src/v4.rs`) is a
/// different code path from the v5 one, needs `mqtt_ver = "v4"`, and — as the
/// issue points out — reports the very same failure at a *lower* level than
/// `src/v5.rs` does (`info!` vs `warn!`), without the SUBACK reason.
struct V3Mock {
    attempts: Arc<AtomicUsize>,
    _accept: tokio::task::JoinHandle<()>,
}

impl V3Mock {
    async fn spawn(port: u16) -> anyhow::Result<Self> {
        let listener = {
            let deadline = Instant::now() + Duration::from_secs(15);
            loop {
                match TcpListener::bind(("127.0.0.1", port)).await {
                    Ok(listener) => break listener,
                    Err(e) if Instant::now() >= deadline => {
                        return Err(anyhow::anyhow!("the MQTT 3.1.1 mock cannot bind 127.0.0.1:{port}: {e}"));
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
                }
            }
        };
        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = attempts.clone();
        let accept = tokio::spawn(async move {
            loop {
                let Ok((stream, _peer)) = listener.accept().await else { break };
                let counter = counter.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve_v3(stream, counter).await {
                        tracing::debug!("v3 mock connection ended: {e:#}");
                    }
                });
            }
        });
        Ok(Self { attempts, _accept: accept })
    }

    fn attempts(&self) -> usize {
        self.attempts.load(Ordering::SeqCst)
    }
}

async fn serve_v3(stream: tokio::net::TcpStream, attempts: Arc<AtomicUsize>) -> anyhow::Result<()> {
    let mut framed = Framed::new(stream, V3Codec::new(0));

    let first = framed.next().await.ok_or_else(|| anyhow::anyhow!("peer closed before CONNECT"))?;
    let (packet, _) = first.map_err(|e| anyhow::anyhow!("decode error while reading CONNECT: {e}"))?;
    if !matches!(packet, PacketV3::Connect(_)) {
        return Err(anyhow::anyhow!("expected CONNECT"));
    }
    framed
        .send(PacketV3::ConnectAck(ConnectAckV3 {
            return_code: ConnectAckReasonV3::ConnectionAccepted,
            session_present: false,
        }))
        .await
        .map_err(|e| anyhow::anyhow!("encode error while sending CONNACK: {e}"))?;

    while let Some(incoming) = framed.next().await {
        let (packet, _) = incoming.map_err(|e| anyhow::anyhow!("decode error: {e}"))?;
        match packet {
            PacketV3::Subscribe { packet_id, topic_filters } => {
                // Every filter is refused: this peer exists to exercise the
                // "the peer will not have it" path, not to deliver messages.
                attempts.fetch_add(topic_filters.len(), Ordering::SeqCst);
                let status = topic_filters.iter().map(|_| SubscribeReturnCode::Failure).collect();
                framed
                    .send(PacketV3::SubscribeAck { packet_id, status })
                    .await
                    .map_err(|e| anyhow::anyhow!("encode error while sending SUBACK: {e}"))?;
            }
            PacketV3::PingRequest => {
                framed
                    .send(PacketV3::PingResponse)
                    .await
                    .map_err(|e| anyhow::anyhow!("encode error: {e}"))?;
            }
            PacketV3::Disconnect => break,
            _ => {}
        }
    }
    Ok(())
}

/// A subscribed-but-refused entry must be attributable and must not be retried
/// on a fixed cadence as if the failure never clears.
///
/// On MQTT 3.1.1 the peer answers SUBACK `0x80` (Failure) to a filter it will
/// not grant, and the protocol carries no reason: a permanent denial and a
/// transient one look identical. The plugin used to sleep 5 s and retry —
/// forever — while logging the event at `info!` (`src/v4.rs`), where the v5 path
/// logs `warn!` with the reason (`src/v5.rs`). Both halves were wrong in the same
/// way: a *permanent* configuration/authorization error dressed up as a transient
/// one, at a level an operator filtering for problems never sees.
///
/// The fix answers with a bounded exponential backoff (and, on v5, gives up at
/// once on a permanent reason code). What must never come back is a fixed 5 s
/// cadence that keeps the log quiet and the entry dead for the lifetime of the
/// process, so this case allows at most [`MAX_RETRIES_IN_WINDOW`] further attempt
/// inside [`RETRY_WINDOW`].
///
/// The backoff starts at the bridge's `reconnect_interval`, which is why the
/// fixture below pins it to [`V311_RECONNECT_INTERVAL`] rather than inheriting the
/// template's 1 s — the bound above only means what it says for that base.
async fn ingress_v311_subscribe_refused() -> anyhow::Result<()> {
    let index = reject_specs().len() as u16 + 1;
    let filter = "up/plain/#";
    let ingress =
        ingress_config_with_reconnect("v4", V3_MOCK_PORT, &remote_topic(filter), V311_RECONNECT_INTERVAL);
    // No egress plugin: this case reads the log, and an egress bridge pointing
    // at an unreachable peer would only add WARNs it does not care about.
    let gen = generate("issue540_ingress_v311_subscribe_refused", index, &ingress, None)?;

    let mock = V3Mock::spawn(V3_MOCK_PORT).await?;
    let mut broker = OwnedBroker::spawn(&gen)?;
    match watch_broker(&mut broker, &gen.mqtt_addr, START_TIMEOUT) {
        Watch::Healthy => {}
        Watch::Exited(status) => {
            return Err(anyhow::anyhow!(
                "the broker exited ({status:?}) with a legal ingress configuration; output tail:\n{}",
                tail(&broker.output(&gen), 20)
            ));
        }
        Watch::Silent => {
            return Err(anyhow::anyhow!(
                "the broker did not become healthy within {START_TIMEOUT:?}; output tail:\n{}",
                tail(&broker.output(&gen), 20)
            ));
        }
    }

    // Control arm: the refusal must actually happen on the wire, otherwise
    // "few attempts" would also hold when the bridge never subscribed at all.
    let deadline = Instant::now() + Duration::from_secs(20);
    while mock.attempts() == 0 {
        if Instant::now() >= deadline {
            return Err(anyhow::anyhow!(
                "control arm: the ingress bridge (mqtt_ver = \"v4\", filter {filter:?}) never \
                 SUBSCRIBEd to the MQTT 3.1.1 mock within 20s, so the retry assertions below \
                 would be vacuous"
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let after_first = mock.attempts();
    tokio::time::sleep(RETRY_WINDOW).await;
    let after_window = mock.attempts();
    let retries = after_window.saturating_sub(after_first);
    let log = std::fs::read_to_string(&gen.broker_log).unwrap_or_default();

    // Both halves are reported together: today the v3.1.1 path fails on both, and
    // a fix that only raises the level must still be told about the cadence.
    let mut problems: Vec<String> = Vec::new();

    if !warn_or_error_record_mentioning(&log, filter) {
        problems.push(format!(
            "the refusal is invisible to an operator: {after_window} SUBSCRIBE attempt(s) for \
             {filter:?} were answered with SUBACK 0x80, but the broker log carries no WARN/ERROR \
             record mentioning the refused filter — the v3.1.1 path reports this at `info!` \
             without the SUBACK reason, while the v5 path reports it at `warn!` with one"
        ));
    }

    if retries > MAX_RETRIES_IN_WINDOW {
        problems.push(format!(
            "a subscription the peer will never grant was retried {retries} time(s) in \
             {RETRY_WINDOW:?} (allowed: {MAX_RETRIES_IN_WINDOW}, with reconnect_interval = \
             {V311_RECONNECT_INTERVAL} as the backoff base): a permanent failure is being \
             retried on a fixed cadence that hides it from the operator, while the entry delivers \
             nothing — see the `subscribe` loop of rmqtt-bridge-ingress-mqtt/src/v4.rs \
             (src/v5.rs has the same shape, covered by the bridge_egress_mqtt suite)"
        ));
    }

    if problems.is_empty() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("{}\nLog tail:\n{}", problems.join("\n- "), tail(&log, 20)))
    }
}

// ---------------------------------------------------------------------------
// Registered cases
// ---------------------------------------------------------------------------

/// One thin case type per entry of [`reject_specs`].
macro_rules! reject_case {
    ($ty:ident, $case:literal) => {
        pub struct $ty;

        impl TestCase for $ty {
            fn name(&self) -> &str {
                $case
            }

            fn execute(&self, _ctx: &mut TestContext) -> TestResult {
                let start = Instant::now();
                let spec = reject_specs()
                    .into_iter()
                    .find(|s| s.case == $case)
                    .expect("a registered reject case must exist in the table");
                match run_reject_case(&spec) {
                    Ok(()) => TestResult::passed($case, SUITE, start.elapsed()),
                    Err(e) => TestResult::failed($case, SUITE, start.elapsed(), format!("{e:#}")),
                }
            }

            fn timeout(&self) -> Duration {
                // The broker plus the 15 s "did it start anyway" window.
                Duration::from_secs(90)
            }
        }
    };
}

/// A case whose body needs a tokio runtime; runs it on a 32 MiB stack thread
/// (the established pattern for cases that build clients).
fn run_async_case<F>(name: &str, body: F) -> TestResult
where
    F: FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + Send>>
        + Send
        + 'static,
{
    let start = Instant::now();
    let spawned = std::thread::Builder::new()
        .name("bridge-config-reject".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let rt = tokio::runtime::Runtime::new()
                .map_err(|e| anyhow::anyhow!("failed to create a tokio runtime: {e}"))?;
            rt.block_on(body())
        });

    match spawned {
        Ok(handle) => match handle.join() {
            Ok(Ok(())) => TestResult::passed(name, SUITE, start.elapsed()),
            Ok(Err(e)) => TestResult::failed(name, SUITE, start.elapsed(), format!("{e:#}")),
            Err(_) => TestResult::failed(name, SUITE, start.elapsed(), "test thread panicked".to_string()),
        },
        Err(e) => {
            TestResult::error(name, SUITE, start.elapsed(), format!("failed to spawn a test thread: {e}"))
        }
    }
}

reject_case!(Issue540IngressRemoteTopicFilterInvalidTest, "issue540_ingress_remote_topic_filter_invalid");
reject_case!(Issue540IngressRemoteTopicEmptyTest, "issue540_ingress_remote_topic_empty");
reject_case!(Issue540EgressTopicFilterInvalidTest, "issue540_egress_topic_filter_invalid");
reject_case!(Issue540EgressTopicFilterEmptyTest, "issue540_egress_topic_filter_empty");
reject_case!(Issue540EgressTopicFilterSharePrefixTest, "issue540_egress_topic_filter_share_prefix");
reject_case!(Issue540EgressTopicFilterLimitPrefixTest, "issue540_egress_topic_filter_limit_prefix");
reject_case!(Issue540EgressTopicFilterExclusivePrefixTest, "issue540_egress_topic_filter_exclusive_prefix");
reject_case!(Issue540IngressRemoteTopicFilterOverlongTest, "issue540_ingress_remote_topic_filter_overlong");

/// Positive control: legal filters (including an ingress `$share/...`) must
/// start.
pub struct Issue540ValidFiltersStartTest;

impl TestCase for Issue540ValidFiltersStartTest {
    fn name(&self) -> &str {
        "issue540_valid_filters_start"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        let start = Instant::now();
        match run_valid_config_case() {
            Ok(()) => TestResult::passed(self.name(), SUITE, start.elapsed()),
            Err(e) => TestResult::failed(self.name(), SUITE, start.elapsed(), format!("{e:#}")),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(90)
    }
}

/// Runtime half on the MQTT 3.1.1 path: a refused subscription must be visible
/// and must not be retried forever.
pub struct Issue540IngressV311SubscribeRefusedTest;

impl TestCase for Issue540IngressV311SubscribeRefusedTest {
    fn name(&self) -> &str {
        "issue540_ingress_v311_subscribe_refused"
    }

    fn execute(&self, _ctx: &mut TestContext) -> TestResult {
        run_async_case(self.name(), || Box::pin(ingress_v311_subscribe_refused()))
    }

    fn timeout(&self) -> Duration {
        // Broker start + the 20 s control arm + the 12 s retry window.
        Duration::from_secs(120)
    }
}
