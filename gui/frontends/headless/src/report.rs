use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use super::{error::RomTestError, results::CaseOutcome};

#[derive(Debug, Clone)]
pub struct ReportSummary {
    pub report_path: PathBuf,
    pub passed: usize,
    pub failed: usize,
    pub ignored: usize,
    pub expected_failed: usize,
}

pub fn default_output_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/rom-tests")
}

/// Compact hex preview for serial bytes: full text when short,
/// truncated with a length otherwise.
pub fn hex_preview(bytes: &[u8]) -> String {
    const PREVIEW: usize = 64;
    let shown = &bytes[..bytes.len().min(PREVIEW)];
    let mut text = String::with_capacity(shown.len() * 2);
    for byte in shown {
        text.push_str(&format!("{byte:02X}"));
    }
    if bytes.len() > PREVIEW {
        text.push_str(&format!("…({} bytes)", bytes.len()));
    }
    text
}

pub fn write_html_report(
    output_dir: &Path,
    title: &str,
    outcomes: &[CaseOutcome],
) -> Result<ReportSummary, RomTestError> {
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
    let passed = outcomes.iter().filter(|outcome| outcome.passed()).count();
    let ignored = outcomes
        .iter()
        .filter(|outcome| outcome.is_skipped())
        .count();
    let expected_failed = outcomes
        .iter()
        .filter(|outcome| outcome.is_expected_failure())
        .count();
    let failed = outcomes
        .len()
        .saturating_sub(passed)
        .saturating_sub(ignored)
        .saturating_sub(expected_failed);

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
        escape_html(title)
    )
    .unwrap();

    write!(
        html,
        "<h1>{}</h1><p>Total cases: {} / passed: <span class=\"pass\">{}</span> / failed: <span class=\"fail\">{}</span> / expected-fail: <span class=\"expected\">{}</span> / ignored: {}</p>",
        escape_html(title),
        outcomes.len(),
        passed,
        failed,
        expected_failed,
        ignored
    )
    .unwrap();

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
                let (status_class, status_label) = if validation.passed() {
                    ("pass", "PASS")
                } else if validation.is_expected_failure() {
                    ("expected", "EXPECTED FAIL")
                } else {
                    ("fail", "FAIL")
                };
                write!(
                    html,
                    "<section class=\"case\"><h3>{}</h3><p>{}</p>\
                     <p>Status: <span class=\"{}\">{}</span></p>\
                     <p>ROM: <code>{}</code></p>\
                     <p>Frames: {} / Final screen hash: <code>0x{:016X}</code></p>",
                    escape_html(&validation.case_id),
                    escape_html(&validation.description),
                    status_class,
                    status_label,
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

                if !validation.failures.is_empty() {
                    html.push_str("<h4>Failures</h4><ul>");
                    for failure in &validation.failures {
                        write!(html, "<li>{}</li>", escape_html(failure)).unwrap();
                    }
                    html.push_str("</ul>");
                }
                if !validation.stale_notes.is_empty() {
                    html.push_str("<h4>Stale (graduate!)</h4><ul>");
                    for note in &validation.stale_notes {
                        write!(html, "<li>{}</li>", escape_html(note)).unwrap();
                    }
                    html.push_str("</ul>");
                }
                if !validation.log_checks.is_empty() {
                    html.push_str(
                        "<h4>Log checks</h4><table><thead><tr>                         <th>Frame</th><th>Channel</th><th>End</th><th>Fail names</th><th>Missing allowed</th><th>Unexpected</th><th>Status</th>                         </tr></thead><tbody>",
                    );
                    for check in &validation.log_checks {
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

                if !validation.screen_checks.is_empty() {
                    html.push_str(
                        "<h4>Screen checks</h4><table><thead><tr>\
                         <th>Frame</th><th>Expected</th><th>Actual</th><th>Status</th><th>Screenshot</th>\
                         </tr></thead><tbody>",
                    );
                    for (index, check) in validation.screen_checks.iter().enumerate() {
                        let screenshot_rel = if let Some(bytes) = &check.screenshot_png {
                            let relative = format!(
                                "screenshots/{}/frame-{:06}-{:02}.png",
                                sanitize_for_path(&validation.case_id),
                                check.frame,
                                index + 1
                            );
                            let absolute = output_dir.join(&relative);
                            if let Some(parent) = absolute.parent() {
                                fs::create_dir_all(parent).map_err(|source| {
                                    RomTestError::CreateDirectory {
                                        path: parent.to_path_buf(),
                                        source,
                                    }
                                })?;
                            }
                            fs::write(&absolute, bytes).map_err(|source| {
                                RomTestError::WriteFile {
                                    path: absolute.clone(),
                                    source,
                                }
                            })?;
                            Some(relative)
                        } else {
                            None
                        };
                        let status_class = if check.passed() { "pass" } else { "fail" };
                        let status_label = if check.passed() { "PASS" } else { "FAIL" };
                        write!(
                            html,
                            "<tr><td>{}</td><td><code>0x{:016X}</code></td><td><code>0x{:016X}</code></td>\
                             <td class=\"{}\">{}</td><td>",
                            check.frame,
                            check.expected_hash,
                            check.actual_hash,
                            status_class,
                            status_label
                        )
                        .unwrap();
                        if let Some(relative) = screenshot_rel {
                            write!(
                                html,
                                "<a href=\"{}\"><img class=\"thumb\" src=\"{}\" alt=\"{} frame {}\"></a>",
                                escape_html(&relative),
                                escape_html(&relative),
                                escape_html(&validation.case_id),
                                check.frame
                            )
                            .unwrap();
                        } else {
                            html.push('—');
                        }
                        html.push_str("</td></tr>");
                    }
                    html.push_str("</tbody></table>");
                }

                if !validation.memory_checks.is_empty() {
                    html.push_str(
                        "<h4>Memory checks</h4><table><thead><tr>\
                         <th>Frame</th><th>Address</th><th>Expected</th><th>Actual</th><th>Expected bus</th><th>Actual bus</th><th>Status</th>\
                         </tr></thead><tbody>",
                    );
                    for check in &validation.memory_checks {
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
                            if check.expected_open_bus {
                                "open bus"
                            } else {
                                "mapped RAM"
                            },
                            if check.actual_open_bus {
                                "open bus"
                            } else {
                                "mapped RAM"
                            },
                            status_class,
                            status_label
                        )
                        .unwrap();
                    }
                    html.push_str("</tbody></table>");
                }
                if !validation.register_checks.is_empty() {
                    html.push_str(
                        "<h4>Register checks</h4><table><thead><tr>\
                         <th>Frame</th><th>Register</th><th>Expected</th><th>Actual</th><th>Status</th>\
                         </tr></thead><tbody>",
                    );
                    for check in &validation.register_checks {
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
                if !validation.serial_checks.is_empty() {
                    html.push_str(
                        "<h4>Serial checks</h4><table><thead><tr>\
                         <th>Frame</th><th>Channel</th><th>Expected</th><th>Actual</th><th>Status</th>\
                         </tr></thead><tbody>",
                    );
                    for check in &validation.serial_checks {
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

                html.push_str("</section>");
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

    html.push_str("</body></html>");
    let report_path = output_dir.join("index.html");
    fs::write(&report_path, html).map_err(|source| RomTestError::WriteFile {
        path: report_path.clone(),
        source,
    })?;

    Ok(ReportSummary {
        report_path,
        passed,
        failed,
        ignored,
        expected_failed,
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::{
        manifest::{AudioExpectation, RomCategory},
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
            rom: format!("{id}.nes"),
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

    #[test]
    fn hex_preview_covers_empty_short_and_long() {
        assert_eq!(hex_preview(&[]), "");
        assert_eq!(hex_preview(b"Pass"), "50617373");
        let exact = vec![0xAB; 64];
        assert_eq!(hex_preview(&exact).len(), 128);
        assert!(!hex_preview(&exact).contains("bytes)"));
        let long = vec![0xAB; 65];
        let preview = hex_preview(&long);
        assert!(preview.starts_with(&"AB".repeat(64)));
        assert!(preview.ends_with("…(65 bytes)"));
    }

    #[test]
    fn default_output_root_points_at_target_rom_tests() {
        assert!(default_output_root().ends_with("target/rom-tests"));
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
        let summary = write_html_report(&dir, "Title <&>", &outcomes).expect("report writes");
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.ignored, 1);
        assert_eq!(summary.expected_failed, 0);
        let html = fs::read_to_string(&summary.report_path).expect("report readable");
        assert!(html.contains("Title &lt;&amp;&gt;"));
        assert!(html.contains("boom &amp; bust"));
        assert!(html.contains("IGNORED"));
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
        let summary = write_html_report(
            &dir,
            "t",
            &[
                CaseOutcome::Completed(Box::new(tracked)),
                CaseOutcome::Completed(Box::new(stale)),
            ],
        )
        .expect("report writes");
        assert_eq!(summary.passed, 0);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.expected_failed, 1);
        let html = fs::read_to_string(&summary.report_path).expect("report readable");
        assert!(html.contains("EXPECTED FAIL"));
        assert!(html.contains("Stale (graduate!)"));
    }
}
