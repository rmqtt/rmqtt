//! Test Scheduler - serial/parallel execution with timeout, retry, DAG dependencies

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use super::context::TestContext;
use super::suite::TestSuite;
use super::testcase::{TestCase, TestResult, TestVerdict};

/// Schedule and execute test suites
pub struct TestScheduler {
    results: Vec<TestResult>,
}

impl TestScheduler {
    pub fn new() -> Self {
        Self { results: Vec::new() }
    }

    /// Run all test suites (synchronous - each test creates its own tokio runtime)
    pub fn run(&mut self, suites: Vec<TestSuite>, ctx: &mut TestContext) {
        for suite in suites {
            self.run_suite(suite, ctx);
        }
    }

    /// Run a single test suite
    fn run_suite(&mut self, suite: TestSuite, ctx: &mut TestContext) {
        info!("Running suite: {} ({} tests)", suite.name, suite.tests.len());

        if suite.parallel {
            self.run_interleaved(&suite, ctx);
        } else {
            self.run_serial(&suite, ctx);
        }
    }

    /// Run tests serially with DAG dependency resolution
    fn run_serial(&mut self, suite: &TestSuite, ctx: &mut TestContext) {
        let order = resolve_dag_order(&suite.tests);

        for idx in order {
            let test = &suite.tests[idx];
            let result = self.execute_test_with_retry(test, ctx, &suite.name);
            let passed = result.verdict.is_passed();
            self.results.push(result);

            if !passed && is_critical_failure(&self.results) {
                warn!("Critical test failure, stopping suite '{}'", suite.name);
                break;
            }
        }
    }

    /// Run tests in an interleaved fashion
    fn run_interleaved(&mut self, suite: &TestSuite, ctx: &mut TestContext) {
        for test in &suite.tests {
            let result = self.execute_test_with_retry(test, ctx, &suite.name);
            self.results.push(result);
        }
    }

    /// Execute a test with retry
    fn execute_test_with_retry(
        &self,
        test: &Arc<dyn TestCase>,
        ctx: &mut TestContext,
        suite_name: &str,
    ) -> TestResult {
        let start = Instant::now();
        let timeout = test.timeout();
        let max_retries = test.max_retries();

        let mut result = Self::execute_once(test.as_ref(), ctx, suite_name, start, timeout);

        for retry in 0..max_retries {
            if result.verdict.is_passed() {
                break;
            }
            info!("Retrying test '{}' (attempt {}/{})", test.name(), retry + 1, max_retries);
            let retry_start = Instant::now();
            result = Self::execute_once(test.as_ref(), ctx, suite_name, retry_start, timeout);
            result.retries = retry + 1;
        }

        result
    }

    /// Execute a test once with timeout
    fn execute_once(
        test: &dyn TestCase,
        ctx: &mut TestContext,
        suite_name: &str,
        start: Instant,
        timeout: Duration,
    ) -> TestResult {
        info!("  Running: {}", test.name());

        let result = test.execute(ctx);
        let elapsed = start.elapsed();

        if elapsed > timeout {
            TestResult::timeout(test.name(), suite_name, elapsed)
        } else {
            result
        }
    }

    /// Get all results
    pub fn results(&self) -> &[TestResult] {
        &self.results
    }

    /// Get summary
    pub fn summary(&self) -> TestSummary {
        let mut passed = 0;
        let mut failed = 0;
        let mut skipped = 0;
        let mut errors = 0;
        let mut timeouts = 0;
        let mut total_duration = Duration::ZERO;

        for r in &self.results {
            total_duration += r.duration;
            match &r.verdict {
                TestVerdict::Passed => passed += 1,
                TestVerdict::Failed(_) => failed += 1,
                TestVerdict::Skipped(_) => skipped += 1,
                TestVerdict::Error(_) => errors += 1,
                TestVerdict::Timeout => timeouts += 1,
            }
        }

        TestSummary { total: self.results.len(), passed, failed, skipped, errors, timeouts, total_duration }
    }
}

/// Test run summary
#[derive(Debug, Clone)]
pub struct TestSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub errors: usize,
    pub timeouts: usize,
    pub total_duration: Duration,
}

/// Resolve DAG dependencies to produce execution order (topological sort)
fn resolve_dag_order(tests: &[Arc<dyn TestCase>]) -> Vec<usize> {
    let n = tests.len();
    let name_to_idx: HashMap<String, usize> =
        tests.iter().enumerate().map(|(i, t)| (t.name().to_string(), i)).collect();

    // Build adjacency list
    let mut in_degree = vec![0usize; n];
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];

    for (i, test) in tests.iter().enumerate() {
        for dep in test.depends_on() {
            if let Some(&j) = name_to_idx.get(&dep) {
                adj[j].push(i);
                in_degree[i] += 1;
            }
        }
    }

    // Kahn's algorithm
    let mut queue: Vec<usize> = (0..n).filter(|&i| in_degree[i] == 0).collect();
    let mut order = Vec::with_capacity(n);

    while let Some(node) = queue.pop() {
        order.push(node);
        for &neighbor in &adj[node] {
            in_degree[neighbor] -= 1;
            if in_degree[neighbor] == 0 {
                queue.push(neighbor);
            }
        }
    }

    // If there's a cycle, fall back to sequential order
    if order.len() != n {
        warn!("Circular dependency detected, falling back to sequential order");
        (0..n).collect()
    } else {
        order
    }
}

/// Check if the most recent failure is critical enough to stop the suite
fn is_critical_failure(results: &[TestResult]) -> bool {
    results.iter().any(|r| matches!(r.verdict, TestVerdict::Error(_)))
}

impl Default for TestScheduler {
    fn default() -> Self {
        Self::new()
    }
}
