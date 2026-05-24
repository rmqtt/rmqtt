//! Console test report

use crate::framework::scheduler::TestSummary;
use crate::framework::testcase::{TestResult, TestVerdict};

pub struct ConsoleReporter;

impl ConsoleReporter {
    /// Print test results to console
    pub fn report(results: &[TestResult], summary: &TestSummary) {
        println!("\n{}", "=".repeat(60));
        println!("  RMQTT Test Harness - Results");
        println!("{}\n", "=".repeat(60));

        for result in results {
            let icon = match &result.verdict {
                TestVerdict::Passed => "\u{2714}",
                TestVerdict::Failed(_) | TestVerdict::Error(_) | TestVerdict::Timeout => "\u{2718}",
                TestVerdict::Skipped(_) => "\u{25CB}",
            };

            let reason = match &result.verdict {
                TestVerdict::Failed(r) | TestVerdict::Error(r) | TestVerdict::Skipped(r) => {
                    format!(" ({})", r)
                }
                TestVerdict::Timeout => " (timeout)".to_string(),
                TestVerdict::Passed => String::new(),
            };

            let duration = if result.duration.as_millis() < 1000 {
                format!("{}ms", result.duration.as_millis())
            } else {
                format!("{:.1}s", result.duration.as_secs_f64())
            };

            println!(" {} {} [{}]{}", icon, result.name, duration, reason);
        }

        println!("\n{}", "-".repeat(60));
        println!(
            "  Total: {} | Passed: {} | Failed: {} | Errors: {} | Timeouts: {} | Skipped: {}",
            summary.total,
            summary.passed,
            summary.failed,
            summary.errors,
            summary.timeouts,
            summary.skipped,
        );
        println!(
            "  Duration: {:.2}s",
            summary.total_duration.as_secs_f64()
        );
        println!("{}\n", "=".repeat(60));
    }
}
