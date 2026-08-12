//! Test Suite - grouping and ordering of test cases

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::info;

use super::testcase::TestCase;

/// A named group of test cases
pub struct TestSuite {
    pub name: String,
    pub tests: Vec<Arc<dyn TestCase>>,
    pub parallel: bool,
    /// Broker config file used by the whole suite (set after splitting;
    /// `None` only before `split_suites_by_config` runs, or when the suite
    /// is explicitly created with the harness default config semantics).
    pub config: Option<PathBuf>,
}

impl TestSuite {
    /// Create a new test suite
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string(), tests: Vec::new(), parallel: false, config: None }
    }

    /// Create a parallel test suite
    pub fn parallel(name: &str) -> Self {
        Self { name: name.to_string(), tests: Vec::new(), parallel: true, config: None }
    }

    /// Create a test suite pinned to a specific broker config
    pub fn with_config(name: &str, config: PathBuf) -> Self {
        Self { name: name.to_string(), tests: Vec::new(), parallel: false, config: Some(config) }
    }

    /// Add a test case
    pub fn add<T: TestCase + 'static>(&mut self, test: T) {
        self.tests.push(Arc::new(test));
    }

    /// Add an already-arc'd test case
    pub fn add_arc(&mut self, test: Arc<dyn TestCase>) {
        self.tests.push(test);
    }

    /// Get the number of tests in this suite
    pub fn len(&self) -> usize {
        self.tests.len()
    }

    /// Check if the suite is empty
    pub fn is_empty(&self) -> bool {
        self.tests.is_empty()
    }
}

/// A group of test cases that share the same declared broker config
/// (`None` = harness default config), preserving their original relative order.
type ConfigGroup = (Option<PathBuf>, Vec<Arc<dyn TestCase>>);

/// Split each suite into sub-suites grouped by the test cases' `broker_config()`.
///
/// - Suites with an explicit `config` are kept as-is (they are already pinned
///   to a single config, e.g. cluster suites).
/// - Otherwise test cases are grouped by their declared config, preserving the
///   original relative order inside each group:
///   - the default-config group keeps the original suite name;
///   - every other group becomes a `{suite}@{config_name}` sub-suite.
/// - Every produced suite gets a concrete `config` (`default_config` for the
///   default group), so the scheduler can switch configs at suite boundaries.
pub fn split_suites_by_config(suites: Vec<TestSuite>, default_config: &Path) -> Vec<TestSuite> {
    let mut out = Vec::new();
    for suite in suites {
        if suite.config.is_some() {
            out.push(suite);
            continue;
        }

        // Group by declared config, preserving first-seen order.
        let mut groups: Vec<ConfigGroup> = Vec::new();
        for test in suite.tests {
            let cfg = test.broker_config();
            match groups.iter_mut().find(|(c, _)| *c == cfg) {
                Some((_, tests)) => tests.push(test),
                None => groups.push((cfg, vec![test])),
            }
        }

        for (cfg, tests) in groups {
            let name = match &cfg {
                None => suite.name.clone(),
                Some(p) => format!("{}@{}", suite.name, config_name(p)),
            };
            let config = cfg.or_else(|| Some(default_config.to_path_buf()));
            let sub = TestSuite { name, tests, parallel: suite.parallel, config };
            info!(
                "split suite '{}' -> '{}' ({} tests, config: {:?})",
                suite.name,
                sub.name,
                sub.tests.len(),
                sub.config.as_ref().map(|p| p.display().to_string())
            );
            out.push(sub);
        }
    }
    out
}

/// Derive a short human-readable name for a config file, e.g.
/// `rmqtt-test/configs/retain-disabled/rmqtt.toml` -> `retain-disabled`.
fn config_name(path: &Path) -> String {
    path.parent()
        .and_then(|d| d.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default())
}
