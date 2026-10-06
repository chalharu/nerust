use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{error::RomTestError, events::RomEvent};

pub const DEFAULT_AUDIO_SAMPLE_RATE: u32 = 48_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RomManifest {
    #[serde(default = "default_rom_root")]
    pub rom_root: PathBuf,
    pub cases: Vec<RomCase>,
}

impl RomManifest {
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

    pub fn select<'a>(
        &'a self,
        ids: &[String],
        perf_only: bool,
    ) -> Result<Vec<&'a RomCase>, RomTestError> {
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

        if selected.is_empty() {
            let scope = if perf_only { "perf-enabled " } else { "" };
            let description = if ids.is_empty() {
                "all cases".to_string()
            } else {
                ids.join(", ")
            };
            return Err(RomTestError::InvalidManifest(format!(
                "no {scope}ROM cases matched {description}"
            )));
        }

        Ok(selected)
    }

    pub(crate) fn resolve_paths(&mut self, manifest_path: &Path) {
        let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
        let resolved_rom_root = if self.rom_root.is_absolute() {
            self.rom_root.clone()
        } else {
            manifest_dir.join(&self.rom_root)
        };

        for case in &mut self.cases {
            case.resolve_rom_path(&resolved_rom_root);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RomCase {
    pub id: String,
    pub category: RomCategory,
    pub description: String,
    pub rom: String,
    #[serde(default)]
    pub perf: bool,
    /// CI scope flag, honored by build.rs test generation.
    /// Absent means in scope.
    #[serde(default = "default_ci")]
    pub ci: bool,
    /// Load-affecting options as raw strings, passed through to the
    /// factory CLI options schema as argv (e.g. `"--mmc3-irq-variant",
    /// "nec"`, `"--submapper", "1"`); clap validates them there.
    /// Anything unknown is rejected loudly — never silently ignored.
    #[serde(default)]
    pub options: Vec<String>,
    pub events: Vec<RomEvent>,
    #[serde(default)]
    pub expected_audio: Option<AudioExpectation>,
    #[serde(skip, default)]
    pub(crate) resolved_rom_path: PathBuf,
}

impl RomCase {
    pub fn validate(&self) -> Result<(), RomTestError> {
        if self.id.trim().is_empty() {
            return Err(RomTestError::InvalidManifest(
                "ROM case id must not be empty".to_string(),
            ));
        }
        if self.rom.trim().is_empty() {
            return Err(RomTestError::InvalidManifest(format!(
                "ROM case `{}` must define a ROM path",
                self.id
            )));
        }
        if self.description.trim().is_empty() {
            return Err(RomTestError::InvalidManifest(format!(
                "ROM case `{}` must define a description",
                self.id
            )));
        }
        // Case options pass through to the factory CLI schema
        // untouched; clap validates them at open.
        // (No manifest-side validation: option validity belongs to
        // the factory that defines the flags.)
        let rom_path = self.resolved_rom_path()?;
        if !rom_path.is_file() {
            return Err(RomTestError::InvalidManifest(format!(
                "ROM case `{}` references missing ROM `{}`",
                self.id,
                rom_path.display()
            )));
        }
        if self.events.is_empty() {
            return Err(RomTestError::InvalidManifest(format!(
                "ROM case `{}` must define at least one event",
                self.id
            )));
        }

        let mut last_frame = 0_u64;
        for (index, event) in self.events.iter().enumerate() {
            event.validate(&self.id)?;
            if index > 0 && event.frame < last_frame {
                return Err(RomTestError::InvalidManifest(format!(
                    "ROM case `{}` has out-of-order event at frame {}",
                    self.id, event.frame
                )));
            }
            last_frame = event.frame;
        }

        if let Some(expected_audio) = &self.expected_audio {
            expected_audio.validate(&self.id)?;
        }

        Ok(())
    }

    pub fn final_frame(&self) -> u64 {
        self.events.last().map(|event| event.frame).unwrap_or(0)
    }

    pub fn audio_sample_rate(&self) -> u32 {
        self.expected_audio
            .as_ref()
            .map_or(DEFAULT_AUDIO_SAMPLE_RATE, |expected| expected.sample_rate)
    }

    fn resolve_rom_path(&mut self, rom_root: &Path) {
        self.resolved_rom_path = rom_root.join(&self.rom);
    }

    pub(crate) fn resolved_rom_path(&self) -> Result<&Path, RomTestError> {
        if self.resolved_rom_path.as_os_str().is_empty() {
            return Err(RomTestError::InvalidManifest(format!(
                "ROM case `{}` does not have a resolved ROM path",
                self.id
            )));
        }

        Ok(&self.resolved_rom_path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RomCategory {
    Cpu,
    Ppu,
    Apu,
    Mapper,
    Input,
}

impl RomCategory {
    pub const fn label(self) -> &'static str {
        match self {
            RomCategory::Cpu => "CPU Tests",
            RomCategory::Ppu => "PPU Tests",
            RomCategory::Apu => "APU Tests",
            RomCategory::Mapper => "Mapper-specific Tests",
            RomCategory::Input => "Input Tests",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioExpectation {
    pub sample_rate: u32,
    pub samples: u64,
    #[serde(with = "super::serde_helpers::hex_u64")]
    pub hash: u64,
}

impl AudioExpectation {
    pub(crate) fn validate(&self, case_id: &str) -> Result<(), RomTestError> {
        if self.sample_rate == 0 {
            return Err(RomTestError::InvalidManifest(format!(
                "ROM case `{case_id}` must not use an audio sample rate of 0"
            )));
        }
        Ok(())
    }
}

fn default_ci() -> bool {
    true
}

pub fn default_manifest_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("rom_tests.yaml")
}

pub fn load_manifest(path: &Path) -> Result<RomManifest, RomTestError> {
    let manifest_source = fs::read_to_string(path).map_err(|source| RomTestError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    let manifest = serde_saphyr::from_str::<RomManifest>(&manifest_source).map_err(|source| {
        RomTestError::ParseManifest {
            path: path.to_path_buf(),
            source: Box::new(source),
        }
    })?;
    let mut manifest = manifest;
    manifest.resolve_paths(path);
    manifest.validate()?;
    Ok(manifest)
}

pub fn load_default_manifest() -> Result<RomManifest, RomTestError> {
    load_manifest(&default_manifest_path())
}

pub fn read_rom(case: &RomCase) -> Result<Vec<u8>, RomTestError> {
    let rom_path = case.resolved_rom_path()?.to_path_buf();
    fs::read(&rom_path).map_err(|source| RomTestError::ReadFile {
        path: rom_path,
        source,
    })
}

fn default_rom_root() -> PathBuf {
    PathBuf::from("../../../roms")
}
