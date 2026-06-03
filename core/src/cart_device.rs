// Copyright (c) 2018 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crate::OpenBusReadResult;
use crate::cartridge_runtime_state::CartridgeRuntimeState;
use crate::interrupt::Interrupt;
use crate::mapper::Mapper;
use crate::mapper_state::MappingMode;
use crate::persistence_error::PersistenceError;
use crate::ppu_memory_access::PpuReadAccess;
use nerust_contract_mirror::MirrorMode;
use std::cmp;

fn mirror_lut(mode: MirrorMode) -> [u8; 4] {
    match mode {
        MirrorMode::Horizontal => [0, 0, 1, 1],
        MirrorMode::Vertical => [0, 1, 0, 1],
        MirrorMode::Single0 => [0, 0, 0, 0],
        MirrorMode::Single1 => [1, 1, 1, 1],
        MirrorMode::Four => [0, 1, 2, 3],
        MirrorMode::Custom(lut) => lut,
    }
}

fn mirror_address(mode: MirrorMode, address: usize) -> usize {
    let vram_address = address & 0x0FFF;
    let table = vram_address >> 10;
    let offset = vram_address & 0x3FF;
    0x2000 + (usize::from(mirror_lut(mode)[table]) << 10) + offset
}

#[typetag::serde(tag = "type")]
pub(crate) trait Cartridge: Mapper {
    fn initialize(&mut self) {
        self.mapper_state_mut().has_battery =
            self.data_ref().has_battery() || self.battery_default();
        self.mapper_state_mut().sram = vec![
            0;
            cmp::max(
                self.data_ref().pram_length() + self.data_ref().save_pram_length(),
                self.ram_len_default()
            )
        ];
        if self.data_ref().char_rom_len() == 0 {
            self.mapper_state_mut().vram = vec![
                0;
                cmp::max(
                    self.data_ref().vram_length() + self.data_ref().save_vram_length(),
                    self.character_ram_page_len_default()
                )
            ];
            self.mapper_state_mut().character_mapping_mode = MappingMode::Ram;
        } else {
            self.mapper_state_mut().character_mapping_mode = MappingMode::Rom;
        };
        self.set_mirror_mode(self.data_ref().mirror_mode());
        Mapper::initialize(self);
    }

    fn read(&self, address: usize) -> OpenBusReadResult {
        match address {
            0..=0x1FFF => self.read_character(address),
            0x4020..=0x5FFF => Mapper::read_expansion(self, address),
            0x6000..=0x7FFF => Cartridge::read_ram(self, address - 0x6000),
            0x8000..=0xFFFF => self.read_program(address - 0x8000),
            _ => {
                log::error!("unhandled mapper read at address: 0x{:04X}", address);
                OpenBusReadResult::new(0, 0)
            }
        }
    }

    fn read_character(&self, address: usize) -> OpenBusReadResult {
        OpenBusReadResult::new(
            self.character_address(address).map_or_else(
                || {
                    self.character_openbus_default()
                        .unwrap_or((address & 0xFF) as u8)
                },
                |x| {
                    if self.mapper_state_ref().character_mapping_mode == MappingMode::Rom {
                        self.data_ref().read_char_rom(x)
                    } else {
                        self.mapper_state_ref().vram[x]
                    }
                },
            ),
            0xFF,
        )
    }

    fn read_ram(&self, address: usize) -> OpenBusReadResult {
        Mapper::read_ram(self, address).map_or_else(
            || OpenBusReadResult::new(0, 0),
            |x| OpenBusReadResult::new(x, 0xFF),
        )
    }

    fn read_program(&self, address: usize) -> OpenBusReadResult {
        OpenBusReadResult::new(
            self.program_address(address)
                .map(|x| self.data_ref().read_prog_rom(x))
                .unwrap_or(0),
            0xFF,
        )
    }

    fn write(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        match address {
            0..=0x1FFF => self.write_character(address, value),
            0x4020..=0x5FFF => Mapper::write_expansion(self, address, value, interrupt),
            0x6000..=0x7FFF => Cartridge::write_ram(self, address, value, interrupt),
            0x8000..=0xFFFF => self.write_program(address, value, interrupt),
            _ => {
                log::error!("unhandled mapper write at address: 0x{:04X}", address);
            }
        }
    }

    fn write_character(&mut self, address: usize, value: u8) {
        if self.mapper_state_ref().character_mapping_mode == MappingMode::Ram
            && let Some(addr) = self.character_address(address)
        {
            self.mapper_state_mut().vram[addr] = value;
        }
    }

    fn write_ram(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        if self.register_addr(address) {
            self.write_register(
                address,
                if self.bus_conflicts() {
                    Mapper::read_ram(self, address - 0x6000).unwrap_or(0) & value
                } else {
                    value
                },
                interrupt,
            );
        } else {
            Mapper::write_ram(self, address - 0x6000, value);
        }
    }

    fn write_program(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        if self.register_addr(address) {
            self.write_register(
                address,
                if self.bus_conflicts() {
                    self.read_program(address - 0x8000).data & value
                } else {
                    value
                },
                interrupt,
            );
        }
    }

    fn mirror_mode(&self) -> MirrorMode {
        self.get_mirror_mode()
    }

    fn notify_cpu_read(&mut self, _address: usize, _value: u8, _interrupt: &mut Interrupt) {}

    fn notify_ppu_status_read(&mut self, _value: u8, _interrupt: &mut Interrupt) {}

    fn notify_oam_dma(&mut self, _interrupt: &mut Interrupt) {}

    fn expansion_audio_output(&self) -> f32 {
        0.0
    }

    fn expansion_audio_inverted(&self) -> bool {
        false
    }

    fn expansion_audio_cpu_step_synchronized(&self) -> bool {
        false
    }

    fn persistent_mapper_save_lengths(&self) -> (usize, usize) {
        let data = self.data_ref();
        let prg_ram_len = if data.save_pram_length() > 0 {
            data.save_pram_length()
                .min(self.mapper_state_ref().sram.len())
        } else if self.mapper_state_ref().has_battery {
            let legacy_ines_prg_ram_len = data.pram_length();
            if legacy_ines_prg_ram_len > 0 {
                legacy_ines_prg_ram_len.min(self.mapper_state_ref().sram.len())
            } else {
                self.save_len_default()
                    .min(self.mapper_state_ref().sram.len())
            }
        } else {
            0
        };
        let chr_ram_len = data
            .save_vram_length()
            .min(self.mapper_state_ref().vram.len());
        (prg_ram_len, chr_ram_len)
    }

    fn has_persistent_mapper_save(&self) -> bool {
        let (prg_ram_len, chr_ram_len) = self.persistent_mapper_save_lengths();
        prg_ram_len > 0 || chr_ram_len > 0
    }

    fn export_runtime_state(&self) -> Result<CartridgeRuntimeState, PersistenceError> {
        Ok(CartridgeRuntimeState {
            mapper_state: self.mapper_state_ref().clone(),
            extra_kind: String::new(),
            extra_body: Vec::new(),
        })
    }

    fn import_runtime_state(
        &mut self,
        state: CartridgeRuntimeState,
    ) -> Result<(), PersistenceError> {
        if !state.extra_kind.is_empty() || !state.extra_body.is_empty() {
            return Err(PersistenceError::Validation(
                "unexpected mapper-specific state for this mapper".into(),
            ));
        }
        self.mapper_state_ref()
            .validate_for_import(
                &state.mapper_state,
                self.data_ref().prog_rom_len(),
                self.data_ref().char_rom_len(),
            )
            .map_err(PersistenceError::Validation)?;
        *self.mapper_state_mut() = state.mapper_state;
        Ok(())
    }

    fn export_mapper_save_state(&self) -> Result<(Vec<u8>, Vec<u8>), PersistenceError> {
        let (save_prg_len, save_chr_len) = self.persistent_mapper_save_lengths();
        Ok((
            self.mapper_state_ref().sram[..save_prg_len].to_vec(),
            self.mapper_state_ref().vram[..save_chr_len].to_vec(),
        ))
    }

    fn import_mapper_save_state(
        &mut self,
        prg_ram: &[u8],
        chr_ram: &[u8],
    ) -> Result<(), PersistenceError> {
        let (save_prg_len, save_chr_len) = self.persistent_mapper_save_lengths();
        if save_prg_len == 0 && save_chr_len == 0 {
            return Err(PersistenceError::Validation(
                "cartridge does not expose persistent mapper save memory".into(),
            ));
        }
        if prg_ram.len() != save_prg_len || chr_ram.len() != save_chr_len {
            return Err(PersistenceError::Validation(
                "persistent mapper memory length mismatch".into(),
            ));
        }
        let sram_len = self.mapper_state_ref().sram.len();
        let vram_len = self.mapper_state_ref().vram.len();
        if save_prg_len > sram_len || save_chr_len > vram_len {
            return Err(PersistenceError::Validation(
                "persistent mapper memory exceeds available backing store".into(),
            ));
        }
        self.mapper_state_mut().sram[..save_prg_len].copy_from_slice(prg_ram);
        self.mapper_state_mut().vram[..save_chr_len].copy_from_slice(chr_ram);
        Ok(())
    }

    fn notify_ppu_ctrl(&mut self, _value: u8) {}

    fn notify_ppu_mask(&mut self, _value: u8) {}

    fn read_ppu_pattern(
        &mut self,
        address: usize,
        _access: PpuReadAccess,
        _interrupt: &mut Interrupt,
    ) -> OpenBusReadResult {
        self.read(address)
    }

    fn write_ppu_pattern(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        self.write(address, value, interrupt);
    }

    fn read_ppu_nametable(
        &mut self,
        address: usize,
        _access: PpuReadAccess,
        ciram: &mut [u8],
    ) -> OpenBusReadResult {
        OpenBusReadResult::new(
            ciram[mirror_address(self.mirror_mode(), address) & 0x7FF],
            0xFF,
        )
    }

    fn write_ppu_nametable(
        &mut self,
        address: usize,
        value: u8,
        ciram: &mut [u8],
        _interrupt: &mut Interrupt,
    ) {
        ciram[mirror_address(self.mirror_mode(), address) & 0x7FF] = value;
    }

    fn peek_ppu_nametable(&self, address: usize, ciram: &[u8]) -> Option<u8> {
        Some(ciram[mirror_address(self.mirror_mode(), address) & 0x7FF])
    }
}

// 本当はこうしたい
// #[typetag::serde]
// impl<T: Mapper> Cartridge for T {}
