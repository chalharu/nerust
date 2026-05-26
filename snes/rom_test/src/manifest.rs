// Copyright (c) 2026 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use serde_derive::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("failed to read ROM manifest `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse ROM manifest `{path}`: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("invalid ROM manifest: {message}")]
    Invalid { message: String },
}

#[derive(Debug, Deserialize)]
pub struct RomManifest {
    #[serde(default)]
    rom_root: PathBuf,
    pub cases: Vec<RomCase>,
}

#[derive(Debug, Deserialize)]
pub struct RomCase {
    pub id: String,
    pub description: String,
    pub rom: PathBuf,
    pub max_steps: u64,
    #[serde(default = "default_check_interval_steps")]
    pub check_interval_steps: u64,
    pub assertions: Vec<Assertion>,
    #[serde(skip)]
    resolved_rom_path: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Assertion {
    BusU8 { address: String, expected: String },
    BusU16 { address: String, expected: String },
    WramU8 { address: String, expected: String },
    WramU16 { address: String, expected: String },
    VramU8 { address: String, expected: String },
    VramU16 { address: String, expected: String },
    CgramU8 { address: String, expected: String },
    CgramU16 { address: String, expected: String },
}

pub fn load_default_manifest() -> Result<RomManifest, ManifestError> {
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("rom_tests.yaml");
    load_manifest(&manifest_path)
}

pub fn load_manifest(path: &Path) -> Result<RomManifest, ManifestError> {
    let manifest_source = fs::read_to_string(path).map_err(|source| ManifestError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut manifest = serde_yaml::from_str::<RomManifest>(&manifest_source).map_err(|source| {
        ManifestError::Parse {
            path: path.to_path_buf(),
            source,
        }
    })?;
    manifest.resolve(path)?;
    Ok(manifest)
}

impl RomManifest {
    pub fn case(&self, case_id: &str) -> Option<&RomCase> {
        self.cases.iter().find(|case| case.id == case_id)
    }

    fn resolve(&mut self, manifest_path: &Path) -> Result<(), ManifestError> {
        if self.cases.is_empty() {
            return Err(ManifestError::Invalid {
                message: "ROM manifest must contain at least one case".to_string(),
            });
        }

        let manifest_dir = manifest_path
            .parent()
            .ok_or_else(|| ManifestError::Invalid {
                message: format!(
                    "ROM manifest path `{}` does not have a parent directory",
                    manifest_path.display()
                ),
            })?;
        let rom_root = manifest_dir.join(&self.rom_root);

        let mut seen_case_ids = BTreeSet::new();
        for case in &mut self.cases {
            if !seen_case_ids.insert(case.id.clone()) {
                return Err(ManifestError::Invalid {
                    message: format!("duplicate ROM case id `{}`", case.id),
                });
            }
            case.resolve(&rom_root)?;
        }

        Ok(())
    }
}

impl RomCase {
    pub fn rom_path(&self) -> &Path {
        &self.resolved_rom_path
    }

    fn resolve(&mut self, rom_root: &Path) -> Result<(), ManifestError> {
        if self.max_steps == 0 {
            return Err(ManifestError::Invalid {
                message: format!("ROM case `{}` must have max_steps > 0", self.id),
            });
        }
        if self.check_interval_steps == 0 {
            return Err(ManifestError::Invalid {
                message: format!("ROM case `{}` must have check_interval_steps > 0", self.id),
            });
        }
        if self.assertions.is_empty() {
            return Err(ManifestError::Invalid {
                message: format!("ROM case `{}` must contain at least one assertion", self.id),
            });
        }

        self.resolved_rom_path = rom_root.join(&self.rom);
        if !self.resolved_rom_path.is_file() {
            return Err(ManifestError::Invalid {
                message: format!(
                    "ROM case `{}` points to missing ROM `{}`",
                    self.id,
                    self.resolved_rom_path.display()
                ),
            });
        }

        for assertion in &self.assertions {
            assertion.validate(&self.id)?;
        }

        Ok(())
    }
}

impl Assertion {
    pub fn address(&self) -> Result<u32, ManifestError> {
        match self {
            Self::BusU8 { address, .. }
            | Self::BusU16 { address, .. }
            | Self::WramU8 { address, .. }
            | Self::WramU16 { address, .. }
            | Self::VramU8 { address, .. }
            | Self::VramU16 { address, .. }
            | Self::CgramU8 { address, .. }
            | Self::CgramU16 { address, .. } => parse_value(address, "address").and_then(|value| {
                u32::try_from(value).map_err(|_| ManifestError::Invalid {
                    message: format!("address `{address}` does not fit in u32"),
                })
            }),
        }
    }

    pub fn expected_u8(&self) -> Result<u8, ManifestError> {
        match self {
            Self::BusU8 { expected, .. }
            | Self::WramU8 { expected, .. }
            | Self::VramU8 { expected, .. }
            | Self::CgramU8 { expected, .. } => {
                parse_value(expected, "expected").and_then(|value| {
                    u8::try_from(value).map_err(|_| ManifestError::Invalid {
                        message: format!("expected value `{expected}` does not fit in u8"),
                    })
                })
            }
            Self::BusU16 { .. }
            | Self::WramU16 { .. }
            | Self::VramU16 { .. }
            | Self::CgramU16 { .. } => Err(ManifestError::Invalid {
                message: "expected_u8 called for 16-bit assertion".to_string(),
            }),
        }
    }

    pub fn expected_u16(&self) -> Result<u16, ManifestError> {
        match self {
            Self::BusU16 { expected, .. }
            | Self::WramU16 { expected, .. }
            | Self::VramU16 { expected, .. }
            | Self::CgramU16 { expected, .. } => {
                parse_value(expected, "expected").and_then(|value| {
                    u16::try_from(value).map_err(|_| ManifestError::Invalid {
                        message: format!("expected value `{expected}` does not fit in u16"),
                    })
                })
            }
            Self::BusU8 { .. }
            | Self::WramU8 { .. }
            | Self::VramU8 { .. }
            | Self::CgramU8 { .. } => Err(ManifestError::Invalid {
                message: "expected_u16 called for 8-bit assertion".to_string(),
            }),
        }
    }

    fn validate(&self, case_id: &str) -> Result<(), ManifestError> {
        self.address().map_err(|error| ManifestError::Invalid {
            message: format!("ROM case `{case_id}` has invalid assertion address: {error}"),
        })?;

        match self {
            Self::BusU8 { .. }
            | Self::WramU8 { .. }
            | Self::VramU8 { .. }
            | Self::CgramU8 { .. } => {
                self.expected_u8().map_err(|error| ManifestError::Invalid {
                    message: format!("ROM case `{case_id}` has invalid 8-bit assertion: {error}"),
                })?;
            }
            Self::BusU16 { .. }
            | Self::WramU16 { .. }
            | Self::VramU16 { .. }
            | Self::CgramU16 { .. } => {
                self.expected_u16()
                    .map_err(|error| ManifestError::Invalid {
                        message: format!(
                            "ROM case `{case_id}` has invalid 16-bit assertion: {error}"
                        ),
                    })?;
            }
        }

        Ok(())
    }
}

fn default_check_interval_steps() -> u64 {
    1024
}

fn parse_value(value: &str, label: &str) -> Result<u64, ManifestError> {
    let trimmed = value.trim();
    let (radix, digits) = match trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        Some(hex) => (16, hex),
        None => (10, trimmed),
    };

    u64::from_str_radix(digits, radix).map_err(|_| ManifestError::Invalid {
        message: format!("invalid {label} literal `{value}`"),
    })
}
