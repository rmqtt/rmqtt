//! mqtt_harness - RMQTT Industrial-Grade Test Harness
//!
//! Entry point for the RMQTT broker test and benchmarking system.
//! Supports functional (v3/v311/v5), stress, and chaos test suites.

#![deny(unsafe_code)]
#![allow(dead_code)]

use std::path::PathBuf;
use std::time::Duration;

use structopt::StructOpt;
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

#[derive(Debug, StructOpt)]
#[structopt(name = "mqtt_harness", about = "RMQTT Industrial-Grade Test Harness")]
struct Opt {
    /// Broker address (default: 127.0.0.1:1883)
    #[structopt(short, long, default_value = "127.0.0.1:1883")]
    addr: String,

    /// Path to rmqttd binary
    #[structopt(short, long)]
    binary: Option<String>,

    /// Path to rmqtt.toml config
    #[structopt(short = "c", long)]
    config: Option<String>,

    /// Workspace root (for finding target/release/rmqttd)
    #[structopt(long)]
    workspace: Option<String>,

    /// Run only specific test suites (functional_v3, functional_v311, functional_v5, stress, chaos)
    #[structopt(short, long, use_delimiter = true)]
    suites: Vec<String>,

    /// Number of parallel workers
    #[structopt(short, long, default_value = "4")]
    workers: usize,

    /// Verbose output
    #[structopt(short, long)]
    verbose: bool,

    /// Output JSON report to file
    #[structopt(long)]
    json: Option<String>,

    /// Output HTML report to file
    #[structopt(long)]
    html: Option<String>,

    /// Do not start/stop broker (assume it's already running)
    #[structopt(long)]
    no_broker: bool,

    /// Stress test client count
    #[structopt(long, default_value = "100")]
    stress_clients: usize,

    /// Chaos test iterations
    #[structopt(long, default_value = "5")]
    chaos_iterations: usize,

    /// Write detailed test log to file (includes packet traces for failed tests)
    #[structopt(long, default_value = "test-detail.log")]
    log_file: String,
}

fn main() {
    let opt = Opt::from_args();

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
    use tests::functional::connect_v311::*;
    use tests::functional::pubsub_v311::*;
    use tests::functional::wildcard::*;

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
    suite
}

fn build_functional_v5_suite() -> TestSuite {
    use tests::functional::connect_v5::*;
    use tests::functional::pubsub_v5::*;

    let mut suite = TestSuite::new("functional_v5");
    suite.add(ConnectV5Test);
    suite.add(ConnectV5ReasonCodeTest);
    suite.add(PubSubV5Qos0Test);
    suite.add(PubSubV5Qos1Test);
    suite.add(PubSubV5Qos2Test);
    suite
}

fn build_stress_suite(client_count: usize) -> TestSuite {
    use tests::stress::fanout::*;
    use tests::stress::load_v311::*;

    let mut suite = TestSuite::new("stress");
    suite.add(ConnectionLoadTest { client_count });
    suite.add(PublishLoadTest::default());
    suite.add(FanOutTest::default());
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
