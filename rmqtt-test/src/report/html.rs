//! HTML test report

use std::path::Path;

use crate::framework::scheduler::TestSummary;
use crate::framework::testcase::{TestResult, TestVerdict};

pub struct HtmlReporter;

impl HtmlReporter {
    /// Generate an HTML report file
    pub fn generate(results: &[TestResult], summary: &TestSummary, path: &Path) -> Result<(), anyhow::Error> {
        let mut html = String::with_capacity(8192);

        html.push_str("<!DOCTYPE html>\n<html><head>\n");
        html.push_str("<meta charset='utf-8'>\n");
        html.push_str("<title>RMQTT Test Report</title>\n");
        html.push_str("<style>\n");
        html.push_str("body { font-family: monospace; margin: 20px; background: #1a1a2e; color: #eee; }\n");
        html.push_str("h1 { color: #e94560; }\n");
        html.push_str(".passed { color: #0cce6b; }\n");
        html.push_str(".failed { color: #ff6b6b; }\n");
        html.push_str(".error { color: #ff4757; }\n");
        html.push_str(".timeout { color: #ffa502; }\n");
        html.push_str(".skipped { color: #747d8c; }\n");
        html.push_str("table { border-collapse: collapse; width: 100%; }\n");
        html.push_str("th, td { padding: 8px 12px; text-align: left; border-bottom: 1px solid #333; }\n");
        html.push_str("th { background: #16213e; }\n");
        html.push_str(".summary { margin: 20px 0; padding: 15px; background: #16213e; border-radius: 8px; }\n");
        html.push_str("</style>\n");
        html.push_str("</head><body>\n");

        html.push_str("<h1>RMQTT Test Report</h1>\n");

        // Summary
        html.push_str("<div class='summary'>\n");
        html.push_str(&format!(
            "<p>Total: {} | <span class='passed'>Passed: {}</span> | \
             <span class='failed'>Failed: {}</span> | \
             <span class='error'>Errors: {}</span> | \
             <span class='timeout'>Timeouts: {}</span> | \
             <span class='skipped'>Skipped: {}</span></p>\n",
            summary.total, summary.passed, summary.failed, summary.errors, summary.timeouts, summary.skipped
        ));
        html.push_str(&format!(
            "<p>Duration: {:.2}s</p>\n",
            summary.total_duration.as_secs_f64()
        ));
        html.push_str("</div>\n");

        // Results table
        html.push_str("<table>\n<tr><th>Status</th><th>Test</th><th>Suite</th><th>Duration</th><th>Reason</th><th>Retries</th></tr>\n");

        for r in results {
            let (cls, label) = match &r.verdict {
                TestVerdict::Passed => ("passed", "PASS"),
                TestVerdict::Failed(_) => ("failed", "FAIL"),
                TestVerdict::Error(_) => ("error", "ERROR"),
                TestVerdict::Timeout => ("timeout", "TIMEOUT"),
                TestVerdict::Skipped(_) => ("skipped", "SKIP"),
            };
            let reason = match &r.verdict {
                TestVerdict::Failed(r) | TestVerdict::Error(r) | TestVerdict::Skipped(r) => r.clone(),
                TestVerdict::Timeout => "timed out".to_string(),
                TestVerdict::Passed => String::new(),
            };
            let duration = if r.duration.as_millis() < 1000 {
                format!("{}ms", r.duration.as_millis())
            } else {
                format!("{:.1}s", r.duration.as_secs_f64())
            };

            html.push_str(&format!(
                "<tr><td class='{}'>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>\n",
                cls, label, r.name, r.suite, duration, reason, r.retries
            ));
        }

        html.push_str("</table>\n</body></html>\n");

        std::fs::write(path, html)?;
        Ok(())
    }
}
