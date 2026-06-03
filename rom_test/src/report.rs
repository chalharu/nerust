// Copyright (c) 2018 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::error::RomTestError;
use super::results::CaseOutcome;
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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/rom-tests")
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
    let failed = outcomes.len().saturating_sub(passed);

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
        "<h1>{}</h1><p>Total cases: {} / passed: <span class=\"pass\">{}</span> / failed: <span class=\"fail\">{}</span></p>",
        escape_html(title),
        outcomes.len(),
        passed,
        failed
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
                let status_class = if validation.passed() { "pass" } else { "fail" };
                let status_label = if validation.passed() { "PASS" } else { "FAIL" };
                write!(
                    html,
                    "<section class=\"case\"><h3>{}</h3><p>{}</p>\
                     <p>Status: <span class=\"{}\">{}</span></p>\
                     <p>ROM: <code>{}</code></p>\
                     <p>Frames: {} / Steps: {} / Final screen hash: <code>0x{:016X}</code></p>",
                    escape_html(&validation.case_id),
                    escape_html(&validation.description),
                    status_class,
                    status_label,
                    escape_html(&validation.rom),
                    validation.frames,
                    validation.steps,
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

                if !validation.work_ram_checks.is_empty() {
                    html.push_str(
                        "<h4>Work RAM checks</h4><table><thead><tr>\
                         <th>Frame</th><th>Address</th><th>Expected</th><th>Actual</th><th>Status</th>\
                         </tr></thead><tbody>",
                    );
                    for check in &validation.work_ram_checks {
                        let status_class = if check.passed() { "pass" } else { "fail" };
                        let status_label = if check.passed() { "PASS" } else { "FAIL" };
                        write!(
                            html,
                            "<tr><td>{}</td><td><code>0x{:04X}</code></td><td><code>0x{:02X}</code></td>\
                             <td><code>0x{:02X}</code></td><td class=\"{}\">{}</td></tr>",
                            check.frame,
                            check.address,
                            check.expected_value,
                            check.actual_value,
                            status_class,
                            status_label
                        )
                        .unwrap();
                    }
                    html.push_str("</tbody></table>");
                }

                if !validation.cartridge_ram_checks.is_empty() {
                    html.push_str(
                        "<h4>Cartridge RAM checks</h4><table><thead><tr>\
                         <th>Frame</th><th>Address</th><th>Expected</th><th>Actual</th><th>Expected bus</th><th>Actual bus</th><th>Status</th>\
                         </tr></thead><tbody>",
                    );
                    for check in &validation.cartridge_ram_checks {
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

                if !validation.ppu_vram_checks.is_empty() {
                    html.push_str(
                        "<h4>PPU VRAM checks</h4><table><thead><tr>\
                         <th>Frame</th><th>Address</th><th>Expected</th><th>Actual</th><th>Status</th>\
                         </tr></thead><tbody>",
                    );
                    for check in &validation.ppu_vram_checks {
                        let status_class = if check.passed() { "pass" } else { "fail" };
                        let status_label = if check.passed() { "PASS" } else { "FAIL" };
                        write!(
                            html,
                            "<tr><td>{}</td><td><code>0x{:04X}</code></td><td><code>0x{:02X}</code></td>\
                             <td><code>0x{:02X}</code></td><td class=\"{}\">{}</td></tr>",
                            check.frame,
                            check.address,
                            check.expected_value,
                            check.actual_value,
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
