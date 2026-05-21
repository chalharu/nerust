// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::MirrorMode;
use crate::cartridge_data::CartridgeData;
use crate::persistence::{
    MapperPersistentMemoryMessage, MapperStateMessage, PersistenceError, ProtoMappingMode,
    mirror_mode_from_proto, mirror_mode_to_proto,
};

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

impl MapperState {
    pub(crate) fn export_state_proto(&self) -> MapperStateMessage {
        MapperStateMessage {
            program_page_table: self
                .program_page_table
                .iter()
                .map(|page| page.map_or(-1, |value| value as i32))
                .collect(),
            character_page_table: self
                .character_page_table
                .iter()
                .map(|page| page.map_or(-1, |value| value as i32))
                .collect(),
            sram_page_table: self
                .sram_page_table
                .iter()
                .map(|page| page.map_or(-1, |value| value as i32))
                .collect(),
            sram: self.sram.clone(),
            vram: self.vram.clone(),
            mirror_mode: Some(mirror_mode_to_proto(self.mirror_mode)),
            has_battery: self.has_battery,
            character_mapping_mode: match self.character_mapping_mode {
                MappingMode::Ram => ProtoMappingMode::Ram,
                MappingMode::Rom => ProtoMappingMode::Rom,
            } as i32,
        }
    }

    pub(crate) fn import_state_proto(
        &mut self,
        program_rom_len: usize,
        character_rom_len: usize,
        payload: &MapperStateMessage,
    ) -> Result<(), PersistenceError> {
        if payload.program_page_table.len() != self.program_page_table.len()
            || payload.character_page_table.len() != self.character_page_table.len()
            || payload.sram_page_table.len() != self.sram_page_table.len()
        {
            return Err(PersistenceError::Validation(
                "mapper page table length mismatch".into(),
            ));
        }
        if payload.sram.len() != self.sram.len() || payload.vram.len() != self.vram.len() {
            return Err(PersistenceError::Validation(
                "mapper backing store length mismatch".into(),
            ));
        }
        let mirror_mode =
            mirror_mode_from_proto(payload.mirror_mode.as_ref().ok_or_else(|| {
                PersistenceError::Validation("missing mapper mirror mode".into())
            })?)?;
        let character_mapping_mode =
            match ProtoMappingMode::try_from(payload.character_mapping_mode)
                .map_err(|_| PersistenceError::Validation("unknown mapper mapping mode".into()))?
            {
                ProtoMappingMode::Ram => MappingMode::Ram,
                ProtoMappingMode::Rom => MappingMode::Rom,
            };
        if payload.has_battery != self.has_battery {
            return Err(PersistenceError::Validation(
                "mapper battery configuration mismatch".into(),
            ));
        }
        if character_mapping_mode != self.character_mapping_mode {
            return Err(PersistenceError::Validation(
                "mapper character mapping mode mismatch".into(),
            ));
        }

        decode_page_table(
            &payload.program_page_table,
            &mut self.program_page_table,
            program_rom_len >> 8,
            "program",
        )?;
        decode_page_table(
            &payload.character_page_table,
            &mut self.character_page_table,
            match self.character_mapping_mode {
                MappingMode::Ram => self.vram.len() >> 8,
                MappingMode::Rom => character_rom_len >> 8,
            },
            "character",
        )?;
        decode_page_table(
            &payload.sram_page_table,
            &mut self.sram_page_table,
            self.sram.len() >> 8,
            "SRAM",
        )?;

        self.sram.copy_from_slice(&payload.sram);
        self.vram.copy_from_slice(&payload.vram);
        self.mirror_mode = mirror_mode;
        self.has_battery = payload.has_battery;
        self.character_mapping_mode = character_mapping_mode;
        Ok(())
    }

    pub(crate) fn export_persistent_memory_proto(
        &self,
        cartridge_data: &CartridgeData,
    ) -> MapperPersistentMemoryMessage {
        let save_prg_len = cartridge_data.save_pram_length().min(self.sram.len());
        let save_chr_len = cartridge_data.save_vram_length().min(self.vram.len());
        MapperPersistentMemoryMessage {
            prg_ram: self.sram[..save_prg_len].to_vec(),
            chr_ram: self.vram[..save_chr_len].to_vec(),
        }
    }

    pub(crate) fn import_persistent_memory_proto(
        &mut self,
        cartridge_data: &CartridgeData,
        payload: &MapperPersistentMemoryMessage,
    ) -> Result<(), PersistenceError> {
        let save_prg_len = cartridge_data.save_pram_length();
        let save_chr_len = cartridge_data.save_vram_length();
        if payload.prg_ram.len() != save_prg_len || payload.chr_ram.len() != save_chr_len {
            return Err(PersistenceError::Validation(
                "persistent mapper memory length mismatch".into(),
            ));
        }
        if save_prg_len > self.sram.len() || save_chr_len > self.vram.len() {
            return Err(PersistenceError::Validation(
                "persistent mapper memory exceeds available backing store".into(),
            ));
        }
        self.sram[..save_prg_len].copy_from_slice(&payload.prg_ram);
        self.vram[..save_chr_len].copy_from_slice(&payload.chr_ram);
        Ok(())
    }
}

fn decode_page_table(
    payload: &[i32],
    destination: &mut [Option<usize>; 256],
    max_page_count: usize,
    label: &str,
) -> Result<(), PersistenceError> {
    for (index, (slot, page)) in destination
        .iter_mut()
        .zip(payload.iter().copied())
        .enumerate()
    {
        *slot = if page < 0 {
            None
        } else {
            let page = usize::try_from(page)
                .map_err(|_| PersistenceError::Validation(format!("{label} page overflow")))?;
            if page >= max_page_count {
                return Err(PersistenceError::Validation(format!(
                    "{label} page table entry {index} out of bounds"
                )));
            }
            Some(page)
        };
    }
    Ok(())
}

pub(crate) trait MapperStateDao {
    fn mapper_state_mut(&mut self) -> &mut MapperState;
    fn mapper_state_ref(&self) -> &MapperState;
}
