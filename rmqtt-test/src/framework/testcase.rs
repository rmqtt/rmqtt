//! TestCase trait and result types

use std::time::Duration;

/// Test verdict
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestVerdict {
    Passed,
    Failed(String),
    Skipped(String),
    Error(String),
    Timeout,
    /// The broker violated the spec as expected by this test case. Recorded
    /// as evidence (see the linked GitHub issue) and NOT counted as a suite
    /// failure. When the broker becomes compliant, the test surfaces as an
    /// unexpected pass and should be promoted to a normal assertion.
    ExpectedFail(String),
    /// Record-type verdict for MAY-level spec behaviors: the test observed
    /// and reported the broker's actual behavior without asserting a
    /// pass/fail outcome. Never counts as a failure.
    Info(String),
}

impl TestVerdict {
    pub fn is_passed(&self) -> bool {
        matches!(self, TestVerdict::Passed)
    }

    /// Verdicts that count as "no failure" for scheduling purposes:
    /// retrying is stopped and the suite is not halted.
    pub fn counts_as_success(&self) -> bool {
        matches!(self, TestVerdict::Passed | TestVerdict::ExpectedFail(_) | TestVerdict::Info(_))
    }
}

/// Declared expectation of a test case, applied by the scheduler after
/// execution to map the raw outcome onto the reported verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Expectation {
    /// Ordinary assertive test (default).
    #[default]
    Normal,
    /// The broker is known to violate the spec here (a GitHub issue must be
    /// registered). A failure is recorded as `ExpectedFail`; a pass is
    /// annotated `UNEXPECTED-PASS` so the test can be promoted.
    ExpectedFail,
    /// Record-type test for MAY-level spec behavior: outcomes are reported
    /// as `Info` observations, never as failures.
    Info,
}

/// Test result with timing and metadata
#[derive(Debug, Clone)]
pub struct TestResult {
    pub name: String,
    pub suite: String,
    pub verdict: TestVerdict,
    pub duration: Duration,
    pub retries: u32,
    /// Optional annotation surfaced in reports, e.g. the reason a test passed
    /// without executing (retain-dependent tests when the plugin is disabled).
    pub note: Option<String>,
}

impl TestResult {
    pub fn passed(name: &str, suite: &str, duration: Duration) -> Self {
        Self {
            name: name.to_string(),
            suite: suite.to_string(),
            verdict: TestVerdict::Passed,
            duration,
            retries: 0,
            note: None,
        }
    }

    /// Passed with an explanatory note (e.g. "skipped: 'rmqtt-retainer'
    /// plugin not enabled"), so the reason is visible in the reports.
    pub fn passed_with_note(name: &str, suite: &str, duration: Duration, note: &str) -> Self {
        Self {
            name: name.to_string(),
            suite: suite.to_string(),
            verdict: TestVerdict::Passed,
            duration,
            retries: 0,
            note: Some(note.to_string()),
        }
    }

    pub fn failed(name: &str, suite: &str, duration: Duration, reason: String) -> Self {
        Self {
            name: name.to_string(),
            suite: suite.to_string(),
            verdict: TestVerdict::Failed(reason),
            duration,
            retries: 0,
            note: None,
        }
    }

    pub fn timeout(name: &str, suite: &str, duration: Duration) -> Self {
        Self {
            name: name.to_string(),
            suite: suite.to_string(),
            verdict: TestVerdict::Timeout,
            duration,
            retries: 0,
            note: None,
        }
    }

    pub fn skipped(name: &str, suite: &str, duration: Duration, reason: &str) -> Self {
        Self {
            name: name.to_string(),
            suite: suite.to_string(),
            verdict: TestVerdict::Skipped(reason.to_string()),
            duration,
            retries: 0,
            note: None,
        }
    }

    pub fn error(name: &str, suite: &str, duration: Duration, msg: String) -> Self {
        Self {
            name: name.to_string(),
            suite: suite.to_string(),
            verdict: TestVerdict::Error(msg),
            duration,
            retries: 0,
            note: None,
        }
    }
}

/// Test case trait
pub trait TestCase: Send + Sync {
    /// Test case name
    fn name(&self) -> &str;

    /// Execute the test case
    fn execute(&self, ctx: &mut TestContext) -> TestResult;

    /// Broker config file required by this test case (absolute or
    /// workspace-relative path), or `None` to use the harness default config
    /// (`--config`, or `rmqtt-test/configs/default/rmqtt.toml`).
    ///
    /// This is a *grouping* hint only: at suite build time, test cases that
    /// declare the same non-default config are split out into their own
    /// `{suite}@{config}` sub-suite, so the scheduler only ever switches the
    /// broker config at suite boundaries.
    fn broker_config(&self) -> Option<PathBuf> {
        None
    }

    /// Broker listen address this test case's config expects, when it differs
    /// from the harness-wide `--addr`.
    ///
    /// Only meaningful together with [`TestCase::broker_config`]: while the
    /// `{suite}@{config}` sub-suite runs, the harness health-probes *this*
    /// address instead of `--addr`, so a fixture that pins its own ports (e.g.
    /// `configs/flapping` on MQTT 1902) is not forced onto the shared 1883
    /// that most configs use. `None` (the default) keeps the harness-wide
    /// address.
    ///
    /// Like `broker_config`, this is part of the *grouping* key: cases
    /// declaring the same config and the same address share a sub-suite, and
    /// the scheduler only switches address at suite boundaries.
    fn broker_addr(&self) -> Option<&'static str> {
        None
    }

    /// Test timeout (default: 60 seconds)
    fn timeout(&self) -> Duration {
        Duration::from_secs(60)
    }

    /// Maximum retries (default: 0)
    fn max_retries(&self) -> u32 {
        0
    }

    /// Test dependencies (names of tests that must complete first)
    fn depends_on(&self) -> Vec<String> {
        Vec::new()
    }

    /// Declared expectation of this test case (default: normal assertion).
    /// See [`Expectation`] for the expected-fail / record-type mechanisms.
    fn expectation(&self) -> Expectation {
        Expectation::Normal
    }
}

use crate::framework::context::TestContext;
use std::path::PathBuf;
