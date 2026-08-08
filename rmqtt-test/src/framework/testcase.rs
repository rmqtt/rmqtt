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
}

impl TestVerdict {
    pub fn is_passed(&self) -> bool {
        matches!(self, TestVerdict::Passed)
    }
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
}

use crate::framework::context::TestContext;
use std::path::PathBuf;
