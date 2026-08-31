use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{error::RomTestError, verify::VerifySpec};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomManifest {
    pub rom_root: PathBuf,
    pub suites: Vec<RomSuite>,
    #[serde(default)]
    pub completion_profiles: BTreeMap<String, CompletionSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomSuite {
    pub name: String,
    #[serde(default)]
    pub cases: Vec<RomCase>,
    #[serde(default)]
    pub case_patterns: Vec<RomCasePattern>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct RomCase {
    pub id: String,
    pub rom: String,
    pub cycles: usize,
    #[serde(default)]
    pub completion: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub verify: VerifySpec,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomCasePattern {
    pub glob: String,
    #[serde(default)]
    pub exclude_globs: Vec<String>,
    #[serde(default)]
    pub id_prefix: String,
    pub cycles: usize,
    #[serde(default)]
    pub completion: Option<String>,
    #[serde(default)]
    pub verify: VerifySpec,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletionSpec {
    #[serde(default = "default_poll_interval")]
    pub poll_interval: usize,
    pub stages: Vec<CompletionStage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletionStage {
    #[serde(default)]
    pub memory: Vec<MemoryCompletion>,
    #[serde(default)]
    pub registers: crate::verify::RegisterVerify,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryCompletion {
    pub address: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub not_value: Option<String>,
    #[serde(default = "default_width")]
    pub width: u8,
}

pub struct SelectedCase<'a> {
    pub suite: &'a RomSuite,
    pub case: &'a RomCase,
    pub completion: Option<&'a CompletionSpec>,
}

impl RomManifest {
    pub fn load(path: &Path) -> Result<Self, RomTestError> {
        let content = std::fs::read_to_string(path)?;
        let mut manifest: Self = serde_saphyr::from_str(&content)?;
        manifest.expand_case_patterns(path)?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn expand_case_patterns(&mut self, manifest_path: &Path) -> Result<(), RomTestError> {
        let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        let rom_root = manifest_dir.join(&self.rom_root);
        for suite in &mut self.suites {
            let suite_dir = rom_root.join(&suite.name);
            let patterns = suite
                .case_patterns
                .iter()
                .map(|pattern| crate::case_expansion::CasePatternRef {
                    glob: &pattern.glob,
                    exclude_globs: &pattern.exclude_globs,
                    id_prefix: &pattern.id_prefix,
                })
                .collect::<Vec<_>>();
            let expanded = crate::case_expansion::expand_case_patterns(&suite_dir, &patterns)
                .map_err(RomTestError::InvalidManifest)?;
            for case in expanded {
                let pattern = &suite.case_patterns[case.pattern_index];
                suite.cases.push(RomCase {
                    id: case.id,
                    rom: case.rom,
                    cycles: pattern.cycles,
                    completion: pattern.completion.clone(),
                    description: case.description,
                    verify: pattern.verify.clone(),
                });
            }
            suite.cases.sort_by(|left, right| left.id.cmp(&right.id));
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), RomTestError> {
        self.validate_structure()?;
        self.validate_completion_profiles()?;
        self.validate_suites()
    }

    fn validate_structure(&self) -> Result<(), RomTestError> {
        if self.rom_root.as_os_str().is_empty() || self.suites.is_empty() {
            return Err(RomTestError::InvalidManifest(
                "rom_root and at least one suite are required".to_string(),
            ));
        }
        Ok(())
    }

    fn validate_completion_profiles(&self) -> Result<(), RomTestError> {
        for (name, profile) in &self.completion_profiles {
            validate_completion_profile(name, profile)?;
        }
        Ok(())
    }

    fn validate_suites(&self) -> Result<(), RomTestError> {
        let mut ids = BTreeSet::new();
        for suite in &self.suites {
            self.validate_suite(suite, &mut ids)?;
        }
        Ok(())
    }

    fn validate_suite(
        &self,
        suite: &RomSuite,
        ids: &mut BTreeSet<String>,
    ) -> Result<(), RomTestError> {
        if suite.name.is_empty() {
            return Err(RomTestError::InvalidManifest(
                "suite name is empty".to_string(),
            ));
        }
        for case in &suite.cases {
            self.validate_case(case, ids)?;
        }
        Ok(())
    }

    fn validate_case(
        &self,
        case: &RomCase,
        ids: &mut BTreeSet<String>,
    ) -> Result<(), RomTestError> {
        if !ids.insert(case.id.clone()) {
            return Err(RomTestError::InvalidManifest(format!(
                "duplicate case id `{}`",
                case.id
            )));
        }
        if case.rom.is_empty() || case.cycles == 0 || case.verify.is_empty() {
            return Err(RomTestError::InvalidManifest(format!(
                "case `{}` needs rom, positive cycles, and verification",
                case.id
            )));
        }
        case.verify.validate()?;
        if case
            .completion
            .as_ref()
            .is_some_and(|name| !self.completion_profiles.contains_key(name))
        {
            return Err(RomTestError::InvalidManifest(format!(
                "case `{}` references unknown completion profile",
                case.id
            )));
        }
        Ok(())
    }

    pub fn select(&self, ids: &[String]) -> Vec<SelectedCase<'_>> {
        self.suites
            .iter()
            .flat_map(|suite| {
                suite
                    .cases
                    .iter()
                    .filter(|case| ids.is_empty() || ids.iter().any(|id| id == &case.id))
                    .map(|case| SelectedCase {
                        suite,
                        case,
                        completion: case
                            .completion
                            .as_ref()
                            .and_then(|name| self.completion_profiles.get(name)),
                    })
            })
            .collect()
    }
}

fn validate_completion_profile(name: &str, profile: &CompletionSpec) -> Result<(), RomTestError> {
    if profile.poll_interval == 0 || profile.stages.is_empty() {
        return Err(RomTestError::InvalidManifest(format!(
            "completion profile `{name}` needs a positive interval and a stage"
        )));
    }
    for stage in &profile.stages {
        validate_completion_stage(name, stage)?;
    }
    Ok(())
}

fn validate_completion_stage(name: &str, stage: &CompletionStage) -> Result<(), RomTestError> {
    if stage.memory.is_empty() && stage.registers.is_empty() {
        return Err(RomTestError::InvalidManifest(format!(
            "completion profile `{name}` contains an empty stage"
        )));
    }
    for condition in &stage.memory {
        condition.validate()?;
    }
    stage.registers.validate()
}

impl MemoryCompletion {
    fn validate(&self) -> Result<(), RomTestError> {
        if self.value.is_some() == self.not_value.is_some() || !matches!(self.width, 1 | 2 | 4) {
            return Err(RomTestError::InvalidManifest(
                "completion memory needs exactly one value and width 1/2/4".to_string(),
            ));
        }
        if crate::verify::parse_hex(&self.address)? > u64::from(u32::MAX) {
            return Err(RomTestError::InvalidManifest(
                "completion address out of range".to_string(),
            ));
        }
        let value = self
            .value
            .as_ref()
            .or(self.not_value.as_ref())
            .expect("presence validated");
        let max = match self.width {
            1 => u64::from(u8::MAX),
            2 => u64::from(u16::MAX),
            _ => u64::from(u32::MAX),
        };
        if crate::verify::parse_hex(value)? > max {
            return Err(RomTestError::InvalidManifest(
                "completion value out of range".to_string(),
            ));
        }
        Ok(())
    }
}

const fn default_poll_interval() -> usize {
    256
}
const fn default_width() -> u8 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_case_without_verification() {
        let manifest: RomManifest = serde_saphyr::from_str(
            "rom_root: roms\nsuites: [{ name: test, cases: [{ id: x, rom: x.gba, cycles: 1 }] }]",
        )
        .unwrap();
        assert!(manifest.validate().is_err());
    }

    #[test]
    fn expands_globs_and_resolves_completion() {
        let root = std::env::temp_dir().join(format!("gba-manifest-{}", std::process::id()));
        let suite = root.join("roms/suite");
        std::fs::create_dir_all(&suite).unwrap();
        std::fs::write(suite.join("case.gba"), []).unwrap();
        let manifest_path = root.join("tests.yaml");
        std::fs::write(
            &manifest_path,
            "rom_root: roms\ncompletion_profiles:\n  done:\n    poll_interval: 1\n    stages:\n      - registers: { r0: 1 }\nsuites:\n  - name: suite\n    case_patterns:\n      - glob: '*.gba'\n        id_prefix: test_\n        cycles: 4\n        completion: done\n        verify:\n          registers: { r0: 1 }\n",
        )
        .unwrap();
        let manifest = RomManifest::load(&manifest_path).unwrap();
        let selected = manifest.select(&["test_case".to_string()]);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].case.rom, "case.gba");
        assert!(selected[0].completion.is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_duplicate_ids_and_invalid_completion() {
        let duplicate: RomManifest = serde_saphyr::from_str(
            "rom_root: roms\nsuites:\n  - name: x\n    cases:\n      - { id: same, rom: a.gba, cycles: 1, verify: { registers: { r0: 1 } } }\n      - { id: same, rom: b.gba, cycles: 1, verify: { registers: { r0: 1 } } }\n",
        )
        .unwrap();
        assert!(duplicate.validate().is_err());

        let empty_profile: RomManifest = serde_saphyr::from_str(
            "rom_root: roms\ncompletion_profiles: { bad: { poll_interval: 0, stages: [] } }\nsuites: [{ name: x }]\n",
        )
        .unwrap();
        assert!(empty_profile.validate().is_err());
    }
}
