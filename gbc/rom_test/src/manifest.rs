use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use super::{error::RomTestError, verify::VerifySpec};

/// Top-level ROM test manifest.
#[derive(Debug, Clone, Deserialize)]
pub struct RomManifest {
    pub rom_root: PathBuf,
    pub suites: Vec<RomSuite>,
    /// Cell ids (`case@model`) whose failure is a known emulator gap and
    /// therefore not counted as a suite failure. A cell in this list that
    /// PASSES is an unexpected pass (the manifest entry should be removed).
    #[serde(default)]
    pub expected_failures: Vec<String>,
}

impl RomManifest {
    pub fn load(path: &Path) -> Result<Self, RomTestError> {
        let yaml = fs::read_to_string(path)?;
        let manifest: RomManifest = serde_saphyr::from_str(&yaml)?;
        manifest.validate()?;
        Ok(manifest)
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
        let mut ids = BTreeSet::new();
        for suite in &self.suites {
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
            }
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
                        cells.push(MatrixCell { suite, case, model });
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
    pub cases: Vec<RomCase>,
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
    pub description: String,
    #[serde(default)]
    pub verify: VerifySpec,
    /// Optional reference image, resolved per model.
    #[serde(default)]
    pub reference: Option<Reference>,
    #[serde(default)]
    pub tags: Vec<String>,
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
        self.verify
            .validate()
            .map_err(|e| RomTestError::InvalidManifest(format!("case `{}`: {}", self.id, e)))?;
        Ok(())
    }
}

/// One cell of the case × model matrix.
#[derive(Debug, Clone)]
pub struct MatrixCell<'a> {
    pub suite: &'a RomSuite,
    pub case: &'a RomCase,
    pub model: GbcModel,
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
