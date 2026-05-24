// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use nerust_contract_mirror::MirrorMode;

#[derive(serde::Serialize, serde::Deserialize, Eq, PartialEq, Debug, Copy, Clone)]
pub(crate) enum MappingMode {
    Ram,
    Rom,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub(crate) struct MapperState {
    #[serde(with = "nerust_serialize::array::BigArray")]
    pub(crate) program_page_table: [Option<usize>; 256],
    #[serde(with = "nerust_serialize::array::BigArray")]
    pub(crate) character_page_table: [Option<usize>; 256],
    #[serde(with = "nerust_serialize::array::BigArray")]
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

    pub(crate) fn validate_for_import(
        &self,
        incoming: &MapperState,
        program_rom_len: usize,
        character_rom_len: usize,
    ) -> Result<(), String> {
        if incoming.sram.len() != self.sram.len() || incoming.vram.len() != self.vram.len() {
            return Err("mapper backing store length mismatch".into());
        }
        if incoming.has_battery != self.has_battery {
            return Err("mapper battery configuration mismatch".into());
        }
        if incoming.character_mapping_mode != self.character_mapping_mode {
            return Err("mapper character mapping mode mismatch".into());
        }

        let program_page_count = program_rom_len >> 8;
        let character_page_count = match self.character_mapping_mode {
            MappingMode::Ram => self.vram.len() >> 8,
            MappingMode::Rom => character_rom_len >> 8,
        };
        let sram_page_count = self.sram.len() >> 8;

        for (index, page) in incoming.program_page_table.iter().copied().enumerate() {
            validate_page_table_entry(page, program_page_count, "program", index)?;
        }
        for (index, page) in incoming.character_page_table.iter().copied().enumerate() {
            validate_page_table_entry(page, character_page_count, "character", index)?;
        }
        for (index, page) in incoming.sram_page_table.iter().copied().enumerate() {
            validate_page_table_entry(page, sram_page_count, "SRAM", index)?;
        }
        Ok(())
    }
}

impl Default for MapperState {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_page_table_entry(
    page: Option<usize>,
    max_page_count: usize,
    label: &str,
    index: usize,
) -> Result<(), String> {
    if let Some(page) = page
        && page >= max_page_count
    {
        return Err(format!("{label} page table entry {index} out of bounds"));
    }
    Ok(())
}

pub(crate) trait MapperStateDao {
    fn mapper_state_mut(&mut self) -> &mut MapperState;
    fn mapper_state_ref(&self) -> &MapperState;
}
