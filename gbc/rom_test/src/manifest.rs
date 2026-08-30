use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use super::{
    error::RomTestError,
    verify::{RegisterVerify, VerifySpec, parse_hex},
};

/// Top-level ROM test manifest.
#[derive(Debug, Clone, Deserialize)]
pub struct RomManifest {
    pub rom_root: PathBuf,
    pub suites: Vec<RomSuite>,
    #[serde(default)]
    pub completion_profiles: BTreeMap<String, CompletionSpec>,
    /// Cell ids (`case@model`) whose failure is a known emulator gap and
    /// therefore not counted as a suite failure. A cell in this list that
    /// PASSES is an unexpected pass (the manifest entry should be removed).
    #[serde(default)]
    pub expected_failures: Vec<String>,
    #[serde(default)]
    expected_failure_files: Vec<PathBuf>,
}

impl RomManifest {
    pub fn load(path: &Path) -> Result<Self, RomTestError> {
        let yaml = fs::read_to_string(path)?;
        let mut manifest: RomManifest = serde_saphyr::from_str(&yaml)?;
        manifest.expand_case_patterns(path)?;
        manifest.load_expected_failure_files(path)?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn load_expected_failure_files(&mut self, manifest_path: &Path) -> Result<(), RomTestError> {
        let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        for relative_path in &self.expected_failure_files {
            let source = fs::read_to_string(manifest_dir.join(relative_path))?;
            self.expected_failures.extend(
                source
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty() && !line.starts_with('#'))
                    .map(str::to_string),
            );
        }
        Ok(())
    }

    fn expand_case_patterns(&mut self, manifest_path: &Path) -> Result<(), RomTestError> {
        let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        let rom_root = manifest_dir.join(&self.rom_root);
        for suite in &mut self.suites {
            let suite_dir = rom_root.join(&suite.name);
            let mut matched_paths = BTreeSet::new();
            for pattern in &suite.case_patterns {
                let absolute_pattern = suite_dir.join(&pattern.glob);
                let absolute_pattern = absolute_pattern.to_string_lossy();
                let paths = glob::glob(&absolute_pattern).map_err(|error| {
                    RomTestError::InvalidManifest(format!(
                        "invalid case glob `{}`: {error}",
                        pattern.glob
                    ))
                })?;
                for path in paths {
                    let path = path.map_err(|error| {
                        RomTestError::InvalidManifest(format!(
                            "failed to expand case glob `{}`: {error}",
                            pattern.glob
                        ))
                    })?;
                    if !matched_paths.insert(path.clone()) {
                        continue;
                    }
                    let relative = path.strip_prefix(&suite_dir).map_err(|_| {
                        RomTestError::InvalidManifest(format!(
                            "case glob `{}` matched outside suite `{}`",
                            pattern.glob, suite.name
                        ))
                    })?;
                    if pattern.exclude_globs.iter().any(|exclude| {
                        glob::Pattern::new(exclude)
                            .is_ok_and(|pattern| pattern.matches_path(relative))
                    }) {
                        continue;
                    }
                    let stem = relative
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .ok_or_else(|| {
                            RomTestError::InvalidManifest(format!(
                                "case glob `{}` matched a path without a UTF-8 file stem",
                                pattern.glob
                            ))
                        })?;
                    suite.cases.push(RomCase {
                        id: format!("{}{}", pattern.id_prefix, stem),
                        rom: relative.to_string_lossy().into_owned(),
                        models: pattern.models.clone(),
                        cycles: pattern.cycles,
                        completion: pattern.completion.clone(),
                        description: stem.replace(['_', '-'], " "),
                        verify: pattern.verify.clone(),
                        reference: None,
                        tags: pattern.tags.clone(),
                        inputs: Vec::new(),
                    });
                }
            }
            suite.cases.sort_by(|left, right| left.id.cmp(&right.id));
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), RomTestError> {
        if self.rom_root.as_os_str().is_empty() {
            return Err(RomTestError::InvalidManifest(
                "manifest must define a rom_root".to_string(),
            ));
        }
        if self.suites.is_empty() {
            return Err(RomTestError::InvalidManifest(
                "manifest must define at least one suite".to_string(),
            ));
        }

        self.validate_expected_failures()?;
        self.validate_completion_profiles()?;
        self.validate_suites()
    }

    fn validate_expected_failures(&self) -> Result<(), RomTestError> {
        let cell_ids: BTreeSet<String> = self
            .suites
            .iter()
            .flat_map(|suite| &suite.cases)
            .flat_map(|case| {
                case.models
                    .iter()
                    .map(|model| format!("{}@{}", case.id, model.name()))
            })
            .collect();
        for expected in &self.expected_failures {
            if !cell_ids.contains(expected) {
                return Err(RomTestError::InvalidManifest(format!(
                    "expected_failures entry `{}` does not match any cell",
                    expected
                )));
            }
        }
        Ok(())
    }

    fn validate_completion_profiles(&self) -> Result<(), RomTestError> {
        for (name, completion) in &self.completion_profiles {
            completion.validate(name)?;
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
                "suite name must not be empty".to_string(),
            ));
        }
        for case in &suite.cases {
            if !ids.insert(case.id.clone()) {
                return Err(RomTestError::InvalidManifest(format!(
                    "duplicate case id `{}`",
                    case.id
                )));
            }
            case.validate()?;
            self.validate_case_completion(case)?;
        }
        Ok(())
    }

    fn validate_case_completion(&self, case: &RomCase) -> Result<(), RomTestError> {
        let Some(name) = &case.completion else {
            return Ok(());
        };
        let completion = self.completion_profiles.get(name).ok_or_else(|| {
            RomTestError::InvalidManifest(format!(
                "case `{}` references unknown completion profile `{name}`",
                case.id
            ))
        })?;
        if completion.stages.iter().any(|stage| stage.serial_hash) && !case.verify.has_serial_hash()
        {
            return Err(RomTestError::InvalidManifest(format!(
                "case `{}` uses serial hash completion without a serial hash verification",
                case.id
            )));
        }
        Ok(())
    }

    pub fn is_expected_failure(&self, cell_id: &str) -> bool {
        self.expected_failures
            .iter()
            .any(|expected| expected == cell_id)
    }

    /// Expand cases into (case, model) matrix cells, applying optional
    /// filters. Filtering by a bare case id selects all of its models;
    /// filtering by `case@model` selects a single cell.
    pub fn select(
        &self,
        ids: &[String],
        models: &[GbcModel],
        tags: &[String],
    ) -> Vec<MatrixCell<'_>> {
        let mut cells = Vec::new();
        for suite in &self.suites {
            for case in &suite.cases {
                for &model in &case.models {
                    if self.matches_filters(case, model, ids, models, tags) {
                        cells.push(MatrixCell {
                            suite,
                            case,
                            model,
                            completion: case
                                .completion
                                .as_ref()
                                .and_then(|name| self.completion_profiles.get(name)),
                        });
                    }
                }
            }
        }
        cells
    }

    fn matches_filters(
        &self,
        case: &RomCase,
        model: GbcModel,
        ids: &[String],
        models: &[GbcModel],
        tags: &[String],
    ) -> bool {
        if !tags.is_empty() && !tags.iter().any(|tag| case.tags.contains(tag)) {
            return false;
        }
        if !models.is_empty() && !models.contains(&model) {
            return false;
        }
        if !ids.is_empty() {
            let cell_id = format!("{}@{}", case.id, model.name());
            return ids.iter().any(|id| *id == case.id || *id == cell_id);
        }
        true
    }
}

/// A group of cases sharing a ROM repository directory under `rom_root`.
#[derive(Debug, Clone, Deserialize)]
pub struct RomSuite {
    pub name: String,
    #[serde(default)]
    pub cases: Vec<RomCase>,
    #[serde(default)]
    pub case_patterns: Vec<RomCasePattern>,
}

/// A group of ROM files sharing the same execution and verification settings.
#[derive(Debug, Clone, Deserialize)]
pub struct RomCasePattern {
    pub glob: String,
    #[serde(default)]
    pub exclude_globs: Vec<String>,
    #[serde(default)]
    pub id_prefix: String,
    pub models: Vec<GbcModel>,
    pub cycles: usize,
    #[serde(default)]
    pub completion: Option<String>,
    #[serde(default)]
    pub verify: VerifySpec,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// GBC hardware model targeted by a matrix cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum GbcModel {
    Dmg0,
    Dmg,
    CgbC,
    CgbD,
    Agb,
}

impl GbcModel {
    pub fn name(self) -> &'static str {
        match self {
            GbcModel::Dmg0 => "dmg0",
            GbcModel::Dmg => "dmg",
            GbcModel::CgbC => "cgb_c",
            GbcModel::CgbD => "cgb_d",
            GbcModel::Agb => "agb",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompletionSpec {
    #[serde(default = "default_poll_interval")]
    pub poll_interval: usize,
    pub stages: Vec<CompletionStage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompletionStage {
    #[serde(default)]
    pub memory: Vec<MemoryCompletion>,
    #[serde(default)]
    pub serial_hash: bool,
    #[serde(default)]
    pub registers: RegisterVerify,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MemoryCompletion {
    pub address: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub not_value: Option<String>,
}

impl CompletionSpec {
    fn validate(&self, name: &str) -> Result<(), RomTestError> {
        if self.poll_interval == 0 || self.stages.is_empty() {
            return Err(RomTestError::InvalidManifest(format!(
                "completion profile `{name}` must have a positive poll interval and at least one stage"
            )));
        }
        for stage in &self.stages {
            if stage.memory.is_empty() && !stage.serial_hash && stage.registers.is_empty() {
                return Err(RomTestError::InvalidManifest(format!(
                    "completion profile `{name}` has an empty stage"
                )));
            }
            for condition in &stage.memory {
                let address = parse_hex(&condition.address)?;
                if address > u16::MAX as u64
                    || condition.value.is_some() == condition.not_value.is_some()
                {
                    return Err(RomTestError::InvalidManifest(format!(
                        "completion profile `{name}` has an invalid memory condition"
                    )));
                }
                let value = condition
                    .value
                    .as_ref()
                    .or(condition.not_value.as_ref())
                    .unwrap();
                if parse_hex(value)? > u8::MAX as u64 {
                    return Err(RomTestError::InvalidManifest(format!(
                        "completion profile `{name}` has an out-of-range memory value"
                    )));
                }
            }
            stage.registers.validate()?;
        }
        Ok(())
    }
}

const fn default_poll_interval() -> usize {
    256
}

/// Reference image specification: a single path applied to every model, or
/// a per-model map. Paths are relative to the suite directory.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Reference {
    Single(String),
    PerModel(BTreeMap<GbcModel, String>),
}

impl Reference {
    pub fn path_for(&self, model: GbcModel) -> Option<&str> {
        match self {
            Reference::Single(path) => Some(path),
            Reference::PerModel(map) => map.get(&model).map(String::as_str),
        }
    }
}

/// A single ROM test case.
#[derive(Debug, Clone, Deserialize)]
pub struct RomCase {
    pub id: String,
    /// Path relative to the suite directory.
    pub rom: String,
    /// Hardware models to run this ROM on (matrix columns).
    pub models: Vec<GbcModel>,
    /// Number of M-cycles to run before verifying.
    pub cycles: usize,
    #[serde(default)]
    pub completion: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub verify: VerifySpec,
    /// Optional reference image, resolved per model.
    #[serde(default)]
    pub reference: Option<Reference>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub inputs: Vec<InputEvent>,
}

impl RomCase {
    pub fn validate(&self) -> Result<(), RomTestError> {
        if self.rom.is_empty() {
            return Err(RomTestError::InvalidManifest(format!(
                "case `{}` has empty rom path",
                self.id
            )));
        }
        if self.models.is_empty() {
            return Err(RomTestError::InvalidManifest(format!(
                "case `{}` must declare at least one model",
                self.id
            )));
        }
        if self.cycles == 0 {
            return Err(RomTestError::InvalidManifest(format!(
                "case `{}` must declare a positive cycle count",
                self.id
            )));
        }
        if self
            .inputs
            .windows(2)
            .any(|events| events[0].cycle >= events[1].cycle)
            || self.inputs.iter().any(|event| event.cycle >= self.cycles)
        {
            return Err(RomTestError::InvalidManifest(format!(
                "case `{}` input events must be ordered within its cycle limit",
                self.id
            )));
        }
        self.verify
            .validate()
            .map_err(|e| RomTestError::InvalidManifest(format!("case `{}`: {}", self.id, e)))?;
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct InputEvent {
    pub cycle: usize,
    #[serde(default)]
    pub buttons: Vec<GbcButton>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GbcButton {
    A,
    B,
    Select,
    Start,
    Right,
    Left,
    Up,
    Down,
}

impl GbcButton {
    pub fn mask(self) -> u8 {
        1 << self as u8
    }
}

/// One cell of the case × model matrix.
#[derive(Debug, Clone)]
pub struct MatrixCell<'a> {
    pub suite: &'a RomSuite,
    pub case: &'a RomCase,
    pub model: GbcModel,
    completion: Option<&'a CompletionSpec>,
}

impl MatrixCell<'_> {
    pub fn id(&self) -> String {
        format!("{}@{}", self.case.id, self.model.name())
    }

    pub fn suite_dir(&self, rom_root: &Path) -> PathBuf {
        rom_root.join(&self.suite.name)
    }

    pub fn rom_path(&self, rom_root: &Path) -> PathBuf {
        self.suite_dir(rom_root).join(&self.case.rom)
    }

    /// Reference image for this cell, if the case declares one for its model.
    pub fn reference_path(&self, rom_root: &Path) -> Option<PathBuf> {
        self.case
            .reference
            .as_ref()
            .and_then(|reference| reference.path_for(self.model))
            .map(|path| self.suite_dir(rom_root).join(path))
    }

    pub fn verify(&self) -> &VerifySpec {
        &self.case.verify
    }

    pub fn completion(&self) -> Option<&CompletionSpec> {
        self.completion
    }

    pub fn cycles(&self) -> usize {
        self.case.cycles
    }

    pub fn description(&self) -> &str {
        &self.case.description
    }

    pub fn tags(&self) -> &[String] {
        &self.case.tags
    }
}
