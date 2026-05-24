//! JSON test report

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::framework::scheduler::TestSummary;
use crate::framework::testcase::{TestResult, TestVerdict};

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonReport {
    pub suite: String,
    pub summary: JsonSummary,
    pub cases: Vec<JsonTestCase>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub errors: usize,
    pub timeouts: usize,
    pub duration_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonTestCase {
    pub name: String,
    pub suite: String,
    pub status: String,
    pub reason: Option<String>,
    pub duration_ms: u64,
    pub retries: u32,
}

pub struct JsonReporter;

impl JsonReporter {
    /// Generate a JSON report
    pub fn generate(suite_name: &str, results: &[TestResult], summary: &TestSummary) -> JsonReport {
        let cases: Vec<JsonTestCase> = results
            .iter()
            .map(|r| JsonTestCase {
                name: r.name.clone(),
                suite: r.suite.clone(),
                status: verdict_to_status(&r.verdict),
                reason: verdict_to_reason(&r.verdict),
                duration_ms: r.duration.as_millis() as u64,
                retries: r.retries,
            })
            .collect();

        JsonReport {
            suite: suite_name.to_string(),
            summary: JsonSummary {
                total: summary.total,
                passed: summary.passed,
                failed: summary.failed,
                skipped: summary.skipped,
                errors: summary.errors,
                timeouts: summary.timeouts,
                duration_ms: summary.total_duration.as_millis() as u64,
            },
            cases,
        }
    }

    /// Write report to a file
    pub fn write_to_file(report: &JsonReport, path: &Path) -> Result<(), anyhow::Error> {
        let json = serde_json::to_string_pretty(report)?;
        std::fs::write(path, json)?;
        Ok(())
    }
}

fn verdict_to_status(v: &TestVerdict) -> String {
    match v {
        TestVerdict::Passed => "passed".to_string(),
        TestVerdict::Failed(_) => "failed".to_string(),
        TestVerdict::Skipped(_) => "skipped".to_string(),
        TestVerdict::Error(_) => "error".to_string(),
        TestVerdict::Timeout => "timeout".to_string(),
    }
}

fn verdict_to_reason(v: &TestVerdict) -> Option<String> {
    match v {
        TestVerdict::Failed(r) | TestVerdict::Skipped(r) | TestVerdict::Error(r) => Some(r.clone()),
        TestVerdict::Timeout => Some("test timed out".to_string()),
        TestVerdict::Passed => None,
    }
}
