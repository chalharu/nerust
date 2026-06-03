// Copyright (c) 2026 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use nerust_contract_mirror::MirrorMode;

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
