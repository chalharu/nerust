// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use nerust_contract_mirror::MirrorMode;
use nerust_contract_rom::RomFormat;

#[derive(Debug, Clone)]
pub struct CartridgeDataParts {
    pub format: RomFormat,
    pub prog_rom: Vec<u8>,
    pub char_rom: Vec<u8>,
    pub pram_length: usize,
    pub save_pram_length: usize,
    pub vram_length: usize,
    pub save_vram_length: usize,
    pub mapper_type: u16,
    pub mirror_mode: MirrorMode,
    pub has_battery: bool,
    pub sub_mapper_type: u8,
    pub trainer: Vec<u8>,
}
