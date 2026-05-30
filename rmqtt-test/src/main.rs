//! mqtt_harness - RMQTT Industrial-Grade Test Harness
//!
//! Entry point for the RMQTT broker test and benchmarking system.
//! Supports functional (v3/v311/v5), stress, and chaos test suites.

#![deny(unsafe_code)]
#![allow(dead_code)]

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

mod broker;
mod chaos;
mod framework;
mod mqtt;
mod report;
mod tests;
mod transport;

use broker::BrokerProcess;
use framework::context::{TestConfig, TestContext};
use framework::scheduler::TestScheduler;
use framework::suite::TestSuite;
use report::{write_detail_log, ConsoleReporter, HtmlReporter, JsonReporter};

#[derive(Debug, Parser)]
#[command(name = "mqtt_harness", about = "RMQTT Industrial-Grade Test Harness")]
struct Opt {
    /// Broker address (default: 127.0.0.1:1883)
    #[arg(short, long, default_value = "127.0.0.1:1883")]
    addr: String,

    /// Path to rmqttd binary
    #[arg(short, long)]
    binary: Option<String>,

    /// Path to rmqtt.toml config
    #[arg(short = 'c', long)]
    config: Option<String>,

    /// Workspace root (for finding target/release/rmqttd)
    #[arg(long)]
    workspace: Option<String>,

    /// Run only specific test suites (functional_v3, functional_v311, functional_v5, stress, chaos)
    #[arg(short, long)]
    suites: Vec<String>,

    /// Number of parallel workers
    #[arg(short, long, default_value = "4")]
    workers: usize,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    /// Output JSON report to file
    #[arg(long)]
    json: Option<String>,

    /// Output HTML report to file
    #[arg(long)]
    html: Option<String>,

    /// Do not start/stop broker (assume it's already running)
    #[arg(long)]
    no_broker: bool,

    /// Stress test client count
    #[arg(long, default_value = "100")]
    stress_clients: usize,

    /// Chaos test iterations
    #[arg(long, default_value = "5")]
    chaos_iterations: usize,

    /// Write detailed test log to file (includes packet traces for failed tests)
    #[arg(long, default_value = "test-detail.log")]
    log_file: String,
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn test_default() {
        let opt = Opt::parse_from(["mqtt_harness"]);
        assert_eq!(opt.addr, "127.0.0.1:1883");
        assert!(opt.binary.is_none());
        assert!(opt.config.is_none());
        assert!(opt.workspace.is_none());
        assert!(opt.suites.is_empty());
        assert_eq!(opt.workers, 4);
        assert!(!opt.verbose);
        assert!(opt.json.is_none());
        assert!(opt.html.is_none());
        assert!(!opt.no_broker);
        assert_eq!(opt.stress_clients, 100);
        assert_eq!(opt.chaos_iterations, 5);
        assert_eq!(opt.log_file, "test-detail.log");
    }

    #[test]
    fn test_addr_short() {
        let opt = Opt::parse_from(["mqtt_harness", "-a", "10.0.0.1:1883"]);
        assert_eq!(opt.addr, "10.0.0.1:1883");
    }

    #[test]
    fn test_addr_long() {
        let opt = Opt::parse_from(["mqtt_harness", "--addr", "0.0.0.0:1883"]);
        assert_eq!(opt.addr, "0.0.0.0:1883");
    }

    #[test]
    fn test_binary_short() {
        let opt = Opt::parse_from(["mqtt_harness", "-b", "./target/release/rmqttd"]);
        assert_eq!(opt.binary.as_deref(), Some("./target/release/rmqttd"));
    }

    #[test]
    fn test_binary_long() {
        let opt = Opt::parse_from(["mqtt_harness", "--binary", "/usr/bin/rmqttd"]);
        assert_eq!(opt.binary.as_deref(), Some("/usr/bin/rmqttd"));
    }

    #[test]
    fn test_config_short() {
        let opt = Opt::parse_from(["mqtt_harness", "-c", "config/rmqtt.toml"]);
        assert_eq!(opt.config.as_deref(), Some("config/rmqtt.toml"));
    }

    #[test]
    fn test_config_long() {
        let opt = Opt::parse_from(["mqtt_harness", "--config", "/etc/rmqtt/rmqtt.toml"]);
        assert_eq!(opt.config.as_deref(), Some("/etc/rmqtt/rmqtt.toml"));
    }

    #[test]
    fn test_workspace() {
        let opt = Opt::parse_from(["mqtt_harness", "--workspace", "/home/user/rmqtt"]);
        assert_eq!(opt.workspace.as_deref(), Some("/home/user/rmqtt"));
    }

    #[test]
    fn test_suites_short() {
        let opt = Opt::parse_from(["mqtt_harness", "-s", "functional_v3", "-s", "functional_v5"]);
        assert_eq!(opt.suites, vec!["functional_v3".to_string(), "functional_v5".to_string()]);
    }

    #[test]
    fn test_suites_long() {
        let opt = Opt::parse_from(["mqtt_harness", "--suites", "stress", "--suites", "chaos"]);
        assert_eq!(opt.suites, vec!["stress".to_string(), "chaos".to_string()]);
    }

    #[test]
    fn test_workers_short() {
        let opt = Opt::parse_from(["mqtt_harness", "-w", "8"]);
        assert_eq!(opt.workers, 8);
    }

    #[test]
    fn test_workers_long() {
        let opt = Opt::parse_from(["mqtt_harness", "--workers", "16"]);
        assert_eq!(opt.workers, 16);
    }

    #[test]
    fn test_verbose_short() {
        let opt = Opt::parse_from(["mqtt_harness", "-v"]);
        assert!(opt.verbose);
    }

    #[test]
    fn test_verbose_long() {
        let opt = Opt::parse_from(["mqtt_harness", "--verbose"]);
        assert!(opt.verbose);
    }

    #[test]
    fn test_json() {
        let opt = Opt::parse_from(["mqtt_harness", "--json", "report.json"]);
        assert_eq!(opt.json.as_deref(), Some("report.json"));
    }

    #[test]
    fn test_html() {
        let opt = Opt::parse_from(["mqtt_harness", "--html", "report.html"]);
        assert_eq!(opt.html.as_deref(), Some("report.html"));
    }

    #[test]
    fn test_no_broker() {
        let opt = Opt::parse_from(["mqtt_harness", "--no-broker"]);
        assert!(opt.no_broker);
    }

    #[test]
    fn test_stress_clients() {
        let opt = Opt::parse_from(["mqtt_harness", "--stress-clients", "500"]);
        assert_eq!(opt.stress_clients, 500);
    }

    #[test]
    fn test_chaos_iterations() {
        let opt = Opt::parse_from(["mqtt_harness", "--chaos-iterations", "10"]);
        assert_eq!(opt.chaos_iterations, 10);
    }

    #[test]
    fn test_log_file() {
        let opt = Opt::parse_from(["mqtt_harness", "--log-file", "custom.log"]);
        assert_eq!(opt.log_file, "custom.log");
    }

    #[test]
    fn test_all_options() {
        let opt = Opt::parse_from([
            "mqtt_harness",
            "-a", "0.0.0.0:1883",
            "-b", "rmqttd",
            "-c", "myconfig.toml",
            "--workspace", "/opt/rmqtt",
            "-s", "functional_v311",
            "-s", "stress",
            "-w", "10",
            "-v",
            "--json", "out.json",
            "--html", "out.html",
            "--no-broker",
            "--stress-clients", "200",
            "--chaos-iterations", "20",
            "--log-file", "detail.log",
        ]);
        assert_eq!(opt.addr, "0.0.0.0:1883");
        assert_eq!(opt.binary.as_deref(), Some("rmqttd"));
        assert_eq!(opt.config.as_deref(), Some("myconfig.toml"));
        assert_eq!(opt.workspace.as_deref(), Some("/opt/rmqtt"));
        assert_eq!(opt.suites, vec!["functional_v311".to_string(), "stress".to_string()]);
        assert_eq!(opt.workers, 10);
        assert!(opt.verbose);
        assert_eq!(opt.json.as_deref(), Some("out.json"));
        assert_eq!(opt.html.as_deref(), Some("out.html"));
        assert!(opt.no_broker);
        assert_eq!(opt.stress_clients, 200);
        assert_eq!(opt.chaos_iterations, 20);
        assert_eq!(opt.log_file, "detail.log");
    }
}

fn main() {
    let opt = Opt::parse();

    // Initialize logging: console (info+) + trace file (debug+ with packet traces)
    let trace_log_path = PathBuf::from("test-trace.log");
    init_logging(&trace_log_path, opt.verbose);

    info!("mqtt_harness starting...");
    info!("Broker address: {}", opt.addr);
    info!("Detail log: {} | Packet trace: test-trace.log", opt.log_file);

    // Configure test context
    let test_config = TestConfig {
        broker_addr: opt.addr.clone(),
        parallel_workers: opt.workers,
        verbose: opt.verbose,
        default_test_timeout: Duration::from_secs(60),
        connect_timeout: Duration::from_secs(10),
    };

    // Start broker if needed (synchronous - BrokerProcess uses std::process)
    let broker = if !opt.no_broker {
        let mut broker = if let Some(ref binary) = opt.binary {
            BrokerProcess::with_config(
                PathBuf::from(binary),
                opt.addr.clone(),
                opt.config.as_ref().map(PathBuf::from),
            )
        } else {
            let workspace = opt.workspace.as_ref().map(PathBuf::from);
            BrokerProcess::new(workspace)
        };

        match broker.start() {
            Ok(()) => {
                info!("Broker started successfully");
                Some(broker)
            }
            Err(e) => {
                error!("Failed to start broker: {}", e);
                error!("Hint: Use --no-broker if the broker is already running");
                std::process::exit(1);
            }
        }
    } else {
        info!("Using external broker at {}", opt.addr);
        None
    };

    // Build test context
    let mut ctx = if let Some(b) = broker {
        TestContext::with_broker(test_config, b)
    } else {
        TestContext::new(test_config)
    };

    // Build test suites
    let suites = build_suites(&opt);

    // Run tests (synchronous - each test creates its own runtime internally)
    let mut scheduler = TestScheduler::new();
    scheduler.run(suites, &mut ctx);

    // Generate reports
    let results = scheduler.results();
    let summary = scheduler.summary();

    ConsoleReporter::report(results, &summary);

    if let Some(ref json_path) = opt.json {
        let report = JsonReporter::generate("rmqtt-test", results, &summary);
        match JsonReporter::write_to_file(&report, PathBuf::from(json_path).as_path()) {
            Ok(()) => info!("JSON report written to {}", json_path),
            Err(e) => error!("Failed to write JSON report: {}", e),
        }
    }

    if let Some(ref html_path) = opt.html {
        match HtmlReporter::generate(results, &summary, PathBuf::from(html_path).as_path()) {
            Ok(()) => info!("HTML report written to {}", html_path),
            Err(e) => error!("Failed to write HTML report: {}", e),
        }
    }

    // Write detailed test log (always)
    let detail_log_path = PathBuf::from(&opt.log_file);
    match write_detail_log(&detail_log_path, results, &summary) {
        Ok(()) => info!("Detail log written to {}", opt.log_file),
        Err(e) => error!("Failed to write detail log: {}", e),
    }

    // Exit code
    if summary.failed > 0 || summary.errors > 0 {
        std::process::exit(1);
    }
}

/// Build test suites based on CLI options
fn build_suites(opt: &Opt) -> Vec<TestSuite> {
    let run_all = opt.suites.is_empty();
    let should_run = |name: &str| run_all || opt.suites.iter().any(|s| s == name);

    let mut suites = Vec::new();

    if should_run("functional_v3") {
        suites.push(build_functional_v3_suite());
    }

    if should_run("functional_v311") {
        suites.push(build_functional_v311_suite());
    }

    if should_run("functional_v5") {
        suites.push(build_functional_v5_suite());
    }

    if should_run("stress") {
        suites.push(build_stress_suite(opt.stress_clients));
    }

    if should_run("chaos") {
        suites.push(build_chaos_suite(opt.chaos_iterations));
    }

    suites
}

fn build_functional_v3_suite() -> TestSuite {
    use tests::functional::connect_v3::*;
    use tests::functional::pubsub_v3::*;

    let mut suite = TestSuite::new("functional_v3");
    suite.add(ConnectV3Test);
    suite.add(PubSubV3Qos0Test);
    suite
}

fn build_functional_v311_suite() -> TestSuite {
    use tests::functional::auth_v311::*;
    use tests::functional::boundary::*;
    use tests::functional::connect_v311::*;
    use tests::functional::dollar_topics::*;
    use tests::functional::keepalive::*;
    use tests::functional::last_will::*;
    use tests::functional::multi_topic::*;
    use tests::functional::protocol_error::*;
    use tests::functional::pubsub_v311::*;
    use tests::functional::session_v311::*;
    use tests::functional::shared_subscription::*;
    use tests::functional::wildcard::*;
    use tests::functional::wildcard_reject::*;

    let mut suite = TestSuite::new("functional_v311");
    suite.add(ConnectV311Test);
    suite.add(ConnectEmptyClientIdTest);
    suite.add(MultipleConnectionsTest::default());
    suite.add(PubSubV311Qos0Test);
    suite.add(PubSubV311Qos1Test);
    suite.add(PubSubV311Qos2Test);
    suite.add(RetainV311Test);
    suite.add(UnsubscribeV311Test);
    suite.add(WildcardPlusTest);
    suite.add(WildcardHashTest);
    // Last Will and Testament
    suite.add(LastWillV311Test);
    suite.add(LastWillV311CleanTest);
    suite.add(LastWillUncleanTest);
    // KeepAlive / PING
    suite.add(KeepAliveV311Test);
    suite.add(KeepAliveTimeoutTest);
    // Shared subscriptions
    suite.add(SharedSubV311Test);
    // Authentication edge cases
    suite.add(AuthEmptyClientIdFailTest);
    suite.add(AuthConnectDisconnectSequenceTest);
    // Boundary tests
    suite.add(MaxClientIdTest);
    suite.add(LongTopicTest);
    suite.add(EmptyPayloadTest);
    suite.add(LargePayloadTest);
    suite.add(SpecialCharsTopicTest);
    suite.add(RapidSubscribeTest);
    // Multi-topic
    suite.add(MultiTopicSubscribeV311Test);
    suite.add(OverlappingSubscriptionsTest);
    suite.add(MessageOrderingTest);
    // Session persistence (v311)
    suite.add(CleanSessionFalseTest);
    suite.add(OfflineQueueV311Test);
    // Wildcard publish rejection
    suite.add(PublishWildcardRejectTest);
    // Dollar topics ($SYS)
    suite.add(DollarTopicsTest);
    // QoS 2 duplicate detection
    suite.add(Qos2DuplicateDetectionTest);
    // Protocol error edge cases
    suite.add(InvalidProtocolVersionTest);
    suite.add(EmptyTopicFilterTest);
    suite
}

fn build_functional_v5_suite() -> TestSuite {
    use tests::functional::assigned_clientid_v5::*;
    use tests::functional::connect_v5::*;
    use tests::functional::disconnect_reason_v5::*;
    use tests::functional::dollar_topics::*;
    use tests::functional::flow_control_v5::*;
    use tests::functional::keepalive::*;
    use tests::functional::last_will::*;
    use tests::functional::max_packet_size_v5::*;
    use tests::functional::no_local_v5::*;
    use tests::functional::payload_format_v5::*;
    use tests::functional::publication_expiry_v5::*;
    use tests::functional::pubsub_v5::*;
    use tests::functional::request_response_v5::*;
    use tests::functional::retain_handling_v5::*;
    use tests::functional::server_keepalive_v5::*;
    use tests::functional::session_v5::*;
    use tests::functional::shared_subscription::*;
    use tests::functional::subscribe_identifiers_v5::*;
    use tests::functional::topic_alias_v5::*;
    use tests::functional::user_properties_v5::*;
    use tests::functional::will_delay_v5::*;

    let mut suite = TestSuite::new("functional_v5");
    suite.add(ConnectV5Test);
    suite.add(ConnectV5ReasonCodeTest);
    suite.add(PubSubV5Qos0Test);
    suite.add(PubSubV5Qos1Test);
    suite.add(PubSubV5Qos2Test);
    // V5 specific features
    suite.add(LastWillV5Test);
    suite.add(PingV5Test);
    // Session management
    suite.add(SessionExpiryV5Test);
    suite.add(SessionTakeoverV5Test);
    suite.add(SessionCleanStartV5Test);
    // Will delay
    suite.add(WillDelayV5Test);
    // No local
    suite.add(NoLocalV5Test);
    // Retain handling
    suite.add(RetainHandlingNoAtSubscribeV5Test);
    suite.add(RetainHandlingNewV5Test);
    suite.add(RetainAsPublishedV5Test);
    // Disconnect reason codes
    suite.add(DisconnectReasonV5Test);
    // Flow control
    suite.add(FlowControlV5Test);
    // Shared subscriptions
    suite.add(SharedSubV5Test);
    // V5 CONNACK property checks
    suite.add(AssignedClientIdV5Test);
    suite.add(ServerKeepAliveV5Test);
    suite.add(ServerTopicAliasV5Test);
    suite.add(MaxPacketSizeV5Test);
    suite.add(MaxPacketSizeEnforcementV5Test);
    suite.add(SubscribeIdentifiersV5Test);
    suite.add(WildcardAvailableV5Test);
    suite.add(PayloadFormatV5Test);
    suite.add(PublicationExpiryV5Test);
    suite.add(RequestResponseV5Test);
    suite.add(UserPropertiesV5Test);
    suite.add(ClientTopicAliasV5Test);
    suite
}

fn build_stress_suite(client_count: usize) -> TestSuite {
    use tests::stress::fanout::*;
    use tests::stress::load_v311::*;
    use tests::stress::memory::*;

    let mut suite = TestSuite::new("stress");
    suite.add(ConnectionLoadTest { client_count });
    suite.add(PublishLoadTest::default());
    suite.add(FanOutTest::default());
    suite.add(RetainFloodTest);
    suite.add(SubscriptionStressTest);
    suite
}

fn build_chaos_suite(iterations: usize) -> TestSuite {
    use tests::chaos::disconnect::*;
    use tests::chaos::packet_loss::*;
    use tests::chaos::restart::*;

    let mut suite = TestSuite::new("chaos");
    suite.add(BrokerRestartTest);
    suite.add(BrokerRestartPubSubTest);
    suite.add(ConnectionChurnTest { cycles: iterations * 5 });
    suite.add(ReconnectStormTest { client_count: 50 });
    suite.add(Qos1ReliabilityTest { message_count: iterations * 10 });
    suite.add(SlowConsumerTest);
    suite
}

/// Initialize logging: console at info level, log file at debug level (includes packet traces)
fn init_logging(log_path: &std::path::Path, verbose: bool) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::Layer;

    // Console filter: info by default, debug if verbose
    let console_level = if verbose { "debug" } else { "info" };
    let console_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(console_level));

    // File filter: always debug (captures SEND/RECV packet traces)
    let file_filter = EnvFilter::new("debug");

    // Create file appender
    match std::fs::File::create(log_path) {
        Ok(file) => {
            let file_layer = tracing_subscriber::fmt::layer()
                .with_writer(std::sync::Arc::new(file))
                .with_filter(file_filter);

            let console_layer = tracing_subscriber::fmt::layer().with_filter(console_filter);

            tracing_subscriber::registry().with(console_layer).with(file_layer).init();
        }
        Err(e) => {
            warn!("Cannot create log file {}: {}, falling back to console-only", log_path.display(), e);
            tracing_subscriber::fmt().with_env_filter(console_filter).init();
        }
    }
}
