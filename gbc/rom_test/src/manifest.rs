use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use super::{error::RomTestError, events::RomEvent};

/// Top-level ROM test manifest matching the user's YAML schema.
#[derive(Debug, Clone, Deserialize)]
pub struct RomManifest {
    pub rom_root: PathBuf,
    pub cases: Vec<RomCase>,
}

impl RomManifest {
    pub fn load(path: &Path) -> Result<Self, RomTestError> {
        let yaml = fs::read_to_string(path)?;
        let manifest: RomManifest = serde_saphyr::from_str(&yaml)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), RomTestError> {
        if self.cases.is_empty() {
            return Err(RomTestError::InvalidManifest(
                "manifest must define at least one ROM case".to_string(),
            ));
        }
        let mut ids = BTreeSet::new();
        for case in &self.cases {
            if !ids.insert(case.id.clone()) {
                return Err(RomTestError::InvalidManifest(format!(
                    "duplicate ROM case id `{}`",
                    case.id
                )));
            }
            case.validate()?;
        }
        Ok(())
    }

    pub fn case(&self, id: &str) -> Option<&RomCase> {
        self.cases.iter().find(|case| case.id == id)
    }

    pub fn select(&self, ids: &[String], perf_only: bool) -> Vec<&RomCase> {
        let mut selected = self
            .cases
            .iter()
            .filter(|case| (!perf_only || case.perf) && (ids.is_empty() || ids.contains(&case.id)))
            .collect::<Vec<_>>();
        selected.sort_by(|left, right| {
            left.category
                .cmp(&right.category)
                .then_with(|| left.id.cmp(&right.id))
        });
        selected
    }
}

/// GBC hardware model targeted by the test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GbcModel {
    Dmg,
    Cgb,
    Agb,
}

impl Default for GbcModel {
    fn default() -> Self {
        Self::Cgb
    }
}

/// A single ROM test case.
#[derive(Debug, Clone, Deserialize)]
pub struct RomCase {
    pub id: String,
    pub category: String,
    pub description: String,
    pub rom: String,
    #[serde(default)]
    pub model: GbcModel,
    #[serde(default)]
    pub perf: bool,
    pub events: Vec<RomEvent>,
}

impl RomCase {
    pub fn validate(&self) -> Result<(), RomTestError> {
        if self.rom.is_empty() {
            return Err(RomTestError::InvalidManifest(format!(
                "case `{}` has empty rom path",
                self.id
            )));
        }
        if self.events.is_empty() {
            return Err(RomTestError::InvalidManifest(format!(
                "case `{}` requires at least one event",
                self.id
            )));
        }
        Ok(())
    }

    pub fn rom_path(&self, rom_root: &Path) -> PathBuf {
        rom_root.join(&self.rom)
    }
}
