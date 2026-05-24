//! Test Suite - grouping and ordering of test cases

use std::sync::Arc;

use super::testcase::TestCase;

/// A named group of test cases
pub struct TestSuite {
    pub name: String,
    pub tests: Vec<Arc<dyn TestCase>>,
    pub parallel: bool,
}

impl TestSuite {
    /// Create a new test suite
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string(), tests: Vec::new(), parallel: false }
    }

    /// Create a parallel test suite
    pub fn parallel(name: &str) -> Self {
        Self { name: name.to_string(), tests: Vec::new(), parallel: true }
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
