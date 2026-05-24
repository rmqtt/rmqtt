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
}

impl TestResult {
    pub fn passed(name: &str, suite: &str, duration: Duration) -> Self {
        Self {
            name: name.to_string(),
            suite: suite.to_string(),
            verdict: TestVerdict::Passed,
            duration,
            retries: 0,
        }
    }

    pub fn failed(name: &str, suite: &str, duration: Duration, reason: String) -> Self {
        Self {
            name: name.to_string(),
            suite: suite.to_string(),
            verdict: TestVerdict::Failed(reason),
            duration,
            retries: 0,
        }
    }

    pub fn timeout(name: &str, suite: &str, duration: Duration) -> Self {
        Self {
            name: name.to_string(),
            suite: suite.to_string(),
            verdict: TestVerdict::Timeout,
            duration,
            retries: 0,
        }
    }

    pub fn error(name: &str, suite: &str, duration: Duration, msg: String) -> Self {
        Self {
            name: name.to_string(),
            suite: suite.to_string(),
            verdict: TestVerdict::Error(msg),
            duration,
            retries: 0,
        }
    }
}

/// Test case trait
pub trait TestCase: Send + Sync {
    /// Test case name
    fn name(&self) -> &str;

    /// Execute the test case
    fn execute(&self, ctx: &mut TestContext) -> TestResult;

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
