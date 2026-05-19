// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::MirrorMode;

#[derive(serde::Serialize, serde::Deserialize, Eq, PartialEq, Debug, Copy, Clone)]
pub(crate) enum MappingMode {
    Ram,
    Rom,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct MapperState {
    #[serde(with = "nerust_serialize::BigArray")]
    pub(crate) program_page_table: [Option<usize>; 256],
    #[serde(with = "nerust_serialize::BigArray")]
    pub(crate) character_page_table: [Option<usize>; 256],
    #[serde(with = "nerust_serialize::BigArray")]
    pub(crate) sram_page_table: [Option<usize>; 256],
    pub(crate) sram: Vec<u8>,
    pub(crate) vram: Vec<u8>,
    pub(crate) mirror_mode: MirrorMode,
    pub(crate) has_battery: bool,
    pub(crate) character_mapping_mode: MappingMode,
}

impl MapperState {
    pub(crate) fn new() -> Self {
        Self {
            program_page_table: [None; 256],
            character_page_table: [None; 256],
            sram_page_table: [None; 256],
            sram: Vec::new(),
            vram: Vec::new(),
            mirror_mode: MirrorMode::try_from(0).unwrap(),
            has_battery: false,
            character_mapping_mode: MappingMode::Rom,
        }
    }
}

impl Default for MapperState {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) trait MapperStateDao {
    fn mapper_state_mut(&mut self) -> &mut MapperState;
    fn mapper_state_ref(&self) -> &MapperState;
}
