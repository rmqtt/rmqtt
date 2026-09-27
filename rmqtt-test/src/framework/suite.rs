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
    /// Address the harness health-probes while `config` is in use
    /// (`None` = the harness-wide `--addr`). Populated from
    /// `TestCase::broker_addr` during splitting, for configs that pin their
    /// own ports.
    pub addr: Option<String>,
}

impl TestSuite {
    /// Create a new test suite
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string(), tests: Vec::new(), parallel: false, config: None, addr: None }
    }

    /// Create a parallel test suite
    pub fn parallel(name: &str) -> Self {
        Self { name: name.to_string(), tests: Vec::new(), parallel: true, config: None, addr: None }
    }

    /// Create a test suite pinned to a specific broker config
    pub fn with_config(name: &str, config: PathBuf) -> Self {
        Self { name: name.to_string(), tests: Vec::new(), parallel: false, config: Some(config), addr: None }
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

/// A group of test cases that share the same declared broker config and listen
/// address (`None` config = harness default config, `None` address = the
/// harness-wide `--addr`), preserving their original relative order.
type ConfigGroup = (Option<PathBuf>, Option<String>, Vec<Arc<dyn TestCase>>);

/// Split each suite into sub-suites grouped by the test cases' `broker_config()`
/// and `broker_addr()`.
///
/// - Suites with an explicit `config` are kept as-is (they are already pinned
///   to a single config, e.g. cluster suites).
/// - Otherwise test cases are grouped by their declared config *and* address,
///   preserving the original relative order inside each group:
///   - the default-config group keeps the original suite name;
///   - every other group becomes a `{suite}@{config_name}` sub-suite.
/// - Every produced suite gets a concrete `config` (`default_config` for the
///   default group) plus the group's `addr`, so the scheduler can switch both
///   at suite boundaries.
///
/// The address is part of the grouping key but not of the generated name: two
/// groups would only share a name by declaring the same config file with
/// different addresses, which no fixture in the repository does.
pub fn split_suites_by_config(suites: Vec<TestSuite>, default_config: &Path) -> Vec<TestSuite> {
    let mut out = Vec::new();
    for suite in suites {
        if suite.config.is_some() {
            out.push(suite);
            continue;
        }

        // Group by declared config and address, preserving first-seen order.
        let mut groups: Vec<ConfigGroup> = Vec::new();
        for test in suite.tests {
            let cfg = test.broker_config();
            let addr = test.broker_addr().map(str::to_string);
            match groups.iter_mut().find(|(c, a, _)| *c == cfg && *a == addr) {
                Some((_, _, tests)) => tests.push(test),
                None => groups.push((cfg, addr, vec![test])),
            }
        }

        for (cfg, addr, tests) in groups {
            let name = match &cfg {
                None => suite.name.clone(),
                Some(p) => format!("{}@{}", suite.name, config_name(p)),
            };
            let config = cfg.or_else(|| Some(default_config.to_path_buf()));
            let sub = TestSuite { name, tests, parallel: suite.parallel, config, addr };
            info!(
                "split suite '{}' -> '{}' ({} tests, config: {:?}, addr: {})",
                suite.name,
                sub.name,
                sub.tests.len(),
                sub.config.as_ref().map(|p| p.display().to_string()),
                sub.addr.as_deref().unwrap_or("<harness --addr>")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framework::context::TestContext;
    use crate::framework::testcase::TestResult;
    use std::time::Duration;

    /// A case whose only behaviour is declaring what it needs.
    struct Stub {
        name: &'static str,
        config: Option<&'static str>,
        addr: Option<&'static str>,
    }

    impl TestCase for Stub {
        fn name(&self) -> &str {
            self.name
        }

        fn execute(&self, _ctx: &mut TestContext) -> TestResult {
            TestResult::passed(self.name, "stub", Duration::ZERO)
        }

        fn broker_config(&self) -> Option<PathBuf> {
            self.config.map(PathBuf::from)
        }

        fn broker_addr(&self) -> Option<&'static str> {
            self.addr
        }
    }

    fn stub(
        name: &'static str,
        config: Option<&'static str>,
        addr: Option<&'static str>,
    ) -> Arc<dyn TestCase> {
        Arc::new(Stub { name, config, addr })
    }

    /// Cases declaring the same config *and* address share a sub-suite; a
    /// different address splits them apart even when the config matches.
    #[test]
    fn splits_by_config_and_address() {
        let mut suite = TestSuite::new("functional_x");
        suite.add_arc(stub("plain-1", None, None));
        suite.add_arc(stub("own-port-a", Some("/cfg/flapping/rmqtt.toml"), Some("127.0.0.1:1902")));
        suite.add_arc(stub("plain-2", None, None));
        suite.add_arc(stub("own-port-b", Some("/cfg/flapping/rmqtt.toml"), Some("127.0.0.1:1903")));

        let default_config = Path::new("/cfg/default/rmqtt.toml");
        let out = split_suites_by_config(vec![suite], default_config);

        assert_eq!(out.len(), 3);

        // The default group keeps the suite name, keeps its cases together and
        // inherits the harness-wide address (`None`).
        assert_eq!(out[0].name, "functional_x");
        assert_eq!(out[0].tests.len(), 2);
        assert_eq!(out[0].config.as_deref(), Some(default_config));
        assert_eq!(out[0].addr, None);

        // Same config + different address => two sub-suites, each carrying its
        // own address. The generated name cannot tell them apart, which is the
        // documented (and currently unused) edge.
        assert_eq!(out[1].name, "functional_x@flapping");
        assert_eq!(out[1].tests.len(), 1);
        assert_eq!(out[1].addr.as_deref(), Some("127.0.0.1:1902"));

        assert_eq!(out[2].name, "functional_x@flapping");
        assert_eq!(out[2].tests.len(), 1);
        assert_eq!(out[2].addr.as_deref(), Some("127.0.0.1:1903"));
    }

    /// A suite created with an explicit config is already pinned and is passed
    /// through untouched, so cluster suites never gain an address.
    #[test]
    fn suite_with_explicit_config_is_not_split() {
        let suite = TestSuite::with_config("pulsar", PathBuf::from("/cfg/pulsar/rmqtt.toml"));
        let out = split_suites_by_config(vec![suite], Path::new("/cfg/default/rmqtt.toml"));

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "pulsar");
        assert_eq!(out[0].addr, None);
        assert!(out[0].is_empty());
    }

    /// The sub-suite name comes from the config's parent directory.
    #[test]
    fn config_name_uses_the_parent_directory() {
        assert_eq!(
            config_name(Path::new("rmqtt-test/configs/retain-disabled/rmqtt.toml")),
            "retain-disabled"
        );
        assert_eq!(config_name(Path::new("rmqtt.toml")), "rmqtt.toml");
    }
}
