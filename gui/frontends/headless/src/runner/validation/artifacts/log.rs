use std::collections::HashMap;

use super::{ValidationArtifacts, peek_serial};
use crate::{
    error::RomTestError,
    results::{LogCheck, ValidationOptions},
};

#[derive(Default)]
pub(super) struct LogArtifacts {
    pub(super) log: LogCheckArtifacts,
}

#[derive(Default)]
pub(in crate::runner::validation::artifacts) struct LogCheckArtifacts {
    pub(in crate::runner::validation::artifacts) checks: Vec<LogCheck>,
}

/// Expected log observation for one assertion: the cumulative
/// newline-delimited text on one channel must contain the exact
/// `end` marker line, and the `fail_prefix`-led test-name set must
/// equal `allowed_fail` exactly.
pub(in crate::runner::validation) struct ExpectedLog {
    pub(in crate::runner::validation) frame: u64,
    pub(in crate::runner::validation) channel: String,
    pub(in crate::runner::validation) end: String,
    pub(in crate::runner::validation) fail_prefix: String,
    pub(in crate::runner::validation) allowed_fail: Vec<String>,
}

/// Split cumulative channel bytes into lines. The core commits whole
/// lines (each with its own newline), so no partial-line hazard:
/// every element is one committed line.
fn split_lines(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .split('\n')
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Test names from `fail_prefix`-led lines, in transcript order.
/// Detail lines (`Got ...: FAIL`) never carry the prefix, so they
/// stay evidence-only, never matched.
fn fail_names(lines: &[String], fail_prefix: &str) -> Vec<String> {
    lines
        .iter()
        .filter_map(|line| line.strip_prefix(fail_prefix))
        .map(|name| name.trim_end().to_string())
        .collect()
}

impl ValidationArtifacts {
    pub(in crate::runner::validation) fn record_log_assert(
        &mut self,
        case_id: &str,
        serial: &HashMap<String, Vec<u8>>,
        options: ValidationOptions,
        expected: ExpectedLog,
    ) -> Result<(), RomTestError> {
        let actual = peek_serial(serial, &expected.channel)?;
        let lines = split_lines(actual);
        let end_found = lines.iter().any(|line| line == &expected.end);
        let names = fail_names(&lines, &expected.fail_prefix);
        let unexpected: Vec<String> = names
            .iter()
            .filter(|name| {
                !expected
                    .allowed_fail
                    .iter()
                    .any(|a| a.as_str() == name.as_str())
            })
            .cloned()
            .collect();
        let missing: Vec<String> = expected
            .allowed_fail
            .iter()
            .filter(|allowed| !names.iter().any(|name| name.as_str() == allowed.as_str()))
            .cloned()
            .collect();
        // Stale bypasses tolerance unconditionally (see results.rs):
        // allowed names are recorded in stale_notes, never failures.
        // Only evaluated when expectations were checked; capture just
        // records the observation.
        let mut stale_here = Vec::new();
        if options.check_expectations {
            if !end_found {
                self.failures.push(format!(
                    "{case_id}: log end marker missing at frame {} channel `{}` (expected \"{}\")",
                    expected.frame, expected.channel, expected.end
                ));
            }
            for name in &unexpected {
                self.failures.push(format!(
                    "{case_id}: unexpected log failure at frame {} channel `{}`: {name}",
                    expected.frame, expected.channel
                ));
            }
            // Allowed-but-present FAILs are real mismatches: recorded
            // (tolerated under the case flag) so a fully-as-expected
            // red transcript never looks clean. Without these, an
            // expected-failure case with zero `failures` would trip
            // the case-level stale rule.
            for name in &names {
                if expected.allowed_fail.iter().any(|a| a == name) {
                    self.failures.push(format!(
                        "{case_id}: known log failure at frame {} channel `{}`: {name} (tracked)",
                        expected.frame, expected.channel
                    ));
                }
            }
            for name in &missing {
                stale_here.push(format!(
                    "{case_id}: allowed log failure went stale at frame {} channel `{}`: {name} no longer fails — drop it from allowed_fail",
                    expected.frame, expected.channel
                ));
            }
        }
        let check = LogCheck {
            frame: expected.frame,
            channel: expected.channel,
            expected_end: expected.end,
            end_found,
            allowed_fail: expected.allowed_fail,
            fail_names: names,
            missing_allowed: missing,
            unexpected_fail: unexpected,
        };
        self.log.log.checks.push(check);
        // Stale notes ride out through finish(): record here needs the
        // case-level list, so stash on self for finish() to drain.
        self.stale_pending.extend(stale_here);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{fail_names, split_lines};

    #[test]
    fn split_skips_trailing_newline_without_fake_line() {
        assert_eq!(
            split_lines(b"BEGIN: X\nPASS: a\n"),
            vec!["BEGIN: X", "PASS: a"]
        );
        assert!(split_lines(b"").is_empty());
    }

    #[test]
    fn fail_names_extract_prefix_led_only() {
        let lines =
            split_lines(b"BEGIN: T\nFAIL: Foo\nFoo: Got 0x1 vs 0x2: FAIL\nPASS: Bar\nEND: 1/2\n");
        assert_eq!(fail_names(&lines, "FAIL: "), vec!["Foo"]);
    }

    #[test]
    fn empty_prefix_matches_nothing_useful() {
        // Guarded at manifest validation (non-empty required); the
        // matcher itself just strips.
        let lines = split_lines(b"FAIL: Foo\n");
        assert_eq!(fail_names(&lines, "FAIL: "), vec!["Foo"]);
    }
}
