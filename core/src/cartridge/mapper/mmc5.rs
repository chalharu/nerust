// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use super::CartridgeData;
use crate::OpenBusReadResult;
use crate::cart_device::Cartridge;
use crate::cpu::interrupt::{Interrupt, IrqSource};
use crate::mapper::{CartridgeDataDao, Mapper};
use crate::mapper_state::{MapperState, MapperStateDao};
use crate::persistence::{
    CartridgeRuntimeMessage, MAPPER_KIND_MMC5, PersistenceError, decode_message, encode_message,
};
use crate::ppu_bus_event::{PpuBusAccess, PpuBusEvent};
use crate::ppu_memory_access::PpuReadAccess;
use prost::Message;

mod audio;
mod ppu;
mod program;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, serde_derive::Serialize, serde_derive::Deserialize, PartialEq, Eq)]
enum ChrBankSet {
    Sprite,
    Background,
}

#[derive(
    Debug, Clone, Copy, serde_derive::Serialize, serde_derive::Deserialize, PartialEq, Eq, Default,
)]
struct SplitTileContext {
    column: u8,
    coarse_y: u8,
    fine_y: u8,
    uses_attribute_tiles: bool,
}

#[derive(Clone, PartialEq, Message)]
struct SplitTileContextMessage {
    #[prost(uint32, tag = "1")]
    column: u32,
    #[prost(uint32, tag = "2")]
    coarse_y: u32,
    #[prost(uint32, tag = "3")]
    fine_y: u32,
    #[prost(bool, tag = "4")]
    uses_attribute_tiles: bool,
}

#[derive(Clone, PartialEq, Message)]
struct Mmc5RuntimeMessage {
    #[prost(uint32, tag = "1")]
    prg_mode: u32,
    #[prost(uint32, tag = "2")]
    chr_mode: u32,
    #[prost(uint32, tag = "3")]
    prg_ram_protect_1: u32,
    #[prost(uint32, tag = "4")]
    prg_ram_protect_2: u32,
    #[prost(uint32, tag = "5")]
    exram_mode: u32,
    #[prost(uint32, repeated, tag = "6")]
    nametable_mapping: Vec<u32>,
    #[prost(uint32, tag = "7")]
    fill_tile: u32,
    #[prost(uint32, tag = "8")]
    fill_attribute: u32,
    #[prost(uint32, repeated, tag = "9")]
    prg_banks: Vec<u32>,
    #[prost(uint32, repeated, tag = "10")]
    sprite_chr_banks: Vec<u32>,
    #[prost(uint32, repeated, tag = "11")]
    background_chr_banks: Vec<u32>,
    #[prost(uint32, tag = "12")]
    chr_upper_bits: u32,
    #[prost(bool, tag = "13")]
    sprite_size_16: bool,
    #[prost(bool, tag = "14")]
    substitutions_enabled: bool,
    #[prost(uint32, tag = "15")]
    last_chr_bank_set: u32,
    #[prost(uint32, tag = "16")]
    current_background_tile_index: u32,
    #[prost(bytes = "vec", tag = "17")]
    exram: Vec<u8>,
    #[prost(bool, tag = "18")]
    split_enabled: bool,
    #[prost(bool, tag = "19")]
    split_right_side: bool,
    #[prost(uint32, tag = "20")]
    split_threshold: u32,
    #[prost(uint32, tag = "21")]
    split_scroll: u32,
    #[prost(uint32, tag = "22")]
    split_chr_bank: u32,
    #[prost(message, optional, tag = "23")]
    current_split_tile: Option<SplitTileContextMessage>,
    #[prost(uint32, tag = "24")]
    background_tile_fetches: u32,
    #[prost(uint32, tag = "25")]
    scanline_compare: u32,
    #[prost(bool, tag = "26")]
    scanline_irq_enabled: bool,
    #[prost(bool, tag = "27")]
    scanline_irq_pending: bool,
    #[prost(bool, tag = "28")]
    in_frame: bool,
    #[prost(uint32, tag = "29")]
    scanline_counter: u32,
    #[prost(uint32, optional, tag = "30")]
    matched_nametable_address: Option<u32>,
    #[prost(uint32, tag = "31")]
    matched_nametable_reads: u32,
    #[prost(bool, tag = "32")]
    scanline_detect_pending: bool,
    #[prost(bool, tag = "33")]
    ppu_read_seen_this_cpu_cycle: bool,
    #[prost(uint32, tag = "34")]
    idle_cpu_cycles: u32,
    #[prost(uint32, tag = "35")]
    multiplier_a: u32,
    #[prost(uint32, tag = "36")]
    multiplier_b: u32,
    #[prost(message, optional, tag = "37")]
    pulse_1: Option<audio::Mmc5PulseMessage>,
    #[prost(message, optional, tag = "38")]
    pulse_2: Option<audio::Mmc5PulseMessage>,
    #[prost(uint64, tag = "39")]
    audio_frame_accumulator: u64,
    #[prost(bool, tag = "40")]
    pcm_read_mode: bool,
    #[prost(bool, tag = "41")]
    pcm_irq_enabled: bool,
    #[prost(bool, tag = "42")]
    pcm_irq_pending: bool,
    #[prost(uint32, tag = "43")]
    pcm_output: u32,
    #[prost(bool, tag = "44")]
    mmc5a_cl3_input_mode: bool,
    #[prost(bool, tag = "45")]
    mmc5a_sl3_input_mode: bool,
    #[prost(bool, tag = "46")]
    mmc5a_cl3_read_strobe: bool,
    #[prost(bool, tag = "47")]
    mmc5a_sl3_write_strobe: bool,
    #[prost(bool, tag = "48")]
    mmc5a_cl3_strobe_low: bool,
    #[prost(bool, tag = "49")]
    mmc5a_sl3_strobe_low: bool,
    #[prost(bool, tag = "50")]
    mmc5a_cl3_output: bool,
    #[prost(bool, tag = "51")]
    mmc5a_sl3_output: bool,
    #[prost(uint32, tag = "52")]
    hardware_timer_counter: u32,
    #[prost(bool, tag = "53")]
    hardware_timer_running: bool,
    #[prost(bool, tag = "54")]
    hardware_timer_irq_pending: bool,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub(crate) struct Mmc5 {
    cartridge_data: CartridgeData,
    state: MapperState,
    prg_mode: u8,
    chr_mode: u8,
    prg_ram_protect_1: u8,
    prg_ram_protect_2: u8,
    exram_mode: u8,
    nametable_mapping: [u8; 4],
    fill_tile: u8,
    fill_attribute: u8,
    prg_banks: [u8; 5],
    sprite_chr_banks: [u16; 8],
    background_chr_banks: [u16; 4],
    chr_upper_bits: u8,
    sprite_size_16: bool,
    substitutions_enabled: bool,
    last_chr_bank_set: ChrBankSet,
    current_background_tile_index: usize,
    exram: Vec<u8>,
    split_enabled: bool,
    split_right_side: bool,
    split_threshold: u8,
    split_scroll: u8,
    split_chr_bank: u8,
    current_split_tile: Option<SplitTileContext>,
    background_tile_fetches: u8,
    scanline_compare: u8,
    scanline_irq_enabled: bool,
    scanline_irq_pending: bool,
    in_frame: bool,
    scanline_counter: u8,
    matched_nametable_address: Option<usize>,
    matched_nametable_reads: u8,
    scanline_detect_pending: bool,
    ppu_read_seen_this_cpu_cycle: bool,
    idle_cpu_cycles: u8,
    multiplier_a: u8,
    multiplier_b: u8,
    pulse_table: Vec<f32>,
    pcm_table: Vec<f32>,
    pulse_1: audio::Mmc5Pulse,
    pulse_2: audio::Mmc5Pulse,
    audio_frame_accumulator: u64,
    pcm_read_mode: bool,
    pcm_irq_enabled: bool,
    pcm_irq_pending: bool,
    pcm_output: u8,
    mmc5a_cl3_input_mode: bool,
    mmc5a_sl3_input_mode: bool,
    mmc5a_cl3_read_strobe: bool,
    mmc5a_sl3_write_strobe: bool,
    mmc5a_cl3_strobe_low: bool,
    mmc5a_sl3_strobe_low: bool,
    mmc5a_cl3_output: bool,
    mmc5a_sl3_output: bool,
    hardware_timer_counter: u16,
    hardware_timer_running: bool,
    hardware_timer_irq_pending: bool,
}

#[typetag::serde]
impl Cartridge for Mmc5 {
    fn export_runtime_proto(&self) -> Result<CartridgeRuntimeMessage, PersistenceError> {
        Ok(CartridgeRuntimeMessage {
            mapper_state: Some(self.state.export_state_proto()),
            mapper_specific_kind: MAPPER_KIND_MMC5.into(),
            mapper_specific_body: encode_message(&Mmc5RuntimeMessage {
                prg_mode: u32::from(self.prg_mode),
                chr_mode: u32::from(self.chr_mode),
                prg_ram_protect_1: u32::from(self.prg_ram_protect_1),
                prg_ram_protect_2: u32::from(self.prg_ram_protect_2),
                exram_mode: u32::from(self.exram_mode),
                nametable_mapping: self
                    .nametable_mapping
                    .iter()
                    .copied()
                    .map(u32::from)
                    .collect(),
                fill_tile: u32::from(self.fill_tile),
                fill_attribute: u32::from(self.fill_attribute),
                prg_banks: self.prg_banks.iter().copied().map(u32::from).collect(),
                sprite_chr_banks: self
                    .sprite_chr_banks
                    .iter()
                    .copied()
                    .map(u32::from)
                    .collect(),
                background_chr_banks: self
                    .background_chr_banks
                    .iter()
                    .copied()
                    .map(u32::from)
                    .collect(),
                chr_upper_bits: u32::from(self.chr_upper_bits),
                sprite_size_16: self.sprite_size_16,
                substitutions_enabled: self.substitutions_enabled,
                last_chr_bank_set: match self.last_chr_bank_set {
                    ChrBankSet::Sprite => 0,
                    ChrBankSet::Background => 1,
                },
                current_background_tile_index: self.current_background_tile_index as u32,
                exram: self.exram.clone(),
                split_enabled: self.split_enabled,
                split_right_side: self.split_right_side,
                split_threshold: u32::from(self.split_threshold),
                split_scroll: u32::from(self.split_scroll),
                split_chr_bank: u32::from(self.split_chr_bank),
                current_split_tile: self.current_split_tile.map(|tile| SplitTileContextMessage {
                    column: u32::from(tile.column),
                    coarse_y: u32::from(tile.coarse_y),
                    fine_y: u32::from(tile.fine_y),
                    uses_attribute_tiles: tile.uses_attribute_tiles,
                }),
                background_tile_fetches: u32::from(self.background_tile_fetches),
                scanline_compare: u32::from(self.scanline_compare),
                scanline_irq_enabled: self.scanline_irq_enabled,
                scanline_irq_pending: self.scanline_irq_pending,
                in_frame: self.in_frame,
                scanline_counter: u32::from(self.scanline_counter),
                matched_nametable_address: self.matched_nametable_address.map(|value| value as u32),
                matched_nametable_reads: u32::from(self.matched_nametable_reads),
                scanline_detect_pending: self.scanline_detect_pending,
                ppu_read_seen_this_cpu_cycle: self.ppu_read_seen_this_cpu_cycle,
                idle_cpu_cycles: u32::from(self.idle_cpu_cycles),
                multiplier_a: u32::from(self.multiplier_a),
                multiplier_b: u32::from(self.multiplier_b),
                pulse_1: Some(self.pulse_1.export_state_proto()),
                pulse_2: Some(self.pulse_2.export_state_proto()),
                audio_frame_accumulator: self.audio_frame_accumulator,
                pcm_read_mode: self.pcm_read_mode,
                pcm_irq_enabled: self.pcm_irq_enabled,
                pcm_irq_pending: self.pcm_irq_pending,
                pcm_output: u32::from(self.pcm_output),
                mmc5a_cl3_input_mode: self.mmc5a_cl3_input_mode,
                mmc5a_sl3_input_mode: self.mmc5a_sl3_input_mode,
                mmc5a_cl3_read_strobe: self.mmc5a_cl3_read_strobe,
                mmc5a_sl3_write_strobe: self.mmc5a_sl3_write_strobe,
                mmc5a_cl3_strobe_low: self.mmc5a_cl3_strobe_low,
                mmc5a_sl3_strobe_low: self.mmc5a_sl3_strobe_low,
                mmc5a_cl3_output: self.mmc5a_cl3_output,
                mmc5a_sl3_output: self.mmc5a_sl3_output,
                hardware_timer_counter: u32::from(self.hardware_timer_counter),
                hardware_timer_running: self.hardware_timer_running,
                hardware_timer_irq_pending: self.hardware_timer_irq_pending,
            })?,
        })
    }

    fn import_runtime_proto(
        &mut self,
        payload: &CartridgeRuntimeMessage,
    ) -> Result<(), PersistenceError> {
        let program_rom_len = self.data_ref().prog_rom_len();
        let character_rom_len = self.data_ref().char_rom_len();
        self.state.import_state_proto(
            program_rom_len,
            character_rom_len,
            payload
                .mapper_state
                .as_ref()
                .ok_or_else(|| PersistenceError::Validation("missing MMC5 mapper state".into()))?,
        )?;
        if payload.mapper_specific_kind != MAPPER_KIND_MMC5 {
            return Err(PersistenceError::Validation(
                "unexpected MMC5 runtime kind".into(),
            ));
        }
        let runtime = decode_message::<Mmc5RuntimeMessage>(&payload.mapper_specific_body)?;
        if runtime.nametable_mapping.len() != self.nametable_mapping.len()
            || runtime.prg_banks.len() != self.prg_banks.len()
            || runtime.sprite_chr_banks.len() != self.sprite_chr_banks.len()
            || runtime.background_chr_banks.len() != self.background_chr_banks.len()
        {
            return Err(PersistenceError::Validation(
                "MMC5 register length mismatch".into(),
            ));
        }
        self.prg_mode = u8::try_from(runtime.prg_mode)
            .map_err(|_| PersistenceError::Validation("MMC5 prg_mode overflow".into()))?;
        self.chr_mode = u8::try_from(runtime.chr_mode)
            .map_err(|_| PersistenceError::Validation("MMC5 chr_mode overflow".into()))?;
        self.prg_ram_protect_1 = u8::try_from(runtime.prg_ram_protect_1)
            .map_err(|_| PersistenceError::Validation("MMC5 prg_ram_protect_1 overflow".into()))?;
        self.prg_ram_protect_2 = u8::try_from(runtime.prg_ram_protect_2)
            .map_err(|_| PersistenceError::Validation("MMC5 prg_ram_protect_2 overflow".into()))?;
        self.exram_mode = u8::try_from(runtime.exram_mode)
            .map_err(|_| PersistenceError::Validation("MMC5 exram_mode overflow".into()))?;
        for (slot, value) in self
            .nametable_mapping
            .iter_mut()
            .zip(runtime.nametable_mapping)
        {
            *slot = u8::try_from(value).map_err(|_| {
                PersistenceError::Validation("MMC5 nametable mapping overflow".into())
            })?;
        }
        self.fill_tile = u8::try_from(runtime.fill_tile)
            .map_err(|_| PersistenceError::Validation("MMC5 fill_tile overflow".into()))?;
        self.fill_attribute = u8::try_from(runtime.fill_attribute)
            .map_err(|_| PersistenceError::Validation("MMC5 fill_attribute overflow".into()))?;
        for (slot, value) in self.prg_banks.iter_mut().zip(runtime.prg_banks) {
            *slot = u8::try_from(value)
                .map_err(|_| PersistenceError::Validation("MMC5 prg_banks overflow".into()))?;
        }
        for (slot, value) in self
            .sprite_chr_banks
            .iter_mut()
            .zip(runtime.sprite_chr_banks)
        {
            *slot = u16::try_from(value).map_err(|_| {
                PersistenceError::Validation("MMC5 sprite_chr_banks overflow".into())
            })?;
        }
        for (slot, value) in self
            .background_chr_banks
            .iter_mut()
            .zip(runtime.background_chr_banks)
        {
            *slot = u16::try_from(value).map_err(|_| {
                PersistenceError::Validation("MMC5 background_chr_banks overflow".into())
            })?;
        }
        self.chr_upper_bits = u8::try_from(runtime.chr_upper_bits)
            .map_err(|_| PersistenceError::Validation("MMC5 chr_upper_bits overflow".into()))?;
        self.sprite_size_16 = runtime.sprite_size_16;
        self.substitutions_enabled = runtime.substitutions_enabled;
        self.last_chr_bank_set = match runtime.last_chr_bank_set {
            0 => ChrBankSet::Sprite,
            1 => ChrBankSet::Background,
            _ => {
                return Err(PersistenceError::Validation(
                    "invalid MMC5 chr bank set".into(),
                ));
            }
        };
        self.current_background_tile_index = usize::try_from(runtime.current_background_tile_index)
            .map_err(|_| {
                PersistenceError::Validation("MMC5 background tile index overflow".into())
            })?;
        if runtime.exram.len() != self.exram.len() {
            return Err(PersistenceError::Validation(
                "MMC5 EXRAM length mismatch".into(),
            ));
        }
        self.exram.copy_from_slice(&runtime.exram);
        self.split_enabled = runtime.split_enabled;
        self.split_right_side = runtime.split_right_side;
        self.split_threshold = u8::try_from(runtime.split_threshold)
            .map_err(|_| PersistenceError::Validation("MMC5 split_threshold overflow".into()))?;
        self.split_scroll = u8::try_from(runtime.split_scroll)
            .map_err(|_| PersistenceError::Validation("MMC5 split_scroll overflow".into()))?;
        self.split_chr_bank = u8::try_from(runtime.split_chr_bank)
            .map_err(|_| PersistenceError::Validation("MMC5 split_chr_bank overflow".into()))?;
        self.current_split_tile = runtime
            .current_split_tile
            .map(|tile| {
                Ok::<_, PersistenceError>(SplitTileContext {
                    column: u8::try_from(tile.column).map_err(|_| {
                        PersistenceError::Validation("MMC5 split column overflow".into())
                    })?,
                    coarse_y: u8::try_from(tile.coarse_y).map_err(|_| {
                        PersistenceError::Validation("MMC5 split coarse_y overflow".into())
                    })?,
                    fine_y: u8::try_from(tile.fine_y).map_err(|_| {
                        PersistenceError::Validation("MMC5 split fine_y overflow".into())
                    })?,
                    uses_attribute_tiles: tile.uses_attribute_tiles,
                })
            })
            .transpose()?;
        self.background_tile_fetches =
            u8::try_from(runtime.background_tile_fetches).map_err(|_| {
                PersistenceError::Validation("MMC5 background_tile_fetches overflow".into())
            })?;
        self.scanline_compare = u8::try_from(runtime.scanline_compare)
            .map_err(|_| PersistenceError::Validation("MMC5 scanline_compare overflow".into()))?;
        self.scanline_irq_enabled = runtime.scanline_irq_enabled;
        self.scanline_irq_pending = runtime.scanline_irq_pending;
        self.in_frame = runtime.in_frame;
        self.scanline_counter = u8::try_from(runtime.scanline_counter)
            .map_err(|_| PersistenceError::Validation("MMC5 scanline_counter overflow".into()))?;
        self.matched_nametable_address = runtime
            .matched_nametable_address
            .map(|value| {
                usize::try_from(value).map_err(|_| {
                    PersistenceError::Validation("MMC5 matched nametable address overflow".into())
                })
            })
            .transpose()?;
        self.matched_nametable_reads =
            u8::try_from(runtime.matched_nametable_reads).map_err(|_| {
                PersistenceError::Validation("MMC5 matched nametable reads overflow".into())
            })?;
        self.scanline_detect_pending = runtime.scanline_detect_pending;
        self.ppu_read_seen_this_cpu_cycle = runtime.ppu_read_seen_this_cpu_cycle;
        self.idle_cpu_cycles = u8::try_from(runtime.idle_cpu_cycles)
            .map_err(|_| PersistenceError::Validation("MMC5 idle_cpu_cycles overflow".into()))?;
        self.multiplier_a = u8::try_from(runtime.multiplier_a)
            .map_err(|_| PersistenceError::Validation("MMC5 multiplier_a overflow".into()))?;
        self.multiplier_b = u8::try_from(runtime.multiplier_b)
            .map_err(|_| PersistenceError::Validation("MMC5 multiplier_b overflow".into()))?;
        self.pulse_1.import_state_proto(
            runtime
                .pulse_1
                .as_ref()
                .ok_or_else(|| PersistenceError::Validation("missing MMC5 pulse_1".into()))?,
        )?;
        self.pulse_2.import_state_proto(
            runtime
                .pulse_2
                .as_ref()
                .ok_or_else(|| PersistenceError::Validation("missing MMC5 pulse_2".into()))?,
        )?;
        self.audio_frame_accumulator = runtime.audio_frame_accumulator;
        self.pcm_read_mode = runtime.pcm_read_mode;
        self.pcm_irq_enabled = runtime.pcm_irq_enabled;
        self.pcm_irq_pending = runtime.pcm_irq_pending;
        self.pcm_output = u8::try_from(runtime.pcm_output)
            .map_err(|_| PersistenceError::Validation("MMC5 pcm_output overflow".into()))?;
        self.mmc5a_cl3_input_mode = runtime.mmc5a_cl3_input_mode;
        self.mmc5a_sl3_input_mode = runtime.mmc5a_sl3_input_mode;
        self.mmc5a_cl3_read_strobe = runtime.mmc5a_cl3_read_strobe;
        self.mmc5a_sl3_write_strobe = runtime.mmc5a_sl3_write_strobe;
        self.mmc5a_cl3_strobe_low = runtime.mmc5a_cl3_strobe_low;
        self.mmc5a_sl3_strobe_low = runtime.mmc5a_sl3_strobe_low;
        self.mmc5a_cl3_output = runtime.mmc5a_cl3_output;
        self.mmc5a_sl3_output = runtime.mmc5a_sl3_output;
        self.hardware_timer_counter =
            u16::try_from(runtime.hardware_timer_counter).map_err(|_| {
                PersistenceError::Validation("MMC5 hardware timer counter overflow".into())
            })?;
        self.hardware_timer_running = runtime.hardware_timer_running;
        self.hardware_timer_irq_pending = runtime.hardware_timer_irq_pending;
        Ok(())
    }

    fn read_character(&self, address: usize) -> OpenBusReadResult {
        self.read_character_with_access(address, PpuReadAccess::CpuData)
    }

    fn write_character(&mut self, address: usize, value: u8) {
        self.write_character_with_access(address, value, PpuReadAccess::CpuData);
    }

    fn read_ram(&self, address: usize) -> OpenBusReadResult {
        self.read_program_target(self.program_target_6000_7fff(address + 0x6000))
    }

    fn write_ram(&mut self, address: usize, value: u8, _interrupt: &mut Interrupt) {
        self.write_program_target(self.program_target_6000_7fff(address + 0x6000), value);
    }

    fn read_program(&self, address: usize) -> OpenBusReadResult {
        self.read_program_target(self.program_target_8000_ffff(address + 0x8000))
    }

    fn write_program(&mut self, address: usize, value: u8, _interrupt: &mut Interrupt) {
        let cpu_address = address + 0x8000;
        self.write_program_target(self.program_target_8000_ffff(cpu_address), value);
    }

    fn notify_ppu_ctrl(&mut self, value: u8) {
        self.sprite_size_16 = value & 0x20 != 0;
    }

    fn notify_ppu_mask(&mut self, value: u8) {
        let substitutions_enabled = value & 0x18 != 0;
        if !substitutions_enabled {
            self.end_frame_due_to_idle();
        } else if !self.substitutions_enabled {
            self.reset_frame_state(true);
        }
        self.substitutions_enabled = substitutions_enabled;
    }

    fn read_ppu_pattern(
        &mut self,
        address: usize,
        access: PpuReadAccess,
        _interrupt: &mut Interrupt,
    ) -> OpenBusReadResult {
        self.read_character_with_access(address, access)
    }

    fn write_ppu_pattern(&mut self, address: usize, value: u8, _interrupt: &mut Interrupt) {
        self.write_character_with_access(address, value, PpuReadAccess::CpuData);
    }

    fn read_ppu_nametable(
        &mut self,
        address: usize,
        access: PpuReadAccess,
        ciram: &mut [u8],
    ) -> OpenBusReadResult {
        let (table, offset) = Self::nametable_table_and_offset(address);
        if matches!(access, PpuReadAccess::BackgroundNameTable) {
            self.current_background_tile_index = offset & 0x03FF;
            self.current_split_tile =
                self.split_tile_context_for_fetch(self.background_tile_fetches);
            self.background_tile_fetches = self.background_tile_fetches.wrapping_add(1);
            if let Some(split_tile) = self.current_split_tile {
                return OpenBusReadResult::new(self.split_nametable_byte(split_tile), 0xFF);
            }
        }
        if matches!(access, PpuReadAccess::BackgroundAttribute)
            && let Some(split_tile) = self.current_split_tile
        {
            return OpenBusReadResult::new(self.split_attribute_byte(split_tile), 0xFF);
        }
        if matches!(access, PpuReadAccess::BackgroundAttribute)
            && self.extended_attributes_enabled()
        {
            return OpenBusReadResult::new(self.extended_attribute_byte(), 0xFF);
        }

        match self.nametable_mapping[table] {
            0 | 1 => OpenBusReadResult::new(
                ciram[(usize::from(self.nametable_mapping[table] & 0x01) << 10) | offset],
                0xFF,
            ),
            2 => {
                if self.exram_visible_to_ppu() {
                    OpenBusReadResult::new(self.exram[offset], 0xFF)
                } else {
                    OpenBusReadResult::new(0, 0xFF)
                }
            }
            3 => OpenBusReadResult::new(
                if offset >= 0x03C0 {
                    self.fill_attribute_byte()
                } else {
                    self.fill_tile
                },
                0xFF,
            ),
            _ => unreachable!(),
        }
    }

    fn write_ppu_nametable(
        &mut self,
        address: usize,
        value: u8,
        ciram: &mut [u8],
        _interrupt: &mut Interrupt,
    ) {
        let (table, offset) = Self::nametable_table_and_offset(address);
        match self.nametable_mapping[table] {
            0 | 1 => {
                let page = usize::from(self.nametable_mapping[table] & 0x01);
                ciram[(page << 10) | offset] = value;
            }
            2 if self.exram_visible_to_ppu() => self.exram[offset] = value,
            _ => {}
        }
    }

    fn peek_ppu_nametable(&self, address: usize, ciram: &[u8]) -> Option<u8> {
        let (table, offset) = Self::nametable_table_and_offset(address);
        Some(match self.nametable_mapping[table] {
            0 | 1 => {
                let page = usize::from(self.nametable_mapping[table] & 0x01);
                ciram[(page << 10) | offset]
            }
            2 => {
                if self.exram_visible_to_ppu() {
                    self.exram[offset]
                } else {
                    0
                }
            }
            3 => {
                if offset >= 0x03C0 {
                    self.fill_attribute_byte()
                } else {
                    self.fill_tile
                }
            }
            _ => unreachable!(),
        })
    }

    fn notify_cpu_read(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        match address {
            0x5010 => {
                self.pcm_irq_pending = false;
                self.update_external_irq(interrupt);
            }
            0x5204 => {
                self.scanline_irq_pending = false;
                self.update_external_irq(interrupt);
            }
            0x5209 => {
                self.hardware_timer_irq_pending = false;
                self.update_external_irq(interrupt);
            }
            0x5800..=0x5BFF if self.mmc5a_cl3_read_strobe => {
                self.mmc5a_cl3_strobe_low = true;
            }
            0x8000..=0xBFFF if self.pcm_read_mode => self.write_pcm_sample(value, interrupt),
            0xFFFA | 0xFFFB => {
                self.reset_frame_state(true);
                self.update_external_irq(interrupt);
            }
            _ => {}
        }
    }

    fn notify_ppu_status_read(&mut self, value: u8, interrupt: &mut Interrupt) {
        if value & 0x80 != 0 {
            self.reset_frame_state(true);
            self.update_external_irq(interrupt);
        }
    }

    fn notify_oam_dma(&mut self, interrupt: &mut Interrupt) {
        self.reset_frame_state(true);
        self.update_external_irq(interrupt);
    }

    fn expansion_audio_output(&self) -> f32 {
        self.audio_output()
    }

    fn expansion_audio_inverted(&self) -> bool {
        true
    }
}

impl Mmc5 {
    pub(crate) fn new(data: CartridgeData) -> Self {
        let pulse_table = (0..31)
            .map(|x| 95.52 / (8128.0 / x as f32 + 100.0))
            .collect::<Vec<_>>();
        let pcm_table = (0..=255)
            .map(|x| {
                if x == 0 {
                    0.0
                } else {
                    163.67 / (22638.0 / x as f32 + 100.0)
                }
            })
            .collect::<Vec<_>>();
        Self {
            cartridge_data: data,
            state: MapperState::new(),
            prg_mode: 3,
            chr_mode: 0,
            prg_ram_protect_1: 0x03,
            prg_ram_protect_2: 0x03,
            exram_mode: 0x03,
            nametable_mapping: [0, 0, 1, 1],
            fill_tile: 0,
            fill_attribute: 0,
            prg_banks: [0, 0, 0, 0, 0xFF],
            sprite_chr_banks: [0; 8],
            background_chr_banks: [0; 4],
            chr_upper_bits: 0,
            sprite_size_16: false,
            substitutions_enabled: false,
            last_chr_bank_set: ChrBankSet::Sprite,
            current_background_tile_index: 0,
            exram: vec![0; 0x400],
            split_enabled: false,
            split_right_side: false,
            split_threshold: 0,
            split_scroll: 0,
            split_chr_bank: 0,
            current_split_tile: None,
            background_tile_fetches: 0,
            scanline_compare: 0,
            scanline_irq_enabled: false,
            scanline_irq_pending: false,
            in_frame: false,
            scanline_counter: 0,
            matched_nametable_address: None,
            matched_nametable_reads: 0,
            scanline_detect_pending: false,
            ppu_read_seen_this_cpu_cycle: false,
            idle_cpu_cycles: 0,
            multiplier_a: 0,
            multiplier_b: 0,
            pulse_table,
            pcm_table,
            pulse_1: audio::Mmc5Pulse::new(),
            pulse_2: audio::Mmc5Pulse::new(),
            audio_frame_accumulator: 0,
            pcm_read_mode: false,
            pcm_irq_enabled: false,
            pcm_irq_pending: false,
            pcm_output: 0,
            mmc5a_cl3_input_mode: true,
            mmc5a_sl3_input_mode: true,
            mmc5a_cl3_read_strobe: false,
            mmc5a_sl3_write_strobe: false,
            mmc5a_cl3_strobe_low: false,
            mmc5a_sl3_strobe_low: false,
            mmc5a_cl3_output: false,
            mmc5a_sl3_output: false,
            hardware_timer_counter: 0,
            hardware_timer_running: false,
            hardware_timer_irq_pending: false,
        }
    }

    fn expand_chr_bank(&self, value: u8) -> u16 {
        u16::from(value) | (u16::from(self.chr_upper_bits & 0x03) << 8)
    }

    fn product(&self) -> u16 {
        u16::from(self.multiplier_a) * u16::from(self.multiplier_b)
    }

    fn split_active(&self) -> bool {
        self.substitutions_enabled
            && self.split_enabled
            && self.exram_mode <= 1
            && self.in_frame
            && self.scanline_counter < 240
    }

    fn split_tile_context_for_fetch(&self, fetch_index: u8) -> Option<SplitTileContext> {
        if !self.split_active() {
            return None;
        }
        let column = fetch_index & 0x1F;
        let in_split = if self.split_right_side {
            column >= self.split_threshold
        } else {
            column < self.split_threshold
        };
        if !in_split {
            return None;
        }
        let raw_split_y = usize::from(self.split_scroll) + usize::from(self.scanline_counter);
        let (split_y, uses_attribute_tiles) = if self.split_scroll < 240 {
            (raw_split_y % 240, false)
        } else if raw_split_y < 256 {
            (raw_split_y, true)
        } else {
            ((raw_split_y - 256) % 240, false)
        };
        Some(SplitTileContext {
            column,
            coarse_y: (split_y / 8) as u8,
            fine_y: (split_y & 0x07) as u8,
            uses_attribute_tiles,
        })
    }

    fn split_nametable_byte(&self, split_tile: SplitTileContext) -> u8 {
        if split_tile.uses_attribute_tiles {
            let attribute_row = usize::from(split_tile.coarse_y.saturating_sub(30));
            self.exram[0x03C0 + attribute_row * 32 + usize::from(split_tile.column)]
        } else {
            self.exram[usize::from(split_tile.coarse_y) * 32 + usize::from(split_tile.column)]
        }
    }

    fn split_attribute_byte(&self, split_tile: SplitTileContext) -> u8 {
        let attribute_index =
            0x03C0 + usize::from(split_tile.coarse_y / 4) * 8 + usize::from(split_tile.column / 4);
        let attribute = self.exram[attribute_index];
        let shift = (u8::from((split_tile.coarse_y & 0x02) != 0) << 2)
            | (u8::from((split_tile.column & 0x02) != 0) << 1);
        let palette = (attribute >> shift) & 0x03;
        palette | (palette << 2) | (palette << 4) | (palette << 6)
    }

    fn split_chr_address(&self, address: usize, split_tile: SplitTileContext) -> usize {
        usize::from(self.split_chr_bank) * 0x1000
            + ((address & 0x0FF8) | usize::from(split_tile.fine_y))
    }

    fn cl3_pin_level(&self) -> bool {
        if self.mmc5a_cl3_read_strobe {
            !self.mmc5a_cl3_strobe_low
        } else if self.mmc5a_cl3_input_mode {
            false
        } else {
            self.mmc5a_cl3_output
        }
    }

    fn sl3_pin_level(&self) -> bool {
        if self.mmc5a_sl3_write_strobe {
            !self.mmc5a_sl3_strobe_low
        } else if self.mmc5a_sl3_input_mode {
            false
        } else {
            self.mmc5a_sl3_output
        }
    }

    fn update_external_irq(&mut self, interrupt: &mut Interrupt) {
        if (self.scanline_irq_enabled && self.scanline_irq_pending)
            || (self.pcm_irq_enabled && self.pcm_irq_pending)
            || self.hardware_timer_irq_pending
        {
            interrupt.set_irq(IrqSource::EXTERNAL);
        } else {
            interrupt.clear_irq(IrqSource::EXTERNAL);
        }
    }

    fn reset_frame_state(&mut self, acknowledge_irq: bool) {
        self.in_frame = false;
        self.scanline_counter = 0;
        self.background_tile_fetches = 0;
        self.current_split_tile = None;
        self.matched_nametable_address = None;
        self.matched_nametable_reads = 0;
        self.scanline_detect_pending = false;
        if acknowledge_irq {
            self.scanline_irq_pending = false;
        }
    }

    fn end_frame_due_to_idle(&mut self) {
        self.in_frame = false;
        self.background_tile_fetches = 0;
        self.current_split_tile = None;
        self.matched_nametable_address = None;
        self.matched_nametable_reads = 0;
        self.scanline_detect_pending = false;
    }

    fn detect_scanline(&mut self) {
        self.background_tile_fetches = 0;
        self.current_split_tile = None;
        self.scanline_detect_pending = false;
        if !self.in_frame {
            self.in_frame = true;
            self.scanline_counter = 0;
            self.scanline_irq_pending = false;
            return;
        }
        self.scanline_counter = self.scanline_counter.wrapping_add(1);
        if self.scanline_counter >= 240 {
            self.reset_frame_state(true);
            return;
        }
        if self.scanline_compare != 0 && self.scanline_counter == self.scanline_compare {
            self.scanline_irq_pending = true;
        }
    }

    fn step_hardware_timer(&mut self, interrupt: &mut Interrupt) {
        if !self.hardware_timer_running {
            return;
        }

        let previous = self.hardware_timer_counter;
        if previous <= 1 {
            self.hardware_timer_counter = 0;
            self.hardware_timer_running = false;
            self.hardware_timer_irq_pending = true;
            self.update_external_irq(interrupt);
            return;
        }

        self.hardware_timer_counter = previous - 1;
    }
}

impl CartridgeDataDao for Mmc5 {
    fn data_mut(&mut self) -> &mut CartridgeData {
        &mut self.cartridge_data
    }

    fn data_ref(&self) -> &CartridgeData {
        &self.cartridge_data
    }
}

impl MapperStateDao for Mmc5 {
    fn mapper_state_mut(&mut self) -> &mut MapperState {
        &mut self.state
    }

    fn mapper_state_ref(&self) -> &MapperState {
        &self.state
    }
}

impl Mapper for Mmc5 {
    fn program_page_len(&self) -> usize {
        0x2000
    }

    fn character_page_len(&self) -> usize {
        0x0400
    }

    fn ram_len_default(&self) -> usize {
        0x10000
    }

    fn initialize(&mut self) {
        self.set_mirror_mode(match self.data_ref().mirror_mode() {
            crate::MirrorMode::Vertical => crate::MirrorMode::Vertical,
            crate::MirrorMode::Horizontal => crate::MirrorMode::Horizontal,
            mode => mode,
        });
    }

    fn name(&self) -> &str {
        "MMC5 (Mapper5)"
    }

    fn read_expansion(&self, address: usize) -> OpenBusReadResult {
        match address {
            0x5010 => OpenBusReadResult::new(
                if self.pcm_irq_pending && self.pcm_irq_enabled {
                    0x80
                } else {
                    0
                },
                0x80,
            ),
            0x5015 => OpenBusReadResult::new(self.read_audio_status(), 0x03),
            0x5208 => OpenBusReadResult::new(
                (if self.sl3_pin_level() { 0x80 } else { 0 })
                    | if self.cl3_pin_level() { 0x40 } else { 0 },
                0xC0,
            ),
            0x5209 => OpenBusReadResult::new(
                if self.hardware_timer_running {
                    0
                } else if self.hardware_timer_irq_pending {
                    0x80
                } else {
                    0
                },
                0x80,
            ),
            0x5C00..=0x5FFF => self.read_exram_cpu(address),
            0x5800..=0x5BFF => OpenBusReadResult::new(0, 0),
            0x5204 => OpenBusReadResult::new(
                (if self.scanline_irq_pending { 0x80 } else { 0 })
                    | (if self.in_frame { 0x40 } else { 0 }),
                0xC0,
            ),
            0x5205 => OpenBusReadResult::new(self.product() as u8, 0xFF),
            0x5206 => OpenBusReadResult::new((self.product() >> 8) as u8, 0xFF),
            _ => OpenBusReadResult::new(0, 0),
        }
    }

    fn write_expansion(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        match address {
            0x5000 => self.pulse_1.write_control(value),
            0x5001 => {}
            0x5002 => self.pulse_1.write_timer_low(value),
            0x5003 => self.pulse_1.write_timer_high(value),
            0x5004 => self.pulse_2.write_control(value),
            0x5005 => {}
            0x5006 => self.pulse_2.write_timer_low(value),
            0x5007 => self.pulse_2.write_timer_high(value),
            0x5010 => {
                self.pcm_irq_enabled = value & 0x80 != 0;
                self.pcm_read_mode = value & 0x01 != 0;
                self.update_external_irq(interrupt);
            }
            0x5011 => self.write_pcm_sample(value, interrupt),
            0x5015 => {
                self.pulse_1.set_enabled(value & 0x01 != 0);
                self.pulse_2.set_enabled(value & 0x02 != 0);
            }
            0x5100 => self.prg_mode = value & 0x03,
            0x5101 => self.chr_mode = value & 0x03,
            0x5102 => self.prg_ram_protect_1 = value & 0x03,
            0x5103 => self.prg_ram_protect_2 = value & 0x03,
            0x5104 => self.exram_mode = value & 0x03,
            0x5105 => {
                self.nametable_mapping = [
                    value & 0x03,
                    (value >> 2) & 0x03,
                    (value >> 4) & 0x03,
                    (value >> 6) & 0x03,
                ];
            }
            0x5106 => self.fill_tile = value,
            0x5107 => self.fill_attribute = value & 0x03,
            0x5113..=0x5117 => self.prg_banks[address - 0x5113] = value,
            0x5120..=0x5127 => {
                self.sprite_chr_banks[address - 0x5120] = self.expand_chr_bank(value);
                self.last_chr_bank_set = ChrBankSet::Sprite;
            }
            0x5128..=0x512B => {
                self.background_chr_banks[address - 0x5128] = self.expand_chr_bank(value);
                self.last_chr_bank_set = ChrBankSet::Background;
            }
            0x5130 => self.chr_upper_bits = value & 0x03,
            0x5200 => {
                self.split_enabled = value & 0x80 != 0;
                self.split_right_side = value & 0x40 != 0;
                self.split_threshold = value & 0x1F;
            }
            0x5201 => self.split_scroll = value,
            0x5202 => self.split_chr_bank = value,
            0x5203 => self.scanline_compare = value,
            0x5204 => {
                self.scanline_irq_enabled = value & 0x80 != 0;
                self.update_external_irq(interrupt);
            }
            0x5205 => self.multiplier_a = value,
            0x5206 => self.multiplier_b = value,
            0x5207 => {
                self.mmc5a_sl3_input_mode = value & 0x80 != 0;
                self.mmc5a_cl3_input_mode = value & 0x40 != 0;
                self.mmc5a_cl3_read_strobe = value & 0x02 != 0;
                self.mmc5a_sl3_write_strobe = value & 0x01 != 0;
            }
            0x5208 => {
                self.mmc5a_sl3_output = value & 0x80 != 0;
                self.mmc5a_cl3_output = value & 0x40 != 0;
            }
            0x5209 => {
                self.hardware_timer_counter =
                    (self.hardware_timer_counter & 0xFF00) | u16::from(value);
                self.hardware_timer_running = self.hardware_timer_counter != 0;
            }
            0x520A => {
                self.hardware_timer_counter =
                    (self.hardware_timer_counter & 0x00FF) | (u16::from(value) << 8);
                self.hardware_timer_running =
                    self.hardware_timer_running && self.hardware_timer_counter != 0;
            }
            0x5800..=0x5BFF if self.mmc5a_sl3_write_strobe => {
                self.mmc5a_sl3_strobe_low = true;
            }
            0x5800..=0x5BFF => {}
            0x5C00..=0x5FFF => self.write_exram_cpu(address, value),
            _ => {}
        }
    }

    fn step(&mut self, interrupt: &mut Interrupt) {
        self.step_hardware_timer(interrupt);
        if self.ppu_read_seen_this_cpu_cycle {
            self.idle_cpu_cycles = 0;
        } else {
            self.idle_cpu_cycles = self.idle_cpu_cycles.saturating_add(1);
            if self.idle_cpu_cycles >= 3 {
                self.end_frame_due_to_idle();
            }
        }
        self.ppu_read_seen_this_cpu_cycle = false;
        self.mmc5a_cl3_strobe_low = false;
        self.mmc5a_sl3_strobe_low = false;
        self.clock_audio(interrupt);
    }

    fn notify_ppu_bus_event(&mut self, event: PpuBusEvent, interrupt: &mut Interrupt) {
        let PpuBusEvent::AddressBusUpdate {
            address,
            from_cpu_register,
            access,
            ..
        } = event;
        if access == PpuBusAccess::Read && !from_cpu_register {
            self.ppu_read_seen_this_cpu_cycle = true;
            if self.scanline_detect_pending {
                self.detect_scanline();
            }
            if (0x2000..=0x2FFF).contains(&address) {
                if self.matched_nametable_address == Some(address) {
                    self.matched_nametable_reads = self.matched_nametable_reads.saturating_add(1);
                    if self.matched_nametable_reads >= 2 {
                        self.scanline_detect_pending = true;
                    }
                } else {
                    self.matched_nametable_reads = 0;
                }
                self.matched_nametable_address = Some(address);
            } else {
                self.matched_nametable_address = None;
                self.matched_nametable_reads = 0;
            }
        }
        self.update_external_irq(interrupt);
    }
}
