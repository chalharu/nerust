use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::error::RomTestError;

/// Per-case test result.
#[derive(Debug, Clone)]
pub struct CaseResult {
    pub id: String,
    pub category: String,
    pub description: String,
    pub passed: bool,
    pub output: String,
    pub error: Option<String>,
    /// Optional screenshot paths (PNG) for each event.
    pub screenshots: Vec<String>,
}

/// Summary of a test run.
#[derive(Debug, Clone)]
pub struct ReportSummary {
    pub report_path: PathBuf,
    pub passed: usize,
    pub failed: usize,
}

fn default_output_root() -> PathBuf {
    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("target"));
    target_dir.join("rom_tests")
}

/// Write an HTML report with embedded screenshots.
pub fn write_html_report(
    output_dir: Option<&Path>,
    title: &str,
    case_results: &[CaseResult],
) -> Result<ReportSummary, RomTestError> {
    let dir = match output_dir {
        Some(d) => d.to_path_buf(),
        None => default_output_root(),
    };
    fs::create_dir_all(&dir).map_err(|e| {
        RomTestError::InvalidManifest(format!("failed to create report dir: {}", e))
    })?;

    let screenshots_dir = dir.join("screenshots");
    fs::create_dir_all(&screenshots_dir).ok();

    let passed = case_results.iter().filter(|r| r.passed).count();
    let failed = case_results.len() - passed;

    let mut html = String::new();
    write!(
        html,
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{}</title>\
         <style>\
         body{{font-family:sans-serif;margin:2rem;background:#111827;color:#e5e7eb;}}\
         h1,h2,h3,h4{{color:#f9fafb;}}\
         .category{{margin-top:2rem;padding-bottom:0.35rem;border-bottom:1px solid #374151;}}\
         table{{border-collapse:collapse;width:100%;margin:1rem 0;}}\
         th,td{{border:1px solid #374151;padding:0.5rem;vertical-align:top;}}\
         th{{background:#1f2937;text-align:left;}}\
         .pass{{color:#10b981;font-weight:700;}}\
         .fail{{color:#f87171;font-weight:700;}}\
         .error{{color:#f87171;}}\
         .screenshot{{max-width:320px;margin-top:0.5rem;border:1px solid #4b5563;}}\
         details{{margin-top:0.25rem;}}\
         summary{{cursor:pointer;color:#9ca3af;}}\
         </style></head><body>\
         <h1>{}</h1><p>{} passed, {} failed</p>",
        title, title, passed, failed
    )
    .ok();

    let mut by_category: std::collections::BTreeMap<&str, Vec<&CaseResult>> =
        std::collections::BTreeMap::new();
    for r in case_results {
        by_category.entry(r.category.as_str()).or_default().push(r);
    }

    for (category, cases) in &by_category {
        write!(html, "<h2 class=\"category\">{}</h2><table><tr><th>ID</th><th>Description</th><th>Result</th><th>Details</th></tr>", category).ok();
        for case in cases {
            let status = if case.passed { "pass" } else { "fail" };
            let label = if case.passed { "PASS" } else { "FAIL" };
            write!(
                html,
                "<tr><td>{}</td><td>{}</td><td class=\"{}\">{}</td><td>",
                case.id, case.description, status, label
            )
            .ok();

            if let Some(ref err) = case.error {
                write!(html, "<div class=\"error\">{}</div>", err).ok();
            }
            if !case.output.is_empty() {
                let sanitized = case
                    .output
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                write!(
                    html,
                    "<details><summary>Serial output</summary><pre>{}</pre></details>",
                    sanitized
                )
                .ok();
            }
            for (i, shot) in case.screenshots.iter().enumerate() {
                write!(
                    html,
                    "<details><summary>Event {} screenshot</summary><img class=\"screenshot\" src=\"{}\" alt=\"event {}\"></details>",
                    i, shot, i
                )
                .ok();
            }
            write!(html, "</td></tr>").ok();
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
    Ok(ReportSummary {
        report_path,
        passed,
        failed,
    })
}
