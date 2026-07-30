//! Detailed test log writer - records per-test execution details for failure analysis

use std::io::Write;
use std::path::Path;

use chrono::Local;

use crate::framework::scheduler::TestSummary;
use crate::framework::testcase::{TestResult, TestVerdict};

/// Write a detailed test log file with per-test information
pub fn write_detail_log(
    path: &Path,
    results: &[TestResult],
    summary: &TestSummary,
) -> Result<(), anyhow::Error> {
    let mut file = std::fs::File::create(path)?;
    writeln!(file, "RMQTT Test Harness - Detailed Test Log")?;
    writeln!(file, "Generated: {}", Local::now().format("%Y-%m-%d %H:%M:%S"))?;
    writeln!(file)?;
    write_summary_section(&mut file, summary)?;
    writeln!(file)?;

    // Write details for each test
    for result in results {
        write_test_detail(&mut file, result)?;
        writeln!(file)?;
    }

    // Write failed test summary at the end for quick reference
    let failed: Vec<&TestResult> = results.iter().filter(|r| !r.verdict.is_passed()).collect();
    if !failed.is_empty() {
        writeln!(file, "═══════════════════════════════════════════════════════════════")?;
        writeln!(file, "FAILURE SUMMARY")?;
        writeln!(file, "═══════════════════════════════════════════════════════════════")?;
        for r in &failed {
            let reason = verdict_reason(&r.verdict);
            writeln!(file, "  {} [{}] - {}", r.name, r.suite, reason)?;
        }
        writeln!(file)?;
    }

    file.flush()?;
    Ok(())
}

fn write_summary_section(file: &mut std::fs::File, summary: &TestSummary) -> Result<(), anyhow::Error> {
    writeln!(file, "───────────────────────────────────────────────────────────────")?;
    writeln!(file, "SUMMARY")?;
    writeln!(file, "───────────────────────────────────────────────────────────────")?;
    writeln!(file, "  Total:   {}", summary.total)?;
    writeln!(file, "  Passed:  {}", summary.passed)?;
    writeln!(file, "  Failed:  {}", summary.failed)?;
    writeln!(file, "  Errors:  {}", summary.errors)?;
    writeln!(file, "  Timeout: {}", summary.timeouts)?;
    writeln!(file, "  Skipped: {}", summary.skipped)?;
    writeln!(file, "  Duration: {:.1}s", summary.total_duration.as_secs_f64())?;
    Ok(())
}

fn write_test_detail(file: &mut std::fs::File, result: &TestResult) -> Result<(), anyhow::Error> {
    let status_str = match &result.verdict {
        TestVerdict::Passed => "PASSED",
        TestVerdict::Failed(_) => "FAILED",
        TestVerdict::Skipped(_) => "SKIPPED",
        TestVerdict::Error(_) => "ERROR",
        TestVerdict::Timeout => "TIMEOUT",
    };

    writeln!(file, "───────────────────────────────────────────────────────────────")?;
    writeln!(file, "TEST: {} [{}]", result.name, result.suite)?;
    writeln!(file, "───────────────────────────────────────────────────────────────")?;
    writeln!(file, "  Status:   {}", status_str)?;
    writeln!(file, "  Duration: {:.3}s", result.duration.as_secs_f64())?;
    if result.retries > 0 {
        writeln!(file, "  Retries:  {}", result.retries)?;
    }

    // Write detailed failure reason
    match &result.verdict {
        TestVerdict::Failed(reason) | TestVerdict::Error(reason) | TestVerdict::Skipped(reason) => {
            writeln!(file)?;
            writeln!(file, "  FAILURE REASON:")?;
            // Write each line of the reason with indentation
            for line in reason.lines() {
                writeln!(file, "    {}", line)?;
            }
            writeln!(file)?;
            write_failure_hints(file, reason)?;
        }
        TestVerdict::Timeout => {
            writeln!(file)?;
            writeln!(file, "  FAILURE REASON: Test timed out after {:.1}s", result.duration.as_secs_f64())?;
            writeln!(file, "  HINT: The test may be waiting for a response that never comes.")?;
            writeln!(file, "  This often indicates a protocol-level issue (e.g., broker rejected")?;
            writeln!(file, "  a packet, or the client sent a malformed packet).")?;
        }
        TestVerdict::Passed => {}
    }

    Ok(())
}

/// Write diagnostic hints based on common failure patterns
fn write_failure_hints(file: &mut std::fs::File, reason: &str) -> Result<(), anyhow::Error> {
    writeln!(file, "  DIAGNOSTIC HINTS:")?;

    if reason.contains("connection closed by broker") {
        writeln!(file, "    - The broker actively disconnected the client.")?;
        writeln!(file, "    - Common causes:")?;
        writeln!(file, "      * Malformed MQTT packet (check packet encoding)")?;
        writeln!(file, "      * V5 property length byte sent in v3.1.1 SUBSCRIBE/UNSUBSCRIBE")?;
        writeln!(file, "      * Invalid topic filter in SUBSCRIBE")?;
        writeln!(file, "      * Protocol violation (e.g., wrong packet order)")?;
        writeln!(file, "    - To debug: run with RUST_LOG=debug to see packet hex traces")?;
    } else if reason.contains("timeout") || reason.contains("timed out") {
        writeln!(file, "    - The test waited for a response that never came.")?;
        writeln!(file, "    - Possible causes:")?;
        writeln!(file, "      * Broker didn't send expected response")?;
        writeln!(file, "      * Client missed the response (wrong packet decode)")?;
        writeln!(file, "      * Subscription not established before publish")?;
    } else if reason.contains("CONNACK") {
        writeln!(file, "    - The broker rejected the connection attempt.")?;
        writeln!(file, "    - Check: client ID, credentials, protocol version")?;
    }

    Ok(())
}

fn verdict_reason(verdict: &TestVerdict) -> String {
    match verdict {
        TestVerdict::Failed(r) | TestVerdict::Error(r) | TestVerdict::Skipped(r) => r.clone(),
        TestVerdict::Timeout => "timeout".to_string(),
        TestVerdict::Passed => "passed".to_string(),
    }
}
