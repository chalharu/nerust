use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use nerust_nes_core::core_options::{CoreOptions, Mmc3IrqVariant};
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
    #[serde(default)]
    pub sub_mapper_type: Option<u8>,
    #[serde(default)]
    pub mmc3_irq_variant: Option<Mmc3IrqVariant>,
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
        if let Some(sub_mapper_type) = self.sub_mapper_type
            && sub_mapper_type > 0x0F
        {
            return Err(RomTestError::InvalidManifest(format!(
                "ROM case `{}` uses unsupported sub_mapper_type {}; NES 2.0 submappers must fit in 4 bits",
                self.id, sub_mapper_type
            )));
        }
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

    pub fn core_options(&self) -> CoreOptions {
        CoreOptions {
            mmc3_irq_variant: self.mmc3_irq_variant,
        }
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
    let rom_bytes = fs::read(&rom_path).map_err(|source| RomTestError::ReadFile {
        path: rom_path,
        source,
    })?;

    apply_case_rom_overrides(case, rom_bytes)
}

pub(crate) fn apply_case_rom_overrides(
    case: &RomCase,
    mut rom_bytes: Vec<u8>,
) -> Result<Vec<u8>, RomTestError> {
    if let Some(sub_mapper_type) = case.sub_mapper_type {
        if rom_bytes.len() < 16 || &rom_bytes[..4] != b"NES\x1A" {
            return Err(RomTestError::InvalidManifest(format!(
                "ROM case `{}` cannot override sub_mapper_type without a 16-byte iNES/NES 2.0 header",
                case.id
            )));
        }

        let was_nes20 = (rom_bytes[7] & 0x0C) == 0x08;
        rom_bytes[7] = (rom_bytes[7] & 0xF3) | 0x08;
        rom_bytes[8] = if was_nes20 { rom_bytes[8] & 0x0F } else { 0 } | (sub_mapper_type << 4);
    }

    Ok(rom_bytes)
}

fn default_rom_root() -> PathBuf {
    PathBuf::from("../roms")
}
