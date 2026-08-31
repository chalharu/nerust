//! Filesystem glob expansion shared by the build script and runtime manifest loader.
//!
//! Keeping this logic in one dependency-free module prevents build-time and runtime
//! case discovery from drifting while still allowing one generated test per ROM.

use std::{collections::BTreeSet, path::Path};

pub struct CasePatternRef<'a> {
    pub glob: &'a str,
    pub exclude_globs: &'a [String],
    pub id_prefix: &'a str,
}

pub struct ExpandedCase {
    pub pattern_index: usize,
    pub id: String,
    pub rom: String,
    pub description: String,
}

pub fn expand_case_patterns(
    suite_dir: &Path,
    patterns: &[CasePatternRef<'_>],
) -> Result<Vec<ExpandedCase>, String> {
    let mut matched = BTreeSet::new();
    let mut cases = Vec::new();
    for (index, pattern) in patterns.iter().enumerate() {
        cases.extend(expand_pattern(suite_dir, pattern, index, &mut matched)?);
    }
    Ok(cases)
}

fn expand_pattern(
    suite_dir: &Path,
    pattern: &CasePatternRef<'_>,
    pattern_index: usize,
    matched: &mut BTreeSet<std::path::PathBuf>,
) -> Result<Vec<ExpandedCase>, String> {
    let exclusions = compile_exclusions(pattern.exclude_globs)?;
    let absolute = suite_dir.join(pattern.glob);
    let paths = glob::glob(&absolute.to_string_lossy())
        .map_err(|error| format!("invalid case glob `{}`: {error}", pattern.glob))?;
    let mut cases = Vec::new();
    for path in paths {
        let path = path
            .map_err(|error| format!("failed to expand case glob `{}`: {error}", pattern.glob))?;
        let relative = path
            .strip_prefix(suite_dir)
            .map_err(|_| format!("glob `{}` escaped its suite", pattern.glob))?;
        if exclusions.iter().any(|item| item.matches_path(relative))
            || !matched.insert(path.clone())
        {
            continue;
        }
        cases.push(expanded_case(relative, pattern, pattern_index)?);
    }
    Ok(cases)
}

fn compile_exclusions(excludes: &[String]) -> Result<Vec<glob::Pattern>, String> {
    excludes
        .iter()
        .map(|exclude| {
            glob::Pattern::new(exclude)
                .map_err(|error| format!("invalid exclude glob `{exclude}`: {error}"))
        })
        .collect()
}

fn expanded_case(
    relative: &Path,
    pattern: &CasePatternRef<'_>,
    pattern_index: usize,
) -> Result<ExpandedCase, String> {
    let stem = relative
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "ROM filename must be UTF-8".to_string())?;
    Ok(ExpandedCase {
        pattern_index,
        id: format!("{}{}", pattern.id_prefix, stem),
        rom: relative.to_string_lossy().into_owned(),
        description: stem.replace(['_', '-'], " "),
    })
}
