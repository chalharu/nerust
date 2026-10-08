use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use super::{Report, ReportRenderer, hex_preview, suite_name};
use crate::{
    error::RomTestError,
    results::{
        CaseOutcome, CaseValidation, LogCheck, MemoryCheck, RegisterCheck, ScreenCheck, SerialCheck,
    },
};

/// HTML report renderer: one `index.html` plus per-frame screenshots.
/// The only [`ReportRenderer`] today; new formats (JSON, markdown)
/// implement the trait without touching data generation.
pub struct HtmlReportRenderer;

impl ReportRenderer for HtmlReportRenderer {
    fn render(&self, report: &Report<'_>, output_dir: &Path) -> Result<PathBuf, RomTestError> {
        fs::create_dir_all(output_dir).map_err(|source| RomTestError::CreateDirectory {
            path: output_dir.to_path_buf(),
            source,
        })?;
        let screenshots_dir = output_dir.join("screenshots");
        fs::create_dir_all(&screenshots_dir).map_err(|source| RomTestError::CreateDirectory {
            path: screenshots_dir.clone(),
            source,
        })?;

        let mut html = String::new();

        let summary = &report.summary;
        let outcomes = report.outcomes;

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
         .expected{{color:#fbbf24;font-weight:700;}}\
         .case{{margin-bottom:2rem;padding:1rem;border:1px solid #374151;border-radius:0.5rem;background:#0f172a;}}\
         .thumb{{max-width:256px;height:auto;border:1px solid #374151;background:#000;}}\
         code{{white-space:nowrap;}}\
         ul{{margin:0.5rem 0 0 1.25rem;}}\
         </style></head><body>",
        escape_html(report.title)
    )
    .unwrap();

        write!(
        html,
        "<h1>{}</h1><p>Total cases: {} / passed: <span class=\"pass\">{}</span> / failed: <span class=\"fail\">{}</span> / expected-fail: <span class=\"expected\">{}</span> / ignored: {}</p>",
        escape_html(report.title),
        outcomes.len(),
        summary.passed,
        summary.failed,
        summary.expected_failed,
        summary.ignored
    )
    .unwrap();

        // Run metadata: when the run started (UTC), then the counts.
        write!(
            html,
            "<h2>Run</h2><table><tbody>\
         <tr><th>Started (UTC)</th><td><code>{}</code></td></tr>\
         <tr><th>Cases</th><td>{}</td></tr>\
         </tbody></table>",
            format_system_time(report.started_at),
            outcomes.len(),
        )
        .unwrap();

        // Overview: one row per case so the whole run reads as a table.
        write_overview(&mut html, outcomes);
        write_details(&mut html, outcomes, output_dir)?;

        html.push_str("</body></html>");
        let report_path = output_dir.join("index.html");
        fs::write(&report_path, html).map_err(|source| RomTestError::WriteFile {
            path: report_path.clone(),
            source,
        })?;

        Ok(report_path)
    }
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn write_log_checks(html: &mut String, checks: &[LogCheck]) {
    if checks.is_empty() {
        return;
    }
    html.push_str(
        "<h4>Log checks</h4><table><thead><tr>                         <th>Frame</th><th>Channel</th><th>End</th><th>Fail names</th><th>Missing allowed</th><th>Unexpected</th><th>Status</th>                         </tr></thead><tbody>",
    );
    for check in checks {
        let status_class = if check.passed() { "pass" } else { "fail" };
        let status_label = if check.passed() { "PASS" } else { "FAIL" };
        write!(
            html,
            "<tr><td>{}</td><td><code>{}</code></td>                             <td><code>{}</code></td><td><code>{}</code></td>                             <td><code>{}</code></td><td><code>{}</code></td>                             <td class=\"{}\">{}</td></tr>",
            check.frame,
            escape_html(&check.channel),
            escape_html(&check.expected_end),
            escape_html(&check.fail_names.join(", ")),
            escape_html(&check.missing_allowed.join(", ")),
            escape_html(&check.unexpected_fail.join(", ")),
            status_class,
            status_label
        )
        .unwrap();
    }
    html.push_str("</tbody></table>");
}

fn write_screen_checks(
    html: &mut String,
    case_id: &str,
    checks: &[ScreenCheck],
    output_dir: &Path,
) -> Result<(), RomTestError> {
    if checks.is_empty() {
        return Ok(());
    }
    html.push_str(
        "<h4>Screen checks</h4><table><thead><tr>\
         <th>Frame</th><th>Expected</th><th>Actual</th><th>Status</th><th>Screenshot</th>\
         </tr></thead><tbody>",
    );
    for (index, check) in checks.iter().enumerate() {
        let screenshot_rel = write_screenshot(case_id, output_dir, index, check)?;
        let status_class = if check.passed() { "pass" } else { "fail" };
        let status_label = if check.passed() { "PASS" } else { "FAIL" };
        write!(
            html,
            "<tr><td>{}</td><td><code>0x{:016X}</code></td><td><code>0x{:016X}</code></td>\
             <td class=\"{}\">{}</td><td>",
            check.frame, check.expected_hash, check.actual_hash, status_class, status_label
        )
        .unwrap();
        if let Some(relative) = screenshot_rel {
            write!(
                html,
                "<a href=\"{}\"><img class=\"thumb\" src=\"{}\" alt=\"{} frame {}\"></a>",
                escape_html(&relative),
                escape_html(&relative),
                escape_html(case_id),
                check.frame
            )
            .unwrap();
        } else {
            html.push('—');
        }
        html.push_str("</td></tr>");
    }
    html.push_str("</tbody></table>");
    Ok(())
}

/// Persist one screen-check PNG under `screenshots/<case>/`, returning
/// the relative link. `None` when the check carries no bytes.
fn write_screenshot(
    case_id: &str,
    output_dir: &Path,
    index: usize,
    check: &ScreenCheck,
) -> Result<Option<String>, RomTestError> {
    let Some(bytes) = &check.screenshot_png else {
        return Ok(None);
    };
    let relative = format!(
        "screenshots/{}/frame-{:06}-{:02}.png",
        sanitize_for_path(case_id),
        check.frame,
        index + 1
    );
    let absolute = output_dir.join(&relative);
    if let Some(parent) = absolute.parent() {
        fs::create_dir_all(parent).map_err(|source| RomTestError::CreateDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(&absolute, bytes).map_err(|source| RomTestError::WriteFile {
        path: absolute.clone(),
        source,
    })?;
    Ok(Some(relative))
}

/// Overview: one row per case so the whole run reads as a table.
/// Status / Case / System / Suite / Target / ROM / Frames / Time.
/// Detail sections grouped by target category, in run order.
fn write_details(
    html: &mut String,
    outcomes: &[CaseOutcome],
    output_dir: &Path,
) -> Result<(), RomTestError> {
    let mut current_category = None;
    for outcome in outcomes {
        let category = outcome.category();
        if current_category != Some(category) {
            current_category = Some(category);
            write!(
                html,
                "<h2 class=\"category\">{}</h2>",
                escape_html(category.label())
            )
            .unwrap();
        }

        match outcome {
            CaseOutcome::Completed(validation) => {
                write_completed(html, validation, output_dir)?;
            }
            CaseOutcome::InternalError {
                case_id,
                description,
                rom,
                message,
                ..
            } => {
                write!(
                    html,
                    "<section class=\"case\"><h3>{}</h3><p>{}</p>\
                     <p>Status: <span class=\"fail\">ERROR</span></p>\
                     <p>ROM: <code>{}</code></p><p>{}</p></section>",
                    escape_html(case_id),
                    escape_html(description),
                    escape_html(rom),
                    escape_html(message)
                )
                .unwrap();
            }
            CaseOutcome::Skipped {
                case_id,
                description,
                rom,
                reason,
                ..
            } => {
                write!(
                    html,
                    "<section class=\"case\"><h3>{}</h3><p>{}</p>\
                     <p>Status: <span>IGNORED</span></p>\
                     <p>ROM: <code>{}</code></p><p>{}</p></section>",
                    escape_html(case_id),
                    escape_html(description),
                    escape_html(rom),
                    escape_html(reason)
                )
                .unwrap();
            }
        }
    }
    Ok(())
}

fn write_completed(
    html: &mut String,
    validation: &CaseValidation,
    output_dir: &Path,
) -> Result<(), RomTestError> {
    let (status_class, status_label) = completed_status(validation);
    write!(
        html,
        "<section class=\"case\"><h3>{}</h3><p>{}</p>\
         <p>Status: <span class=\"{}\">{}</span></p>\
         <p>System: <code>{}</code> / Suite: <code>{}</code> / Elapsed: <code>{}</code></p>\
         <p>ROM: <code>{}</code></p>\
         <p>Frames: {} / Final screen hash: <code>0x{:016X}</code></p>",
        escape_html(&validation.case_id),
        escape_html(&validation.description),
        status_class,
        status_label,
        escape_html(validation.system),
        escape_html(suite_name(&validation.rom)),
        format_elapsed(validation.elapsed),
        escape_html(&validation.rom),
        validation.frames,
        validation.final_screen_hash
    )
    .unwrap();

    write!(
        html,
        "<p>Audio ({} Hz): samples=<code>{}</code> hash=<code>0x{:016X}</code>",
        validation.audio.sample_rate, validation.audio.samples, validation.audio.hash
    )
    .unwrap();
    if let Some(expected) = &validation.audio.expected {
        write!(
            html,
            " expected samples=<code>{}</code> expected hash=<code>0x{:016X}</code>",
            expected.samples, expected.hash
        )
        .unwrap();
    }
    html.push_str("</p>");

    write_bullet_list(html, "Failures", &validation.failures);
    write_bullet_list(html, "Stale (graduate!)", &validation.stale_notes);
    write_log_checks(html, &validation.log_checks);
    write_screen_checks(
        html,
        &validation.case_id,
        &validation.screen_checks,
        output_dir,
    )?;
    write_memory_checks(html, &validation.memory_checks);
    write_register_checks(html, &validation.register_checks);
    write_serial_checks(html, &validation.serial_checks);

    html.push_str("</section>");
    Ok(())
}

fn write_bullet_list(html: &mut String, heading: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    write!(html, "<h4>{heading}</h4><ul>").unwrap();
    for item in items {
        write!(html, "<li>{}</li>", escape_html(item)).unwrap();
    }
    html.push_str("</ul>");
}

fn write_overview(html: &mut String, outcomes: &[CaseOutcome]) {
    html.push_str(
        "<h2>Overview</h2><table><thead><tr>\
         <th>Status</th><th>Case</th><th>System</th><th>Suite</th>\
         <th>Target</th><th>ROM</th><th>Frames</th><th>Time</th>\
         </tr></thead><tbody>",
    );
    for outcome in outcomes {
        write_overview_row(html, outcome);
    }
    html.push_str("</tbody></table>");
}

fn write_overview_row(html: &mut String, outcome: &CaseOutcome) {
    let (status_class, status_label) = overview_status(outcome);
    let (case_id, category, rom, system, frames, elapsed) = match outcome {
        CaseOutcome::Completed(validation) => (
            validation.case_id.as_str(),
            validation.category,
            validation.rom.as_str(),
            validation.system,
            validation.frames.to_string(),
            format_elapsed(validation.elapsed),
        ),
        CaseOutcome::InternalError {
            case_id,
            category,
            rom,
            ..
        } => (
            case_id.as_str(),
            *category,
            rom.as_str(),
            "—",
            "—".to_string(),
            "—".to_string(),
        ),
        CaseOutcome::Skipped {
            case_id,
            category,
            rom,
            ..
        } => (
            case_id.as_str(),
            *category,
            rom.as_str(),
            "—",
            "—".to_string(),
            "—".to_string(),
        ),
    };
    write!(
        html,
        "<tr><td class=\"{}\">{}</td><td><code>{}</code></td><td>{}</td>\
         <td><code>{}</code></td><td>{}</td><td><code>{}</code></td>\
         <td>{}</td><td>{}</td></tr>",
        status_class,
        status_label,
        escape_html(case_id),
        escape_html(system),
        escape_html(suite_name(rom)),
        escape_html(category.label()),
        escape_html(rom),
        frames,
        elapsed
    )
    .unwrap();
}

fn write_memory_checks(html: &mut String, checks: &[MemoryCheck]) {
    if checks.is_empty() {
        return;
    }
    html.push_str(
        "<h4>Memory checks</h4><table><thead><tr>\
         <th>Frame</th><th>Address</th><th>Expected</th><th>Actual</th><th>Expected bus</th><th>Actual bus</th><th>Status</th>\
         </tr></thead><tbody>",
    );
    for check in checks {
        let status_class = if check.passed() { "pass" } else { "fail" };
        let status_label = if check.passed() { "PASS" } else { "FAIL" };
        write!(
            html,
            "<tr><td>{}</td><td><code>0x{:04X}</code></td><td><code>0x{:02X}</code></td>\
             <td><code>0x{:02X}</code></td><td>{}</td><td>{}</td><td class=\"{}\">{}</td></tr>",
            check.frame,
            check.address,
            check.expected_value,
            check.actual_value,
            bus_label(check.expected_open_bus),
            bus_label(check.actual_open_bus),
            status_class,
            status_label
        )
        .unwrap();
    }
    html.push_str("</tbody></table>");
}

fn bus_label(open_bus: bool) -> &'static str {
    if open_bus { "open bus" } else { "mapped RAM" }
}

fn write_register_checks(html: &mut String, checks: &[RegisterCheck]) {
    if checks.is_empty() {
        return;
    }
    html.push_str(
        "<h4>Register checks</h4><table><thead><tr>\
         <th>Frame</th><th>Register</th><th>Expected</th><th>Actual</th><th>Status</th>\
         </tr></thead><tbody>",
    );
    for check in checks {
        let status_class = if check.passed() { "pass" } else { "fail" };
        let status_label = if check.passed() { "PASS" } else { "FAIL" };
        write!(
            html,
            "<tr><td>{}</td><td><code>{}</code></td><td><code>0x{:X}</code></td>\
             <td><code>0x{:X}</code></td><td class=\"{}\">{}</td></tr>",
            check.frame,
            check.name,
            check.expected_value,
            check.actual_value,
            status_class,
            status_label
        )
        .unwrap();
    }
    html.push_str("</tbody></table>");
}

fn write_serial_checks(html: &mut String, checks: &[SerialCheck]) {
    if checks.is_empty() {
        return;
    }
    html.push_str(
        "<h4>Serial checks</h4><table><thead><tr>\
         <th>Frame</th><th>Channel</th><th>Expected</th><th>Actual</th><th>Status</th>\
         </tr></thead><tbody>",
    );
    for check in checks {
        let status_class = if check.passed() { "pass" } else { "fail" };
        let status_label = if check.passed() { "PASS" } else { "FAIL" };
        write!(
            html,
            "<tr><td>{}</td><td><code>{}</code></td>\
             <td><code>{}</code></td><td><code>{}</code></td>\
             <td class=\"{}\">{}</td></tr>",
            check.frame,
            check.channel,
            hex_preview(&check.expected_bytes),
            hex_preview(&check.actual_bytes),
            status_class,
            status_label
        )
        .unwrap();
    }
    html.push_str("</tbody></table>");
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

/// Overview-table status cell shared with the detail sections:
/// pass / expected-fail / fail for completed, fail for internal
/// errors, unstyled IGNORED for skips.
fn overview_status(outcome: &CaseOutcome) -> (&'static str, &'static str) {
    match outcome {
        CaseOutcome::Completed(validation) => completed_status(validation),
        CaseOutcome::InternalError { .. } => ("fail", "ERROR"),
        CaseOutcome::Skipped { .. } => ("", "IGNORED"),
    }
}

fn completed_status(validation: &CaseValidation) -> (&'static str, &'static str) {
    if validation.passed() {
        ("pass", "PASS")
    } else if validation.is_expected_failure() {
        ("expected", "EXPECTED FAIL")
    } else {
        ("fail", "FAIL")
    }
}

/// Elapsed display: millis below one second, seconds with millis above.
fn format_elapsed(elapsed: Duration) -> String {
    if elapsed.as_secs() > 0 {
        format!("{:.3}s", elapsed.as_secs_f64())
    } else {
        format!("{}ms", elapsed.as_millis())
    }
}

/// `SystemTime` as `YYYY-MM-DD HH:MM:SS UTC` without external date
/// crates (days-to-civil, Howard Hinnant's algorithm). Pre-epoch
/// times cannot occur for run starts; they render as unknown.
fn format_system_time(started_at: SystemTime) -> String {
    let secs = match started_at.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs(),
        Err(_) => return "unknown".to_string(),
    };
    let (year, month, day) = civil_from_days((secs / 86_400) as i64);
    let clock = secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        clock / 3600,
        (clock % 3600) / 60,
        clock % 60
    )
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_pair = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_pair + 2) / 5 + 1) as u32;
    let month = if month_pair < 10 {
        month_pair + 3
    } else {
        month_pair - 9
    } as u32;
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::{
        manifest::{AudioExpectation, RomCategory},
        report::{ReportRenderer, summarize},
        results::{
            AudioObservation, CaseOutcome, CaseValidation, LogCheck, MemoryCheck, RegisterCheck,
            ScreenCheck, SerialCheck,
        },
    };

    static REPORT_SEQ: AtomicU64 = AtomicU64::new(0);

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nerust-report-test-{}-{}-{}",
            std::process::id(),
            REPORT_SEQ.fetch_add(1, Ordering::SeqCst),
            name
        ));
        fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    fn passing_case(id: &str) -> CaseOutcome {
        CaseOutcome::Completed(Box::new(CaseValidation {
            case_id: id.to_string(),
            category: RomCategory::Cpu,
            description: format!("{id} description"),
            rom: "nes-test-roms/blargg_suite/01.case.nes".to_string(),
            system: "NES",
            elapsed: std::time::Duration::from_millis(850),
            frames: 10,
            final_screen_hash: 0x1234,
            screen_checks: vec![ScreenCheck {
                frame: 10,
                expected_hash: 0x1234,
                actual_hash: 0x1234,
                screenshot_png: Some(vec![0x89, 0x50, 0x4E, 0x47]),
            }],
            memory_checks: vec![MemoryCheck {
                frame: 10,
                address: 0x0200,
                expected_value: 0x42,
                actual_value: 0x42,
                expected_open_bus: false,
                actual_open_bus: false,
            }],
            register_checks: vec![RegisterCheck {
                frame: 10,
                name: "a".to_string(),
                expected_value: 1,
                actual_value: 1,
            }],
            serial_checks: vec![SerialCheck {
                frame: 10,
                channel: "serial".to_string(),
                expected_bytes: vec![0x50],
                actual_bytes: vec![0x50],
            }],
            log_checks: vec![LogCheck {
                frame: 10,
                channel: "serial".to_string(),
                expected_end: "END".to_string(),
                end_found: true,
                allowed_fail: Vec::new(),
                fail_names: Vec::new(),
                missing_allowed: Vec::new(),
                unexpected_fail: Vec::new(),
            }],
            audio: AudioObservation {
                sample_rate: 44100,
                samples: 100,
                hash: 0xABCD,
                expected: Some(AudioExpectation {
                    sample_rate: 44100,
                    samples: 100,
                    hash: 0xABCD,
                }),
            },
            failures: Vec::new(),
            expected_failure: false,
            stale_notes: Vec::new(),
        }))
    }

    fn render_html(
        dir: &std::path::Path,
        title: &'static str,
        outcomes: &[CaseOutcome],
    ) -> PathBuf {
        let renderer = HtmlReportRenderer;
        let summary = summarize(outcomes);
        let report = crate::report::Report {
            title,
            summary: &summary,
            started_at: std::time::UNIX_EPOCH,
            outcomes,
        };
        renderer.render(&report, dir).expect("report writes")
    }

    #[test]
    fn html_report_counts_and_escapes() {
        let dir = scratch_dir("counts");
        let outcomes = vec![
            passing_case("nes.pass"),
            CaseOutcome::InternalError {
                case_id: "nes.broken".to_string(),
                category: RomCategory::Ppu,
                description: "broken <desc>".to_string(),
                rom: "broken.nes".to_string(),
                message: "boom & bust".to_string(),
            },
            CaseOutcome::Skipped {
                case_id: "nes.skip".to_string(),
                category: RomCategory::Apu,
                description: "skip".to_string(),
                rom: "skip.nes".to_string(),
                reason: "not in CI".to_string(),
            },
        ];
        let summary = summarize(&outcomes);
        let report_path = render_html(&dir, "Title <&>", &outcomes);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.ignored, 1);
        assert_eq!(summary.expected_failed, 0);
        let html = fs::read_to_string(&report_path).expect("report readable");
        assert!(html.contains("Title &lt;&amp;&gt;"));
        assert!(html.contains("boom &amp; bust"));
        assert!(html.contains("IGNORED"));
        // Run header carries the start time; overview carries one row
        // per case with system / suite / target / time columns.
        assert!(html.contains("1970-01-01 00:00:00 UTC"));
        assert!(html.contains("<td>NES</td>"));
        assert!(html.contains("<code>blargg_suite</code>"));
        assert!(html.contains("CPU Tests"));
        assert!(html.contains("<td>850ms</td>"));
        // Screenshot bytes land under the sanitized case directory.
        assert!(
            dir.join("screenshots/nes_pass/frame-000010-01.png")
                .exists()
        );
    }

    #[test]
    fn html_report_marks_expected_failures() {
        let dir = scratch_dir("expected");
        let mut tracked = match passing_case("nes.tracked") {
            CaseOutcome::Completed(validation) => *validation,
            _ => unreachable!(),
        };
        tracked.failures.push("mismatch".to_string());
        tracked.expected_failure = true;
        let mut stale = match passing_case("nes.stale") {
            CaseOutcome::Completed(validation) => *validation,
            _ => unreachable!(),
        };
        stale.failures.push("mismatch".to_string());
        stale.expected_failure = true;
        stale.stale_notes.push("graduate me".to_string());
        let outcomes = [
            CaseOutcome::Completed(Box::new(tracked)),
            CaseOutcome::Completed(Box::new(stale)),
        ];
        let summary = summarize(&outcomes);
        let report_path = render_html(&dir, "t", &outcomes);
        assert_eq!(summary.passed, 0);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.expected_failed, 1);
        let html = fs::read_to_string(&report_path).expect("report readable");
        assert!(html.contains("EXPECTED FAIL"));
        assert!(html.contains("Stale (graduate!)"));
    }

    #[test]
    fn time_helpers_format_without_date_crate() {
        assert_eq!(
            format_elapsed(Duration::from_millis(850)),
            "850ms".to_string()
        );
        assert_eq!(
            format_elapsed(Duration::from_millis(12_345)),
            "12.345s".to_string()
        );
        assert_eq!(
            format_system_time(SystemTime::UNIX_EPOCH),
            "1970-01-01 00:00:00 UTC".to_string()
        );
        // 2026-10-07 00:00:00 UTC, including a leap day round-trip.
        assert_eq!(
            format_system_time(SystemTime::UNIX_EPOCH + Duration::from_secs(1_791_331_200)),
            "2026-10-07 00:00:00 UTC".to_string()
        );
        assert_eq!(civil_from_days(60), (1970, 3, 2));
    }
}
