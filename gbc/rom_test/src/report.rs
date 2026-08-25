use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

use super::{error::RomTestError, verify::CheckResult};

/// Outcome of one matrix cell (case × model).
#[derive(Debug, Clone, Serialize)]
pub struct CaseResult {
    /// Full cell id, e.g. `m3_scy_change@cgb_d`.
    pub id: String,
    pub suite: String,
    pub model: String,
    pub description: String,
    pub tags: Vec<String>,
    pub passed: bool,
    /// Cell is declared in the manifest's `expected_failures` list.
    pub expected_failure: bool,
    /// Every check that was declared for the cell, with its outcome.
    pub checks: Vec<CheckResult>,
    pub error: Option<String>,
    /// Coarse error category (`io`, `config`, `png`).
    pub error_kind: Option<String>,
    pub screenshot: Option<String>,
    pub diff_image: Option<String>,
    pub duration_ms: u64,
}

impl CaseResult {
    /// An outcome that diverges from the manifest's expectations:
    /// an unlisted failure, or a pass of a listed expected failure.
    pub fn unexpected(&self) -> bool {
        if self.expected_failure {
            self.passed
        } else {
            !self.passed
        }
    }
}

/// Aggregated counts of a test run.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Summary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub expected_failures: usize,
    pub unexpected: usize,
}

impl Summary {
    pub fn of(results: &[CaseResult]) -> Summary {
        let total = results.len();
        let passed = results.iter().filter(|r| r.passed).count();
        let expected_failures = results.iter().filter(|r| r.expected_failure).count();
        let unexpected = results.iter().filter(|r| r.unexpected()).count();
        Summary {
            total,
            passed,
            failed: total - passed,
            expected_failures,
            unexpected,
        }
    }
}

/// Result of writing an HTML report.
#[derive(Debug, Clone)]
pub struct ReportOutcome {
    pub report_path: PathBuf,
    pub summary: Summary,
}

fn default_output_root() -> PathBuf {
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target"));
    target_dir.join("rom_tests")
}

/// Serialize results as a single JSON document (printed to stdout).
pub fn write_json(manifest_name: &str, results: &[CaseResult]) -> String {
    let summary = Summary::of(results);
    let doc = serde_json::json!({
        "manifest": manifest_name,
        "summary": summary,
        "results": results,
    });
    serde_json::to_string_pretty(&doc).expect("result serialization cannot fail")
}

/// Write an HTML report with screenshots, diff images and check details.
pub fn write_html_report(
    output_dir: Option<&Path>,
    title: &str,
    case_results: &[CaseResult],
) -> Result<ReportOutcome, RomTestError> {
    let dir = match output_dir {
        Some(d) => d.to_path_buf(),
        None => default_output_root(),
    };
    fs::create_dir_all(&dir).map_err(|e| {
        RomTestError::InvalidManifest(format!("failed to create report dir: {}", e))
    })?;
    fs::create_dir_all(dir.join("screenshots")).ok();
    fs::create_dir_all(dir.join("diffs")).ok();

    let summary = Summary::of(case_results);

    let mut html = String::new();
    write!(
        html,
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{}</title>\
         <style>\
         body{{font-family:sans-serif;margin:2rem;background:#111827;color:#e5e7eb;}}\
         h1,h2,h3,h4{{color:#f9fafb;}}\
         .suite{{margin-top:2rem;padding-bottom:0.35rem;border-bottom:1px solid #374151;}}\
         table{{border-collapse:collapse;width:100%;margin:1rem 0;}}\
         th,td{{border:1px solid #374151;padding:0.5rem;vertical-align:top;}}\
         th{{background:#1f2937;text-align:left;}}\
         .pass{{color:#10b981;font-weight:700;}}\
         .fail{{color:#f87171;font-weight:700;}}\
         .expected{{color:#fbbf24;font-weight:700;}}\
         .error{{color:#f87171;}}\
         .tag{{background:#1f2937;color:#9ca3af;border-radius:4px;padding:0 0.35rem;margin-right:0.25rem;font-size:0.8rem;}}\
         .screenshot{{max-width:320px;margin-top:0.5rem;border:1px solid #4b5563;}}\
         .checks{{margin:0.25rem 0 0 1rem;padding:0;font-size:0.85rem;}}\
         details{{margin-top:0.25rem;}}\
         summary{{cursor:pointer;color:#9ca3af;}}\
         </style></head><body>\
         <h1>{}</h1><p>{} passed, {} failed, {} unexpected</p>",
        title, title, summary.passed, summary.failed, summary.unexpected
    )
    .ok();

    for (suite, cases) in &group_by_suite(case_results) {
        let suite_passed = cases.iter().filter(|c| c.passed).count();
        write!(
            html,
            "<h2 class=\"suite\">{} <span class=\"{}\">{} / {}</span></h2>\
             <table><tr><th>ID</th><th>Model</th><th>Description</th><th>Tags</th><th>Result</th><th>Details</th></tr>",
            suite,
            if suite_passed == cases.len() { "pass" } else { "fail" },
            suite_passed,
            cases.len()
        )
        .ok();
        for case in cases {
            write_case_row(&mut html, case);
        }
        write!(html, "</table>").ok();
    }

    write!(html, "</body></html>").ok();

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let file_name = format!("{}-{}.html", title, timestamp);
    let report_path = dir.join(&file_name);
    fs::write(&report_path, &html)
        .map_err(|e| RomTestError::InvalidManifest(format!("failed to write report: {}", e)))?;

    // Also write as latest.html
    let latest_path = dir.join("latest.html");
    let _ = fs::write(&latest_path, &html);

    eprintln!("Report written to {}", report_path.display());
    Ok(ReportOutcome {
        report_path,
        summary,
    })
}

fn group_by_suite(case_results: &[CaseResult]) -> Vec<(String, Vec<&CaseResult>)> {
    let mut groups: Vec<(String, Vec<&CaseResult>)> = Vec::new();
    for result in case_results {
        if let Some((_, cases)) = groups.iter_mut().find(|(name, _)| name == &result.suite) {
            cases.push(result);
        } else {
            groups.push((result.suite.clone(), vec![result]));
        }
    }
    groups
}

fn write_case_row(html: &mut String, case: &CaseResult) {
    let (status, label) = match (case.passed, case.expected_failure) {
        (true, false) => ("pass", "PASS"),
        (true, true) => ("fail", "UNEXPECTED PASS"),
        (false, true) => ("expected", "EXPECTED FAIL"),
        (false, false) => ("fail", "FAIL"),
    };
    let tags = case
        .tags
        .iter()
        .map(|tag| format!("<span class=\"tag\">{}</span>", tag))
        .collect::<String>();
    write!(
        html,
        "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"{}\">{}</td><td>",
        case.id, case.model, case.description, tags, status, label
    )
    .ok();

    if let Some(ref err) = case.error {
        write!(html, "<div class=\"error\">{}</div>", err).ok();
    }
    if !case.checks.is_empty() {
        write!(
            html,
            "<details><summary>Checks</summary><ul class=\"checks\">"
        )
        .ok();
        for check in &case.checks {
            let mark = if check.passed { "PASS" } else { "FAIL" };
            write!(
                html,
                "<li>{}: {} — expected <code>{}</code>, actual <code>{}</code></li>",
                mark, check.name, check.expected, check.actual
            )
            .ok();
        }
        write!(html, "</ul></details>").ok();
    }
    if let Some(shot) = &case.screenshot {
        write!(
            html,
            "<details><summary>Screenshot</summary><img class=\"screenshot\" src=\"screenshots/{}\" alt=\"screenshot\"></details>",
            shot
        )
        .ok();
    }
    if let Some(diff) = &case.diff_image {
        write!(
            html,
            "<details><summary>Diff (actual | reference | differences)</summary><img class=\"screenshot\" src=\"diffs/{}\" alt=\"diff\"></details>",
            diff
        )
        .ok();
    }
    write!(html, "</td></tr>").ok();
}
