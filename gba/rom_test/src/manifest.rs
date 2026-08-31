use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct RomManifest {
    pub rom_root: PathBuf,
    pub suites: Vec<RomSuite>,
}

#[derive(Debug, Deserialize)]
pub struct RomSuite {
    pub name: String,
    #[serde(default)]
    pub cases: Vec<RomCase>,
    #[serde(default)]
    pub case_patterns: Vec<CasePattern>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RomCase {
    pub id: String,
    pub rom: Option<String>,
    pub cycles: Option<usize>,
    pub verify: Option<crate::verify::VerifySpec>,
}

#[derive(Debug, Deserialize)]
pub struct CasePattern {
    pub glob: String,
    #[serde(default)]
    pub exclude_globs: Vec<String>,
    #[serde(default)]
    pub id_prefix: String,
}

impl RomManifest {
    pub fn load(path: &Path) -> Result<Self, crate::error::RomTestError> {
        let content = std::fs::read_to_string(path)?;
        let manifest: Self = serde_saphyr::from_str(&content)?;
        Ok(manifest)
    }
}
