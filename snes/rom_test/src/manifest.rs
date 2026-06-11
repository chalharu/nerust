use serde_derive::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const APU_RAM_SIZE: u32 = 0x1_0000;

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
    #[serde(default)]
    pub expected_screen_hash: Option<String>,
    #[serde(default)]
    pub reference_png: Option<PathBuf>,
    #[serde(default)]
    pub assertions: Vec<Assertion>,
    #[serde(default)]
    pub reset_at_steps: Vec<u64>,
    #[serde(skip)]
    resolved_rom_path: PathBuf,
    #[serde(skip)]
    resolved_png_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Assertion {
    BusU8 { address: String, expected: String },
    BusU16 { address: String, expected: String },
    ApuRamU8 { address: String, expected: String },
    ApuRamU16 { address: String, expected: String },
    WramU8 { address: String, expected: String },
    WramU16 { address: String, expected: String },
    VramU8 { address: String, expected: String },
    VramU16 { address: String, expected: String },
    CgramU8 { address: String, expected: String },
    CgramU16 { address: String, expected: String },
    OamU8 { address: String, expected: String },
    OamU16 { address: String, expected: String },
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

    pub fn select<'a>(&'a self, case_ids: &[String]) -> Result<Vec<&'a RomCase>, ManifestError> {
        if !case_ids.is_empty() {
            let available = self
                .cases
                .iter()
                .map(|case| case.id.as_str())
                .collect::<BTreeSet<_>>();
            let missing = case_ids
                .iter()
                .map(String::as_str)
                .filter(|case_id| !available.contains(case_id))
                .collect::<BTreeSet<_>>();
            if !missing.is_empty() {
                return Err(ManifestError::Invalid {
                    message: format!(
                        "unknown ROM case id(s): {}",
                        missing.into_iter().collect::<Vec<_>>().join(", ")
                    ),
                });
            }
        }

        let mut selected = self
            .cases
            .iter()
            .filter(|case| case_ids.is_empty() || case_ids.contains(&case.id))
            .collect::<Vec<_>>();
        selected.sort_by(|left, right| left.id.cmp(&right.id));

        if selected.is_empty() {
            return Err(ManifestError::Invalid {
                message: if case_ids.is_empty() {
                    "no ROM cases matched all cases".to_string()
                } else {
                    format!("no ROM cases matched {}", case_ids.join(", "))
                },
            });
        }

        Ok(selected)
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

    pub fn png_path(&self) -> Option<&Path> {
        self.resolved_png_path.as_deref()
    }

    pub fn expected_screen_hash(&self) -> Result<Option<u64>, ManifestError> {
        self.expected_screen_hash
            .as_deref()
            .map(|value| parse_value(value, "expected_screen_hash"))
            .transpose()
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
        if self.assertions.is_empty()
            && self.expected_screen_hash.is_none()
            && self.reference_png.is_none()
        {
            return Err(ManifestError::Invalid {
                message: format!(
                    "ROM case `{}` must contain at least one assertion, expected_screen_hash, or reference_png",
                    self.id
                ),
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

        self.resolved_png_path = self.reference_png.as_ref().map(|png| rom_root.join(png));
        if let Some(ref png_path) = self.resolved_png_path
            && !png_path.is_file() {
                return Err(ManifestError::Invalid {
                    message: format!(
                        "ROM case `{}` points to missing reference PNG `{}`",
                        self.id,
                        png_path.display()
                    ),
                });
            }

        for assertion in &self.assertions {
            assertion.validate(&self.id)?;
        }
        if self.expected_screen_hash.is_some() {
            self.expected_screen_hash()
                .map_err(|error| ManifestError::Invalid {
                    message: format!(
                        "ROM case `{}` has invalid expected_screen_hash: {error}",
                        self.id
                    ),
                })?;
        }

        // Validate reset_at_steps is sorted and within max_steps
        for (i, step) in self.reset_at_steps.windows(2).enumerate() {
            if step[0] >= step[1] {
                return Err(ManifestError::Invalid {
                    message: format!(
                        "ROM case `{}` has unsorted or duplicate reset_at_steps values at index {}",
                        self.id, i
                    ),
                });
            }
        }
        if let Some(&last_step) = self.reset_at_steps.last()
            && last_step >= self.max_steps
        {
            return Err(ManifestError::Invalid {
                message: format!(
                    "ROM case `{}` has reset_at_steps value {} >= max_steps {}",
                    self.id, last_step, self.max_steps
                ),
            });
        }

        Ok(())
    }
}

impl Assertion {
    pub fn address(&self) -> Result<u32, ManifestError> {
        match self {
            Self::BusU8 { address, .. }
            | Self::BusU16 { address, .. }
            | Self::ApuRamU8 { address, .. }
            | Self::ApuRamU16 { address, .. }
            | Self::WramU8 { address, .. }
            | Self::WramU16 { address, .. }
            | Self::VramU8 { address, .. }
            | Self::VramU16 { address, .. }
            | Self::CgramU8 { address, .. }
            | Self::CgramU16 { address, .. }
            | Self::OamU8 { address, .. }
            | Self::OamU16 { address, .. } => parse_value(address, "address").and_then(|value| {
                u32::try_from(value).map_err(|_| ManifestError::Invalid {
                    message: format!("address `{address}` does not fit in u32"),
                })
            }),
        }
    }

    pub fn expected_u8(&self) -> Result<u8, ManifestError> {
        match self {
            Self::BusU8 { expected, .. }
            | Self::ApuRamU8 { expected, .. }
            | Self::WramU8 { expected, .. }
            | Self::VramU8 { expected, .. }
            | Self::CgramU8 { expected, .. }
            | Self::OamU8 { expected, .. } => parse_value(expected, "expected").and_then(|value| {
                u8::try_from(value).map_err(|_| ManifestError::Invalid {
                    message: format!("expected value `{expected}` does not fit in u8"),
                })
            }),
            Self::BusU16 { .. }
            | Self::ApuRamU16 { .. }
            | Self::WramU16 { .. }
            | Self::VramU16 { .. }
            | Self::CgramU16 { .. }
            | Self::OamU16 { .. } => Err(ManifestError::Invalid {
                message: "expected_u8 called for 16-bit assertion".to_string(),
            }),
        }
    }

    pub fn expected_u16(&self) -> Result<u16, ManifestError> {
        match self {
            Self::BusU16 { expected, .. }
            | Self::ApuRamU16 { expected, .. }
            | Self::WramU16 { expected, .. }
            | Self::VramU16 { expected, .. }
            | Self::CgramU16 { expected, .. }
            | Self::OamU16 { expected, .. } => {
                parse_value(expected, "expected").and_then(|value| {
                    u16::try_from(value).map_err(|_| ManifestError::Invalid {
                        message: format!("expected value `{expected}` does not fit in u16"),
                    })
                })
            }
            Self::BusU8 { .. }
            | Self::ApuRamU8 { .. }
            | Self::WramU8 { .. }
            | Self::VramU8 { .. }
            | Self::CgramU8 { .. }
            | Self::OamU8 { .. } => Err(ManifestError::Invalid {
                message: "expected_u16 called for 8-bit assertion".to_string(),
            }),
        }
    }

    fn validate(&self, case_id: &str) -> Result<(), ManifestError> {
        let address = self.address().map_err(|error| ManifestError::Invalid {
            message: format!("ROM case `{case_id}` has invalid assertion address: {error}"),
        })?;

        match self {
            Self::ApuRamU8 { .. } if address >= APU_RAM_SIZE => {
                return Err(ManifestError::Invalid {
                    message: format!(
                        "ROM case `{case_id}` has APU RAM u8 assertion address 0x{address:04X} outside 64 KiB APU RAM"
                    ),
                });
            }
            Self::ApuRamU16 { .. } if address >= APU_RAM_SIZE - 1 => {
                return Err(ManifestError::Invalid {
                    message: format!(
                        "ROM case `{case_id}` has APU RAM u16 assertion address 0x{address:04X} crossing 64 KiB APU RAM"
                    ),
                });
            }
            _ => {}
        }

        match self {
            Self::BusU8 { .. }
            | Self::ApuRamU8 { .. }
            | Self::WramU8 { .. }
            | Self::VramU8 { .. }
            | Self::CgramU8 { .. }
            | Self::OamU8 { .. } => {
                self.expected_u8().map_err(|error| ManifestError::Invalid {
                    message: format!("ROM case `{case_id}` has invalid 8-bit assertion: {error}"),
                })?;
            }
            Self::BusU16 { .. }
            | Self::ApuRamU16 { .. }
            | Self::WramU16 { .. }
            | Self::VramU16 { .. }
            | Self::CgramU16 { .. }
            | Self::OamU16 { .. } => {
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

#[cfg(test)]
mod tests {
    use super::{Assertion, RomCase, RomManifest};
    use std::path::PathBuf;

    fn dummy_case(id: &str) -> RomCase {
        RomCase {
            id: id.to_string(),
            description: format!("case {id}"),
            rom: PathBuf::from(format!("{id}.sfc")),
            max_steps: 1,
            check_interval_steps: 1,
            expected_screen_hash: None,
            reference_png: None,
            assertions: vec![Assertion::BusU8 {
                address: "0x00".to_string(),
                expected: "0x00".to_string(),
            }],
            reset_at_steps: vec![],
            resolved_rom_path: PathBuf::from(format!("/tmp/{id}.sfc")),
            resolved_png_path: None,
        }
    }

    #[test]
    fn select_rejects_unknown_requested_case_ids() {
        let manifest = RomManifest {
            rom_root: PathBuf::new(),
            cases: vec![dummy_case("alpha"), dummy_case("beta")],
        };

        let error = manifest
            .select(&["alpha".to_string(), "missing".to_string()])
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unknown ROM case id(s): missing"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn select_returns_cases_sorted_by_id() {
        let manifest = RomManifest {
            rom_root: PathBuf::new(),
            cases: vec![dummy_case("beta"), dummy_case("alpha")],
        };

        let selected = manifest.select(&[]).unwrap();

        assert_eq!(
            selected
                .iter()
                .map(|case| case.id.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );
    }

    #[test]
    fn expected_screen_hash_parses_hex_u64_values() {
        let mut case = dummy_case("hash");
        case.expected_screen_hash = Some("0x2F605F796DA9D7E0".to_string());

        assert_eq!(
            case.expected_screen_hash().unwrap(),
            Some(0x2F60_5F79_6DA9_D7E0)
        );
    }

    #[test]
    fn validation_rejects_out_of_bounds_apu_ram_assertions() {
        let assertion = Assertion::ApuRamU16 {
            address: "0xFFFF".to_string(),
            expected: "0x0000".to_string(),
        };

        let error = assertion.validate("apu-case").unwrap_err();

        assert!(
            error.to_string().contains("crossing 64 KiB APU RAM"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn validation_rejects_large_apu_ram_addresses_without_overflowing() {
        let assertion = Assertion::ApuRamU16 {
            address: "0xFFFFFFFF".to_string(),
            expected: "0x0000".to_string(),
        };

        let error = assertion.validate("apu-case").unwrap_err();

        assert!(
            error.to_string().contains("crossing 64 KiB APU RAM"),
            "unexpected error: {error}"
        );
    }
}
