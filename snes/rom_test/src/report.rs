use crate::results::CaseOutcome;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ReportSummary {
    pub report_path: PathBuf,
    pub passed: usize,
    pub failed: usize,
}

pub fn default_output_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/snes-rom-tests")
}

pub fn write_html_report(
    output_dir: &Path,
    title: &str,
    outcomes: &[CaseOutcome],
) -> Result<ReportSummary, String> {
    fs::create_dir_all(output_dir)
        .map_err(|error| format!("failed to create `{}`: {error}", output_dir.display()))?;
    let screenshots_dir = output_dir.join("screenshots");
    fs::create_dir_all(&screenshots_dir)
        .map_err(|error| format!("failed to create `{}`: {error}", screenshots_dir.display()))?;

    let passed = outcomes.iter().filter(|outcome| outcome.passed()).count();
    let failed = outcomes.len().saturating_sub(passed);
    let mut html = String::new();

    write!(
        html,
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>{}</title>\
         <style>\
         body{{font-family:sans-serif;margin:2rem;background:#111827;color:#e5e7eb;}}\
         h1,h2{{color:#f9fafb;}}\
         .case{{margin:1rem 0;padding:1rem;border:1px solid #374151;border-radius:0.5rem;background:#0f172a;}}\
         .pass{{color:#10b981;font-weight:700;}}\
         .fail{{color:#f87171;font-weight:700;}}\
         .thumb{{max-width:256px;height:auto;border:1px solid #374151;background:#000;}}\
         code{{white-space:nowrap;}}\
         ul{{margin:0.5rem 0 0 1.25rem;}}\
         </style></head><body>",
        escape_html(title)
    )
    .unwrap();

    write!(
        html,
        "<h1>{}</h1><p>Total cases: {} / passed: <span class=\"pass\">{}</span> / failed: <span class=\"fail\">{}</span></p>",
        escape_html(title),
        outcomes.len(),
        passed,
        failed
    )
    .unwrap();

    for outcome in outcomes {
        match outcome {
            CaseOutcome::Completed(validation) => {
                let screenshot_rel = if let Some(bytes) = &validation.screenshot_png {
                    let relative =
                        format!("screenshots/{}.png", sanitize_for_path(&validation.case_id));
                    let absolute = output_dir.join(&relative);
                    fs::write(&absolute, bytes).map_err(|error| {
                        format!("failed to write `{}`: {error}", absolute.display())
                    })?;
                    Some(relative)
                } else {
                    None
                };
                let status_class = if validation.passed() { "pass" } else { "fail" };
                let status_label = if validation.passed() { "PASS" } else { "FAIL" };
                write!(
                    html,
                    "<section class=\"case\"><h2>{}</h2><p>{}</p>\
                     <p>Status: <span class=\"{}\">{}</span></p>\
                     <p>ROM: <code>{}</code></p>\
                     <p>Steps executed: {} / Final screen hash: <code>0x{:016X}</code></p>",
                    escape_html(&validation.case_id),
                    escape_html(&validation.description),
                    status_class,
                    status_label,
                    escape_html(&validation.rom),
                    validation.steps_executed,
                    validation.final_screen_hash
                )
                .unwrap();

                if let Some(relative) = screenshot_rel {
                    write!(
                        html,
                        "<p><a href=\"{}\"><img class=\"thumb\" src=\"{}\" alt=\"{} screenshot\"></a></p>",
                        escape_html(&relative),
                        escape_html(&relative),
                        escape_html(&validation.case_id)
                    )
                    .unwrap();
                } else {
                    html.push_str("<p>Screenshot: unavailable</p>");
                }

                if validation.failures.is_empty() {
                    html.push_str("<p>No assertion failures.</p>");
                } else {
                    html.push_str("<h3>Failures</h3><ul>");
                    for failure in &validation.failures {
                        write!(html, "<li>{}</li>", escape_html(failure)).unwrap();
                    }
                    html.push_str("</ul>");
                }
                html.push_str("</section>");
            }
            CaseOutcome::InternalError {
                case_id,
                description,
                rom,
                message,
            } => {
                write!(
                    html,
                    "<section class=\"case\"><h2>{}</h2><p>{}</p>\
                     <p>Status: <span class=\"fail\">ERROR</span></p>\
                     <p>ROM: <code>{}</code></p><p>{}</p></section>",
                    escape_html(case_id),
                    escape_html(description),
                    escape_html(rom),
                    escape_html(message)
                )
                .unwrap();
            }
        }
    }

    html.push_str("</body></html>");
    let report_path = output_dir.join("index.html");
    fs::write(&report_path, html)
        .map_err(|error| format!("failed to write `{}`: {error}", report_path.display()))?;

    Ok(ReportSummary {
        report_path,
        passed,
        failed,
    })
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn sanitize_for_path(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}
