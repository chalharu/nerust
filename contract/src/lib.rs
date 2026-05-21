// Copyright (c) 2026 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
pub enum MirrorMode {
    Horizontal,
    Vertical,
    Single0,
    Single1,
    Four,
    Custom([u8; 4]),
}

impl MirrorMode {
    pub fn try_from<'a>(mode: u8) -> Result<MirrorMode, &'a str> {
        match mode {
            0 => Ok(MirrorMode::Horizontal),
            1 => Ok(MirrorMode::Vertical),
            2 => Ok(MirrorMode::Single0),
            3 => Ok(MirrorMode::Single1),
            4 => Ok(MirrorMode::Four),
            _ => Err("parse error"),
        }
    }
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RomFormat {
    INes,
    Nes20,
}

impl RomFormat {
    pub const fn label(self) -> &'static str {
        match self {
            Self::INes => "iNES",
            Self::Nes20 => "NES 2.0",
        }
    }
}

#[derive(
    serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum Mmc3IrqVariant {
    #[default]
    Sharp,
    Nec,
}

#[derive(
    serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq,
)]
pub struct CoreOptions {
    pub mmc3_irq_variant: Option<Mmc3IrqVariant>,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomIdentity {
    pub format: RomFormat,
    pub mapper_type: u16,
    pub sub_mapper_type: u8,
    pub mirror_mode: MirrorMode,
    pub has_battery: bool,
    pub trainer_len: usize,
    pub prg_rom_len: usize,
    pub chr_rom_len: usize,
    pub prg_ram_len: usize,
    pub save_prg_ram_len: usize,
    pub chr_ram_len: usize,
    pub save_chr_ram_len: usize,
    pub prg_rom_crc64: u64,
    pub chr_rom_crc64: u64,
    pub trainer_crc64: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistenceTarget {
    pub rom_identity: RomIdentity,
    pub options: CoreOptions,
}
