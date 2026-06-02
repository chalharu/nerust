#![allow(
    dead_code,
    reason = "emulator components include future mapper/APU hooks"
)]

// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod apu;
mod cart_device;
mod cartridge;
mod cartridge_bus;
pub mod cartridge_data_parts;
pub mod cartridge_error;
pub mod cartridge_rom;
mod cartridge_runtime_state;
pub mod controller;
mod cpu;
mod interrupt;
mod mapper;
mod mapper_state;
mod persistence_codec;
mod persistence_error;
mod ppu;
mod ppu_memory_access;

use self::apu::Core as Apu;
use self::cart_device::Cartridge;
#[cfg(test)]
use self::cartridge_data_parts::CartridgeDataParts;
use self::cartridge_rom::CartridgeData;
use self::cartridge_runtime_state::CartridgeRuntimeState;
use self::controller::Controller;
use self::cpu::Core as Cpu;
use self::persistence_codec::{
    PERSISTENCE_SCHEMA_VERSION, decode_payload, encode_payload, validate_schema_version,
};
use self::persistence_error::PersistenceError;
use self::ppu::Core as Ppu;
use nerust_contract_mirror::MirrorMode;
use nerust_contract_options::CoreOptions;
#[cfg(test)]
use nerust_contract_options::Mmc3IrqVariant;
use nerust_contract_rom::{RomFormat, RomIdentity};
use nerust_crc64_hasher::crc64;
use nerust_screen_video::Screen;
use nerust_sound_traits::MixerInput;

pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomInfo {
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
    pub raw_file_len: usize,
    pub body_len: usize,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
pub struct Core {
    cpu: Cpu,
    ppu: Ppu,
    apu: Apu,
    cartridge: Box<dyn Cartridge>,
    options: CoreOptions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ApuBatchMode {
    Exact,
    Batched,
    SynchronizedExpansionAudio,
}

/// Core-owned mapper-save payload.
///
/// This payload is intentionally scoped to battery-backed mapper RAM/VRAM plus the ROM identity
/// needed to reject incompatible imports. The `options` field is recorded for diagnostics and
/// fixture visibility today, but mapper-save import compatibility is currently enforced only by
/// `rom_identity`; changing that policy would require an explicit compatibility decision.
#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct MapperSavePayload {
    schema_version: u32,
    rom_identity: RomIdentity,
    options: CoreOptions,
    #[serde(with = "serde_bytes")]
    prg_ram: Vec<u8>,
    #[serde(with = "serde_bytes")]
    chr_ram: Vec<u8>,
}

/// Core-owned full machine-state payload.
///
/// This schema owns CPU/PPU/APU/cartridge runtime bytes and the import validation that protects
/// them. Both `rom_identity` and `options` are part of the compatibility contract for imports, so
/// any incompatible change here requires a `PERSISTENCE_SCHEMA_VERSION` bump.
#[derive(serde_derive::Serialize, serde_derive::Deserialize)]
struct MachineStatePayload {
    schema_version: u32,
    rom_identity: RomIdentity,
    options: CoreOptions,
    cpu: Cpu,
    ppu: Ppu,
    apu: Apu,
    cartridge: CartridgeRuntimeState,
}

impl Core {
    const INSTRUCTION_SCHEDULER_MAX_BATCH_CYCLES: u64 = 24;
    const INSTRUCTION_SCHEDULER_DISABLE_MISSES: u64 = 2048;

    pub fn new(cartridge_data: CartridgeData) -> Result<Core, Error> {
        Self::new_with_options(cartridge_data, CoreOptions::default())
    }

    pub fn new_with_options(
        cartridge_data: CartridgeData,
        options: CoreOptions,
    ) -> Result<Core, Error> {
        cartridge_data.validate()?;
        let mut cpu = Cpu::new();
        let cartridge = cartridge::try_from_with_options(cartridge_data, options)?;
        let apu = Apu::new(cpu.interrupt_mut());
        Ok(Self {
            cpu,
            ppu: Ppu::new(),
            apu,
            cartridge,
            options,
        })
    }

    pub fn reset(&mut self) {
        self.cpu.reset();
        self.ppu.reset();
        self.apu.reset(self.cpu.interrupt_mut());
    }

    pub fn peek_work_ram(&self, address: usize) -> Option<u8> {
        self.cpu.peek_work_ram(address)
    }

    pub fn peek_cartridge_ram(&self, address: usize) -> Option<OpenBusReadResult> {
        if (0x6000..=0x7FFF).contains(&address) {
            Some(self.cartridge.read(address))
        } else {
            None
        }
    }

    pub fn inspect_cartridge(
        cartridge_data: &CartridgeData,
        raw_file_len: usize,
    ) -> Result<RomInfo, Error> {
        let mapper = cartridge::try_from(cartridge_data.clone())?;
        let save_prg_ram_len = if cartridge_data.save_pram_length() > 0 {
            cartridge_data.save_pram_length()
        } else if cartridge_data.has_battery() {
            if cartridge_data.pram_length() > 0 {
                cartridge_data.pram_length()
            } else {
                mapper.save_len_default()
            }
        } else {
            0
        };
        Ok(RomInfo {
            format: cartridge_data.format(),
            mapper_type: cartridge_data.mapper_type(),
            sub_mapper_type: cartridge_data.sub_mapper_type(),
            mirror_mode: cartridge_data.mirror_mode(),
            has_battery: cartridge_data.has_battery(),
            trainer_len: cartridge_data.trainer().len(),
            prg_rom_len: cartridge_data.prog_rom_len(),
            chr_rom_len: cartridge_data.char_rom_len(),
            prg_ram_len: cartridge_data.pram_length(),
            save_prg_ram_len,
            chr_ram_len: cartridge_data.vram_length(),
            save_chr_ram_len: cartridge_data.save_vram_length(),
            raw_file_len,
            body_len: raw_file_len.saturating_sub(16),
        })
    }

    pub fn peek_ppu_vram(&self, address: usize) -> Option<u8> {
        self.ppu.peek_vram(address, self.cartridge.as_ref())
    }

    pub fn rom_identity(&self) -> RomIdentity {
        let data = self.cartridge.data_ref();
        RomIdentity {
            format: data.format(),
            mapper_type: data.mapper_type(),
            sub_mapper_type: data.sub_mapper_type(),
            mirror_mode: data.mirror_mode(),
            has_battery: data.has_battery(),
            trainer_len: data.trainer().len(),
            prg_rom_len: data.prog_rom_len(),
            chr_rom_len: data.char_rom_len(),
            prg_ram_len: data.pram_length(),
            save_prg_ram_len: data.save_pram_length(),
            chr_ram_len: data.vram_length(),
            save_chr_ram_len: data.save_vram_length(),
            prg_rom_crc64: crc64(data.prog_rom()),
            chr_rom_crc64: crc64(data.char_rom()),
            trainer_crc64: crc64(data.trainer()),
        }
    }

    pub fn options(&self) -> CoreOptions {
        self.options
    }

    pub fn has_persistent_mapper_save(&self) -> bool {
        self.cartridge.has_persistent_mapper_save()
    }

    pub fn export_mapper_save(&self) -> Result<Option<Vec<u8>>, Error> {
        if !self.has_persistent_mapper_save() {
            return Ok(None);
        }
        let (prg_ram, chr_ram) = self.cartridge.export_mapper_save_state()?;
        let payload = MapperSavePayload {
            schema_version: PERSISTENCE_SCHEMA_VERSION,
            rom_identity: self.rom_identity(),
            options: self.options,
            prg_ram,
            chr_ram,
        };
        Ok(Some(encode_payload(&payload)?))
    }

    pub fn import_mapper_save(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let payload: MapperSavePayload = decode_payload(bytes)?;
        validate_schema_version(payload.schema_version)?;
        if payload.rom_identity != self.rom_identity() {
            return Err(PersistenceError::Validation("ROM identity mismatch".into()).into());
        }
        self.cartridge
            .import_mapper_save_state(&payload.prg_ram, &payload.chr_ram)?;
        Ok(())
    }

    pub fn export_machine_state(&self) -> Result<Vec<u8>, Error> {
        let payload = MachineStatePayload {
            schema_version: PERSISTENCE_SCHEMA_VERSION,
            rom_identity: self.rom_identity(),
            options: self.options,
            cpu: self.cpu.clone(),
            ppu: self.ppu.clone(),
            apu: self.apu.clone(),
            cartridge: self.cartridge.export_runtime_state()?,
        };
        Ok(encode_payload(&payload)?)
    }

    pub fn import_machine_state(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let payload: MachineStatePayload = decode_payload(bytes)?;
        validate_schema_version(payload.schema_version)?;
        let rom_identity = payload.rom_identity;
        let options = payload.options;
        self.validate_persistence_target(rom_identity, options)?;
        let cpu = payload.cpu;
        cpu.validate_runtime_state()?;
        let ppu = payload.ppu;
        ppu.validate_runtime_state()?;
        let apu = payload.apu;
        apu.validate_runtime_state()?;
        self.cartridge.import_runtime_state(payload.cartridge)?;
        self.cpu = cpu;
        self.ppu = ppu;
        self.apu = apu;
        Ok(())
    }

    pub fn step<S: Screen, M: MixerInput>(
        &mut self,
        screen: &mut S,
        controller: &mut dyn Controller,
        mixer: &mut M,
    ) -> bool {
        self.step_cycle(screen, controller, mixer, mixer.sample_rate())
    }

    pub fn run_frame<S: Screen, M: MixerInput>(
        &mut self,
        screen: &mut S,
        controller: &mut dyn Controller,
        mixer: &mut M,
    ) -> u64 {
        let mut cycles = 0;
        let mixer_sample_rate = mixer.sample_rate();
        let apu_batch_mode = self.apu_batch_mode(mixer_sample_rate);
        let mut scheduler_enabled = true;
        let mut scheduler_misses = 0;
        let mut scheduler_cycles = 0;
        loop {
            if scheduler_enabled {
                if let Some((elapsed_cycles, screen_updated)) = self.step_instruction_event(
                    screen,
                    controller,
                    mixer,
                    mixer_sample_rate,
                    apu_batch_mode,
                ) {
                    cycles += elapsed_cycles;
                    scheduler_cycles += elapsed_cycles;
                    scheduler_misses = 0;
                    if screen_updated {
                        return cycles;
                    }
                    continue;
                } else {
                    scheduler_misses += 1;
                    if scheduler_misses >= Self::INSTRUCTION_SCHEDULER_DISABLE_MISSES
                        && scheduler_cycles * 4 < cycles
                    {
                        scheduler_enabled = false;
                    }
                }
            }

            cycles += 1;
            if self.step_cycle(screen, controller, mixer, mixer_sample_rate) {
                return cycles;
            }
        }
    }

    #[inline(always)]
    fn step_cycle<S: Screen, M: MixerInput>(
        &mut self,
        screen: &mut S,
        controller: &mut dyn Controller,
        mixer: &mut M,
        mixer_sample_rate: u32,
    ) -> bool {
        // 1CPUサイクルにつき、APUは1、PPUはNTSC=>3,PAL=>3.2となる
        let mut result = false;
        self.cpu.step(
            &mut self.ppu,
            self.cartridge.as_mut(),
            controller,
            &mut self.apu,
        );
        let mut ppu_cartridge = crate::cartridge_bus::mapper_cartridge_bus(self.cartridge.as_mut());
        if self
            .ppu
            .step_exact_many(screen, &mut ppu_cartridge, self.cpu.interrupt_mut(), 3)
        {
            result = true;
        }
        self.cartridge.step(self.cpu.interrupt_mut());
        self.apu.step(
            &mut self.cpu,
            mixer,
            mixer_sample_rate,
            self.cartridge.expansion_audio_output(),
            self.cartridge.expansion_audio_inverted(),
        );

        result
    }

    #[inline]
    fn apu_batch_mode(&self, mixer_sample_rate: u32) -> ApuBatchMode {
        if self.cartridge.expansion_audio_cpu_step_synchronized() {
            ApuBatchMode::SynchronizedExpansionAudio
        } else if Apu::should_step_many_exact(mixer_sample_rate) {
            ApuBatchMode::Exact
        } else {
            ApuBatchMode::Batched
        }
    }

    fn step_instruction_event<S: Screen, M: MixerInput>(
        &mut self,
        screen: &mut S,
        controller: &mut dyn Controller,
        mixer: &mut M,
        mixer_sample_rate: u32,
        apu_batch_mode: ApuBatchMode,
    ) -> Option<(u64, bool)> {
        if !self.cartridge.allow_instruction_fast_path() {
            return None;
        }

        let mut instruction_cycles = self
            .cpu
            .instruction_fast_path_max_cycles(self.cartridge.as_ref())?;
        let max_cpu_cycles = self.scheduler_fast_path_window();
        if max_cpu_cycles < instruction_cycles {
            return None;
        }

        let mut cpu_cycles = 0;
        while cpu_cycles + instruction_cycles <= max_cpu_cycles {
            if instruction_cycles == 0 {
                break;
            }
            let elapsed = self.cpu.step_fast_path_instruction(
                &mut self.ppu,
                self.cartridge.as_mut(),
                controller,
                &mut self.apu,
            )?;
            debug_assert_eq!(elapsed, instruction_cycles);
            cpu_cycles += elapsed;
            if cpu_cycles >= max_cpu_cycles {
                break;
            }
            let Some(next_instruction_cycles) = self
                .cpu
                .instruction_fast_path_max_cycles(self.cartridge.as_ref())
            else {
                break;
            };
            instruction_cycles = next_instruction_cycles;
        }
        if cpu_cycles == 0 {
            return None;
        }

        let ppu_cycles = cpu_cycles * 3;
        let screen_updated = {
            let mut ppu_cartridge =
                crate::cartridge_bus::mapper_cartridge_bus(self.cartridge.as_mut());
            self.ppu.step_many(
                screen,
                &mut ppu_cartridge,
                self.cpu.interrupt_mut(),
                ppu_cycles,
            )
        };
        match apu_batch_mode {
            ApuBatchMode::Exact => {
                self.cartridge
                    .step_cpu_cycles(cpu_cycles, self.cpu.interrupt_mut());
                self.apu.step_many(
                    &mut self.cpu,
                    mixer,
                    mixer_sample_rate,
                    self.cartridge.expansion_audio_output(),
                    self.cartridge.expansion_audio_inverted(),
                    cpu_cycles,
                );
            }
            ApuBatchMode::Batched => {
                self.cartridge
                    .step_cpu_cycles(cpu_cycles, self.cpu.interrupt_mut());
                self.apu.step_many_batched(
                    &mut self.cpu,
                    mixer,
                    mixer_sample_rate,
                    self.cartridge.expansion_audio_output(),
                    self.cartridge.expansion_audio_inverted(),
                    cpu_cycles,
                );
            }
            ApuBatchMode::SynchronizedExpansionAudio => {
                self.step_synchronized_mapper_and_apu(mixer, mixer_sample_rate, cpu_cycles);
            }
        }

        Some((cpu_cycles, screen_updated))
    }

    fn step_synchronized_mapper_and_apu<M: MixerInput>(
        &mut self,
        mixer: &mut M,
        mixer_sample_rate: u32,
        cpu_cycles: u64,
    ) {
        // MMC5 expansion audio is clocked by mapper CPU cycles and sampled by the APU.
        // Advance the mapper before the APU for each sample-bounded segment to preserve
        // the exact per-cycle order used by step_cycle.
        let mut remaining = cpu_cycles;
        while remaining > 0 {
            let segment = remaining.min(self.apu.cycles_until_next_sample(mixer_sample_rate));
            self.cartridge
                .step_cpu_cycles(segment, self.cpu.interrupt_mut());
            self.apu.step_many(
                &mut self.cpu,
                mixer,
                mixer_sample_rate,
                self.cartridge.expansion_audio_output(),
                self.cartridge.expansion_audio_inverted(),
                segment,
            );
            remaining -= segment;
        }
    }

    fn scheduler_fast_path_window(&self) -> u64 {
        let max_cpu_cycles = Self::INSTRUCTION_SCHEDULER_MAX_BATCH_CYCLES;
        let ppu_event_cycles = self
            .ppu
            .cycles_until_next_scheduler_event(max_cpu_cycles * 3);
        let ppu_safe_cpu_cycles = ppu_event_cycles.saturating_sub(1) / 3;
        let apu_safe_cpu_cycles = self
            .apu
            .cycles_until_next_scheduler_event(self.cpu.interrupt_ref(), max_cpu_cycles)
            .saturating_sub(1);
        let mapper_safe_cpu_cycles = self
            .cartridge
            .cycles_until_next_cpu_event()
            .saturating_sub(1);

        max_cpu_cycles
            .min(ppu_safe_cpu_cycles)
            .min(apu_safe_cpu_cycles)
            .min(mapper_safe_cpu_cycles)
    }

    fn validate_persistence_target(
        &self,
        identity: RomIdentity,
        options: CoreOptions,
    ) -> Result<(), PersistenceError> {
        if self.rom_identity() != identity {
            return Err(PersistenceError::Validation("ROM identity mismatch".into()));
        }
        if self.options != options {
            return Err(PersistenceError::Validation(
                "runtime options mismatch".into(),
            ));
        }
        Ok(())
    }
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone, Copy)]
struct OpenBus {
    data: u8,
}

impl OpenBus {
    pub(crate) fn new() -> Self {
        Self { data: 0 }
    }

    pub(crate) fn unite(&mut self, data: OpenBusReadResult) -> u8 {
        let result = (self.data & !data.mask) | (data.data & data.mask);
        self.data = result;
        result
    }
}

#[derive(Debug, Copy, Clone)]
pub struct OpenBusReadResult {
    pub data: u8,
    pub mask: u8,
}

impl OpenBusReadResult {
    pub fn new(data: u8, mask: u8) -> Self {
        Self { data, mask }
    }
}

#[cfg(test)]
fn nrom_test_cartridge() -> Box<dyn Cartridge> {
    cartridge::try_from(
        CartridgeData::new(CartridgeDataParts {
            format: RomFormat::INes,
            prog_rom: vec![0; 0x8000],
            char_rom: vec![0; 0x2000],
            pram_length: 0,
            save_pram_length: 0,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type: 0,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: false,
            sub_mapper_type: 0,
            trainer: Vec::new(),
        })
        .expect("test cartridge data should be valid"),
    )
    .expect("cartridge should construct")
}

#[cfg(test)]
fn nrom_test_data() -> CartridgeData {
    CartridgeData::new(CartridgeDataParts {
        format: RomFormat::INes,
        prog_rom: vec![0; 0x8000],
        char_rom: vec![0; 0x2000],
        pram_length: 0,
        save_pram_length: 0,
        vram_length: 0,
        save_vram_length: 0,
        mapper_type: 0,
        mirror_mode: MirrorMode::Horizontal,
        has_battery: false,
        sub_mapper_type: 0,
        trainer: Vec::new(),
    })
    .expect("test cartridge data should be valid")
}

#[cfg(test)]
fn mmc6_test_data() -> CartridgeData {
    CartridgeData::new(CartridgeDataParts {
        format: RomFormat::INes,
        prog_rom: vec![0; 0x8000],
        char_rom: vec![0; 0x2000],
        pram_length: 0,
        save_pram_length: 0,
        vram_length: 0,
        save_vram_length: 0,
        mapper_type: 4,
        mirror_mode: MirrorMode::Horizontal,
        has_battery: true,
        sub_mapper_type: 1,
        trainer: Vec::new(),
    })
    .expect("test cartridge data should be valid")
}

#[cfg(test)]
fn nrom_program_test_data(program: &[u8]) -> CartridgeData {
    let mut prog_rom = vec![0xEA; 0x8000];
    prog_rom[..program.len()].copy_from_slice(program);
    prog_rom[0x7FFC] = 0x00;
    prog_rom[0x7FFD] = 0x80;
    CartridgeData::new(CartridgeDataParts {
        format: RomFormat::INes,
        prog_rom,
        char_rom: vec![0; 0x2000],
        pram_length: 0,
        save_pram_length: 0,
        vram_length: 0,
        save_vram_length: 0,
        mapper_type: 0,
        mirror_mode: MirrorMode::Horizontal,
        has_battery: false,
        sub_mapper_type: 0,
        trainer: Vec::new(),
    })
    .expect("test cartridge data should be valid")
}

#[cfg(test)]
fn nrom_nop_program_test_data() -> CartridgeData {
    nrom_program_test_data(&[])
}

#[cfg(test)]
fn nrom_repeating_fast_path_program_test_data() -> CartridgeData {
    let mut prog_rom = Vec::with_capacity(0x8000);
    while prog_rom.len() < 0x8000 {
        prog_rom.push(0xA9);
        prog_rom.push(0x01);
    }
    prog_rom[0x7FFC] = 0x00;
    prog_rom[0x7FFD] = 0x80;
    CartridgeData::new(CartridgeDataParts {
        format: RomFormat::INes,
        prog_rom,
        char_rom: vec![0; 0x2000],
        pram_length: 0,
        save_pram_length: 0,
        vram_length: 0,
        save_vram_length: 0,
        mapper_type: 0,
        mirror_mode: MirrorMode::Horizontal,
        has_battery: false,
        sub_mapper_type: 0,
        trainer: Vec::new(),
    })
    .expect("test cartridge data should be valid")
}

#[cfg(test)]
fn mapper_program_test_data(
    mapper_type: u16,
    format: RomFormat,
    prog_rom_len: usize,
    char_rom_len: usize,
) -> CartridgeData {
    mapper_program_with_prefix_test_data(mapper_type, format, prog_rom_len, char_rom_len, &[])
}

#[cfg(test)]
fn mapper_program_with_prefix_test_data(
    mapper_type: u16,
    format: RomFormat,
    prog_rom_len: usize,
    char_rom_len: usize,
    program: &[u8],
) -> CartridgeData {
    let mut prog_rom = vec![0xEA; prog_rom_len];
    prog_rom[..program.len()].copy_from_slice(program);
    let vector = prog_rom_len - 4;
    prog_rom[vector] = 0x00;
    prog_rom[vector + 1] = 0x80;
    CartridgeData::new(CartridgeDataParts {
        format,
        prog_rom,
        char_rom: vec![0; char_rom_len],
        pram_length: 0,
        save_pram_length: 0,
        vram_length: 0,
        save_vram_length: 0,
        mapper_type,
        mirror_mode: MirrorMode::Horizontal,
        has_battery: false,
        sub_mapper_type: 0,
        trainer: Vec::new(),
    })
    .expect("test cartridge data should be valid")
}

#[cfg(test)]
mod scheduler_tests {
    use super::*;
    use crate::OpenBusReadResult;
    use crate::interrupt::{DmcDmaKind, Interrupt};

    #[derive(Default)]
    struct NullScreen;

    impl Screen for NullScreen {
        fn push(&mut self, _palette: u8) {}

        fn render(&mut self) {}
    }

    #[derive(Default)]
    struct NullController;

    impl Controller for NullController {
        fn read(&mut self, _address: usize) -> OpenBusReadResult {
            OpenBusReadResult::new(0, 0)
        }

        fn write(&mut self, _value: u8) {}
    }

    #[derive(Default)]
    struct CountingMixer {
        samples: usize,
    }

    impl MixerInput for CountingMixer {
        fn push(&mut self, _data: f32) {
            self.samples += 1;
        }
    }

    struct RecordingMixer {
        samples: Vec<f32>,
        sample_rate: u32,
    }

    impl Default for RecordingMixer {
        fn default() -> Self {
            Self {
                samples: Vec::new(),
                sample_rate: 48_000,
            }
        }
    }

    impl MixerInput for RecordingMixer {
        fn push(&mut self, data: f32) {
            self.samples.push(data);
        }

        fn sample_rate(&self) -> u32 {
            self.sample_rate
        }
    }

    fn assert_run_frame_matches_cycle_stepping(data: CartridgeData) {
        let mut scheduled = Core::new(data.clone()).expect("scheduled core should construct");
        let mut exact = Core::new(data).expect("exact core should construct");
        let mut scheduled_screen = NullScreen;
        let mut exact_screen = NullScreen;
        let mut scheduled_controller = NullController;
        let mut exact_controller = NullController;
        let mut scheduled_mixer = CountingMixer::default();
        let mut exact_mixer = CountingMixer::default();

        let scheduled_cycles = scheduled.run_frame(
            &mut scheduled_screen,
            &mut scheduled_controller,
            &mut scheduled_mixer,
        );

        let mut exact_cycles = 0;
        let exact_sample_rate = exact_mixer.sample_rate();
        loop {
            exact_cycles += 1;
            if exact.step_cycle(
                &mut exact_screen,
                &mut exact_controller,
                &mut exact_mixer,
                exact_sample_rate,
            ) {
                break;
            }
        }

        assert_eq!(scheduled_cycles, exact_cycles);
        assert_eq!(scheduled_mixer.samples, exact_mixer.samples);
        assert_eq!(
            scheduled
                .export_machine_state()
                .expect("scheduled state should export"),
            exact
                .export_machine_state()
                .expect("exact state should export")
        );
    }

    fn advance_to_fast_path_candidate(
        core: &mut Core,
        screen: &mut NullScreen,
        controller: &mut NullController,
        mixer: &mut CountingMixer,
        limit: usize,
    ) -> u64 {
        let sample_rate = mixer.sample_rate();
        for cycles in 0..limit {
            if core
                .cpu
                .instruction_fast_path_max_cycles(core.cartridge.as_ref())
                .is_some()
            {
                return cycles as u64;
            }
            core.step_cycle(screen, controller, mixer, sample_rate);
        }
        panic!("CPU should reach a schedulable instruction boundary");
    }

    fn step_instruction_event_for_test(
        core: &mut Core,
        screen: &mut NullScreen,
        controller: &mut NullController,
        mixer: &mut CountingMixer,
        sample_rate: u32,
    ) -> Option<(u64, bool)> {
        let apu_batch_mode = core.apu_batch_mode(sample_rate);
        core.step_instruction_event(screen, controller, mixer, sample_rate, apu_batch_mode)
    }

    #[test]
    fn instruction_scheduler_matches_cycle_stepping_for_safe_nrom_frame() {
        assert_run_frame_matches_cycle_stepping(nrom_nop_program_test_data());
    }

    #[test]
    fn instruction_scheduler_matches_cycle_stepping_for_safe_immediate_program() {
        assert_run_frame_matches_cycle_stepping(nrom_program_test_data(&[
            0xA9, 0x01, // LDA #$01
            0x69, 0x01, // ADC #$01
            0xAA, // TAX
            0xE8, // INX
            0x38, // SEC
        ]));
    }

    #[test]
    fn instruction_scheduler_matches_cycle_stepping_for_fast_cpu_operations() {
        assert_run_frame_matches_cycle_stepping(nrom_program_test_data(&[
            0xA9, 0x55, // LDA #$55
            0x69, 0x0A, // ADC #$0A
            0x29, 0x0F, // AND #$0F
            0x49, 0xF0, // EOR #$F0
            0x09, 0x01, // ORA #$01
            0xC9, 0x20, // CMP #$20
            0xA2, 0x03, // LDX #$03
            0xE0, 0x04, // CPX #$04
            0xA0, 0x05, // LDY #$05
            0xC0, 0x05, // CPY #$05
            0xAA, // TAX
            0xA8, // TAY
            0x8A, // TXA
            0x98, // TYA
            0xBA, // TSX
            0x9A, // TXS
            0xE8, // INX
            0xC8, // INY
            0xCA, // DEX
            0x88, // DEY
            0x0A, // ASL A
            0x4A, // LSR A
            0x2A, // ROL A
            0x6A, // ROR A
            0x18, // CLC
            0x38, // SEC
            0xD8, // CLD
            0xF8, // SED
            0x58, // CLI
            0x78, // SEI
            0xB8, // CLV
            0xE9, 0x01, // SBC #$01
            0xEA, // NOP
        ]));
    }

    #[test]
    fn instruction_scheduler_matches_cycle_stepping_for_memory_fast_path_operations() {
        assert_run_frame_matches_cycle_stepping(nrom_program_test_data(&[
            0xA9, 0x3F, // LDA #$3F
            0x85, 0x10, // STA $10
            0xA2, 0x02, // LDX #$02
            0xA0, 0x03, // LDY #$03
            0xA5, 0x10, // LDA $10
            0x65, 0x0E, // ADC $0E
            0x95, 0x20, // STA $20,X
            0xB5, 0x1E, // LDA $1E,X
            0x2C, 0x22, 0x00, // BIT $0022
            0xCD, 0x22, 0x00, // CMP $0022
            0xBD, 0x20, 0x00, // LDA $0020,X
            0xB9, 0x1F, 0x00, // LDA $001F,Y
            0x8D, 0x30, 0x00, // STA $0030
        ]));
    }

    #[test]
    fn instruction_scheduler_matches_cycle_stepping_for_branch_fast_path() {
        assert_run_frame_matches_cycle_stepping(nrom_program_test_data(&[
            0xA2, 0x03, // LDX #$03
            0xCA, // DEX
            0xD0, 0xFD, // BNE $8002
            0xA9, 0x01, // LDA #$01
        ]));
    }

    #[test]
    fn instruction_scheduler_preserves_sxrom_write_spacing_across_fast_path() {
        assert_run_frame_matches_cycle_stepping(mapper_program_with_prefix_test_data(
            1,
            RomFormat::INes,
            0x8000,
            0x2000,
            &[
                0xA9, 0x80, // LDA #$80
                0x8D, 0x00, 0x80, // STA $8000
                0xA9, 0x00, // LDA #$00, eligible for the instruction fast path
                0x8D, 0x00, 0x80, // STA $8000
            ],
        ));
    }

    #[test]
    fn instruction_scheduler_matches_cycle_stepping_for_mmc3_idle_program() {
        assert_run_frame_matches_cycle_stepping(mapper_program_with_prefix_test_data(
            4,
            RomFormat::INes,
            0x8000,
            0x2000,
            &[0xA9, 0x01, 0x69, 0x01, 0xEA],
        ));
    }

    #[test]
    fn instruction_scheduler_matches_cycle_stepping_for_mmc5_idle_program() {
        assert_run_frame_matches_cycle_stepping(mapper_program_with_prefix_test_data(
            5,
            RomFormat::Nes20,
            0x20000,
            0x40000,
            &[0xA9, 0x01, 0x69, 0x01, 0xEA],
        ));
    }

    #[test]
    fn instruction_scheduler_matches_cycle_stepping_for_mmc5_expansion_audio_samples() {
        let mut program = vec![
            0xA9, 0x40, 0x8D, 0x11, 0x50, // LDA #$40; STA $5011
            0xA9, 0x03, 0x8D, 0x15, 0x50, // LDA #$03; STA $5015
            0xA9, 0x3F, 0x8D, 0x00, 0x50, // LDA #$3F; STA $5000
            0xA9, 0x08, 0x8D, 0x02, 0x50, // LDA #$08; STA $5002
            0xA9, 0xF8, 0x8D, 0x03, 0x50, // LDA #$F8; STA $5003
        ];
        program.extend([0xEA; 128]);
        program.extend_from_slice(&[
            0xA9, 0x20, 0x8D, 0x11, 0x50, // LDA #$20; STA $5011
        ]);
        program.extend([0xEA; 128]);
        program.extend_from_slice(&[
            0xA9, 0x70, 0x8D, 0x11, 0x50, // LDA #$70; STA $5011
        ]);
        program.extend([0xEA; 128]);
        program.extend_from_slice(&[0xA9, 0x01, 0x69, 0x01, 0xEA]);

        let mut prog_rom = vec![0xEA; 0x20000];
        let program_start = 0x1E000;
        prog_rom[program_start..program_start + program.len()].copy_from_slice(&program);
        let vector = prog_rom.len() - 4;
        prog_rom[vector] = 0x00;
        prog_rom[vector + 1] = 0xE0;
        let data = CartridgeData::new(CartridgeDataParts {
            format: RomFormat::Nes20,
            prog_rom,
            char_rom: vec![0; 0x40000],
            pram_length: 0,
            save_pram_length: 0,
            vram_length: 0,
            save_vram_length: 0,
            mapper_type: 5,
            mirror_mode: MirrorMode::Horizontal,
            has_battery: false,
            sub_mapper_type: 0,
            trainer: Vec::new(),
        })
        .expect("test cartridge data should be valid");
        let mut scheduled = Core::new(data.clone()).expect("scheduled core should construct");
        let mut exact = Core::new(data).expect("exact core should construct");
        let mut scheduled_screen = NullScreen;
        let mut exact_screen = NullScreen;
        let mut scheduled_controller = NullController;
        let mut exact_controller = NullController;
        let mut scheduled_mixer = RecordingMixer::default();
        let mut exact_mixer = RecordingMixer::default();

        let mut scheduled_cycles = 0;
        for _ in 0..3 {
            scheduled_cycles += scheduled.run_frame(
                &mut scheduled_screen,
                &mut scheduled_controller,
                &mut scheduled_mixer,
            );
        }
        let exact_sample_rate = exact_mixer.sample_rate();
        let mut exact_cycles = 0;
        let mut exact_frames = 0;
        while exact_frames < 3 {
            exact_cycles += 1;
            if exact.step_cycle(
                &mut exact_screen,
                &mut exact_controller,
                &mut exact_mixer,
                exact_sample_rate,
            ) {
                exact_frames += 1;
            }
        }

        assert_eq!(scheduled_cycles, exact_cycles);
        assert!(scheduled_mixer.samples.iter().any(|sample| *sample != 0.0));
        assert!(
            scheduled_mixer
                .samples
                .windows(2)
                .any(|window| window[0] != window[1])
        );
        assert_eq!(scheduled_mixer.samples, exact_mixer.samples);
        assert_eq!(
            scheduled
                .export_machine_state()
                .expect("scheduled state should export"),
            exact
                .export_machine_state()
                .expect("exact state should export")
        );
    }

    #[test]
    fn instruction_scheduler_fast_path_runs_at_safe_instruction_boundary() {
        let mut core =
            Core::new(nrom_program_test_data(&[0xA9, 0x01])).expect("core should construct");
        let mut screen = NullScreen;
        let mut controller = NullController;
        let mut mixer = CountingMixer::default();
        let sample_rate = mixer.sample_rate();

        for _ in 0..16 {
            if core
                .cpu
                .instruction_fast_path_max_cycles(core.cartridge.as_ref())
                .is_some()
            {
                let advanced = step_instruction_event_for_test(
                    &mut core,
                    &mut screen,
                    &mut controller,
                    &mut mixer,
                    sample_rate,
                );
                assert!(advanced.is_some());
                return;
            }
            core.step_cycle(&mut screen, &mut controller, &mut mixer, sample_rate);
        }

        panic!("CPU should reach a schedulable instruction boundary after reset");
    }

    #[test]
    fn instruction_scheduler_falls_back_when_dma_is_pending() {
        let mut core =
            Core::new(nrom_repeating_fast_path_program_test_data()).expect("core should construct");
        let mut screen = NullScreen;
        let mut controller = NullController;
        let mut mixer = CountingMixer::default();
        let sample_rate = mixer.sample_rate();

        advance_to_fast_path_candidate(&mut core, &mut screen, &mut controller, &mut mixer, 32);
        core.cpu.interrupt_mut().dmc_dma_request = Some(DmcDmaKind::Load);

        assert!(
            step_instruction_event_for_test(
                &mut core,
                &mut screen,
                &mut controller,
                &mut mixer,
                sample_rate
            )
            .is_none()
        );
    }

    #[test]
    fn instruction_scheduler_falls_back_when_indexed_read_crosses_into_ppu_registers() {
        let mut core = Core::new(nrom_program_test_data(&[
            0xA2, 0x01, // LDX #$01
            0xBD, 0xFF, 0x1F, // LDA $1FFF,X
        ]))
        .expect("core should construct");
        let mut screen = NullScreen;
        let mut controller = NullController;
        let mut mixer = CountingMixer::default();
        let sample_rate = mixer.sample_rate();

        advance_to_fast_path_candidate(&mut core, &mut screen, &mut controller, &mut mixer, 16);
        assert!(
            step_instruction_event_for_test(
                &mut core,
                &mut screen,
                &mut controller,
                &mut mixer,
                sample_rate
            )
            .is_some()
        );
        assert!(
            core.cpu
                .instruction_fast_path_max_cycles(core.cartridge.as_ref())
                .is_none()
        );
    }

    #[test]
    fn instruction_scheduler_falls_back_when_store_targets_ppu_registers() {
        let mut core = Core::new(nrom_program_test_data(&[
            0xA9, 0x01, // LDA #$01
            0x8D, 0x00, 0x20, // STA $2000
        ]))
        .expect("core should construct");
        let mut screen = NullScreen;
        let mut controller = NullController;
        let mut mixer = CountingMixer::default();
        let sample_rate = mixer.sample_rate();

        advance_to_fast_path_candidate(&mut core, &mut screen, &mut controller, &mut mixer, 16);
        assert!(
            step_instruction_event_for_test(
                &mut core,
                &mut screen,
                &mut controller,
                &mut mixer,
                sample_rate
            )
            .is_some()
        );
        assert!(
            core.cpu
                .instruction_fast_path_max_cycles(core.cartridge.as_ref())
                .is_none()
        );
    }

    #[test]
    fn instruction_scheduler_falls_back_before_ppu_event() {
        let mut core =
            Core::new(nrom_repeating_fast_path_program_test_data()).expect("core should construct");
        let mut screen = NullScreen;
        let mut controller = NullController;
        let mut mixer = CountingMixer::default();
        let sample_rate = mixer.sample_rate();

        for _ in 0..30_000 {
            if let Some(max_cpu_cycles) = core
                .cpu
                .instruction_fast_path_max_cycles(core.cartridge.as_ref())
            {
                let max_ppu_cycles = max_cpu_cycles * 3;
                if core.ppu.cycles_until_next_scheduler_event(max_ppu_cycles) <= max_ppu_cycles {
                    assert!(
                        step_instruction_event_for_test(
                            &mut core,
                            &mut screen,
                            &mut controller,
                            &mut mixer,
                            sample_rate
                        )
                        .is_none()
                    );
                    return;
                }
            }
            core.step_cycle(&mut screen, &mut controller, &mut mixer, sample_rate);
        }

        panic!("PPU scheduler event should become close enough to force fallback");
    }

    #[test]
    fn instruction_scheduler_falls_back_before_apu_irq_event() {
        let mut core =
            Core::new(nrom_repeating_fast_path_program_test_data()).expect("core should construct");
        let mut screen = NullScreen;
        let mut controller = NullController;
        let mut mixer = CountingMixer::default();
        let sample_rate = mixer.sample_rate();

        for _ in 0..30_000 {
            if let Some(max_cpu_cycles) = core
                .cpu
                .instruction_fast_path_max_cycles(core.cartridge.as_ref())
                && core
                    .apu
                    .cycles_until_next_scheduler_event(core.cpu.interrupt_ref(), max_cpu_cycles)
                    <= max_cpu_cycles
            {
                assert!(
                    step_instruction_event_for_test(
                        &mut core,
                        &mut screen,
                        &mut controller,
                        &mut mixer,
                        sample_rate
                    )
                    .is_none()
                );
                return;
            }
            core.step_cycle(&mut screen, &mut controller, &mut mixer, sample_rate);
        }

        panic!("APU IRQ event should become close enough to force fallback");
    }

    #[test]
    fn instruction_scheduler_falls_back_before_mapper_cpu_event() {
        let mut core = Core::new(mapper_program_with_prefix_test_data(
            69,
            RomFormat::INes,
            0x8000,
            0x2000,
            &[0xA9, 0x01],
        ))
        .expect("core should construct");
        let mut screen = NullScreen;
        let mut controller = NullController;
        let mut mixer = CountingMixer::default();
        let sample_rate = mixer.sample_rate();

        advance_to_fast_path_candidate(&mut core, &mut screen, &mut controller, &mut mixer, 32);

        let mut interrupt = Interrupt::new();
        core.cartridge.write(0x8000, 0x0E, &mut interrupt);
        core.cartridge.write(0xA000, 0x01, &mut interrupt);
        core.cartridge.write(0x8000, 0x0F, &mut interrupt);
        core.cartridge.write(0xA000, 0x00, &mut interrupt);
        core.cartridge.write(0x8000, 0x0D, &mut interrupt);
        core.cartridge.write(0xA000, 0x81, &mut interrupt);

        assert!(core.cartridge.cycles_until_next_cpu_event() <= 2);
        assert!(
            step_instruction_event_for_test(
                &mut core,
                &mut screen,
                &mut controller,
                &mut mixer,
                sample_rate
            )
            .is_none()
        );
    }

    #[test]
    fn instruction_scheduler_stays_disabled_for_unaudited_mapper() {
        let mut core = Core::new(mapper_program_test_data(2, RomFormat::INes, 0x8000, 0x2000))
            .expect("core should construct");
        let mut screen = NullScreen;
        let mut controller = NullController;
        let mut mixer = CountingMixer::default();
        let sample_rate = mixer.sample_rate();

        assert!(
            step_instruction_event_for_test(
                &mut core,
                &mut screen,
                &mut controller,
                &mut mixer,
                sample_rate
            )
            .is_none()
        );
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;

    const MACHINE_STATE_FIXTURE_HEX: &str = "87ae736368656d615f76657273696f6e02ac726f6d5f6964656e746974798fa6666f726d6174a4494e6573ab6d61707065725f7479706500af7375625f6d61707065725f7479706500ab6d6972726f725f6d6f6465aa486f72697a6f6e74616cab6861735f62617474657279c2ab747261696e65725f6c656e00ab7072675f726f6d5f6c656ecd8000ab6368725f726f6d5f6c656ecd2000ab7072675f72616d5f6c656e00b0736176655f7072675f72616d5f6c656e00ab6368725f72616d5f6c656e00b0736176655f6368725f72616d5f6c656e00ad7072675f726f6d5f6372633634cf9b3690a319de92d5ad6368725f726f6d5f6372633634cfc7e021a7a1a6dd3aad747261696e65725f637263363400a76f7074696f6e7381b06d6d63335f6972715f76617269616e74c0a363707587a66d656d6f727982a47772616ddc08000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a76f70656e62757381a46461746100a872656769737465728da2706300a2737000a16100a17800a17900a163c2a17ac2a169c2a164c2a162c2a172c3a176c2a16ec2ad696e7465726e616c5f7374617488a66f70636f646500a76164647265737300a47374657000a874656d706164647200a46461746100a763726f73736564c2a9696e74657272757074c2a57374617465a55265736574a9696e7465727275707489a36e6d69c2a9657865637574696e67c2a86465746563746564c2ab72756e6e696e675f646d61c2a86972715f6d61736b00a86972715f666c616700a76f616d5f646d61c0af646d635f646d615f72657175657374c0a57772697465c2a66379636c657300a76f616d5f646d6182a57374617465a44e6f6e65a576616c756583a66f666673657400a5636f756e7400a576616c756500a7646d635f646d61c0a3707075de0024a47672616ddc08000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a770616c65747465dc0020090100010002020d081008240000042c0901340300040014083a000200202c08a5737461746589a7636f6e74726f6c00a46d61736b00ab6f616d5f6164647265737300a97672616d5f6164647200ae74656d705f7672616d5f6164647200a8785f7363726f6c6c00ac77726974655f746f67676c65c2ae686967685f6269745f736869667400ad6c6f775f6269745f736869667400a56379636c6500a97363616e5f6c696e6500a66672616d657300a86275735f7469636b00ad62756666657265645f6461746100ab7072696d6172795f6f616ddc010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000ad7365636f6e646172795f6f616ddc00200000000000000000000000000000000000000000000000000000000000000000b57365636f6e646172795f6f616d5f6164647265737300a7636f6e74726f6c87aa6e616d655f7461626c6500a9696e6372656d656e74c2ac7370726974655f7461626c65c2b06261636b67726f756e645f7461626c65c2ab7370726974655f73697a65c2ac6d61737465725f736c617665c2aa6e6d695f6f7574707574c2a46d61736b88a9677261797363616c65c2b473686f775f6c6566745f6261636b67726f756e64c2b173686f775f6c6566745f73707269746573c2af73686f775f6261636b67726f756e64c2ac73686f775f73707269746573c2a87265645f74696e74c2aa677265656e5f74696e74c2a9626c75655f74696e74c2a673746174757383af7370726974655f7a65726f5f686974c2af7370726974655f6f766572666c6f77c2ac6e6d695f6f63637572726564c2ac63757272656e745f74696c6584a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200ad70726576696f75735f74696c6584a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200a96e6578745f74696c6584a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200a773707269746573dc004087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e0087a86c6f775f6279746500a9686967685f6279746500ae70616c657474655f6f666673657400a974696c655f6164647200b1686f72697a6f6e74616c5f6d6972726f72c2a87072696f72697479c2a8706f736974696f6e00ac7370726974655f696e64657800ac7370726974655f636f756e7400b072656e6465725f657865637574696e67c2b5706f73745f72656e6465725f657865637574696e67c2af6f616d5f726561645f62756666657200af7672616d5f726561645f64656c617900b67672616d5f616464725f7570646174655f64656c617900ad6e65775f7672616d5f6164647200b56861735f66697273745f7370726974655f6e657874c2b06861735f66697273745f737072697465c2aa6861735f737072697465c2b57370726974655f6f766572666c6f775f64656c617900ae7370726974655f72656164696e67c2b06f616d5f616464726573735f6869676800af6f616d5f616464726573735f6c6f7700ac6f70656e6275735f7672616d81a46461746100aa6f70656e6275735f696f82a46461746100a56465636179980000000000000000af6861735f6e6578745f737072697465c2a361707587a670756c7365318eb069735f66697273745f6368616e6e656cc3a9647574795f6d6f646500aa647574795f76616c756504ac73776565705f72656c6f6164c2ad73776565705f656e61626c6564c2ac73776565705f6e6567617465c2ab73776565705f736869667400ac73776565705f706572696f6400ab73776565705f76616c756500b373776565705f7461726765745f706572696f6400a6706572696f6400a8656e76656c6f706585a7656e61626c6564c2a6766f6c756d6500a57374617274c2a576616c756500a6706572696f6400ae6c656e6774685f636f756e74657286a96e6578745f68616c74c2a468616c74c2a7656e61626c6564c2aa6e6578745f76616c756500a576616c756500aa707265765f76616c756500a574696d657282a576616c756500a6706572696f6400a670756c7365328eb069735f66697273745f6368616e6e656cc2a9647574795f6d6f646500aa647574795f76616c756504ac73776565705f72656c6f6164c2ad73776565705f656e61626c6564c2ac73776565705f6e6567617465c2ab73776565705f736869667400ac73776565705f706572696f6400ab73776565705f76616c756500b373776565705f7461726765745f706572696f6400a6706572696f6400a8656e76656c6f706585a7656e61626c6564c2a6766f6c756d6500a57374617274c2a576616c756500a6706572696f6400ae6c656e6774685f636f756e74657286a96e6578745f68616c74c2a468616c74c2a7656e61626c6564c2aa6e6578745f76616c756500a576616c756500aa707265765f76616c756500a574696d657282a576616c756500a6706572696f6400a8747269616e676c6588aa647574795f76616c756500ae636f756e7465725f706572696f6400ad636f756e7465725f76616c756500ae636f756e7465725f72656c6f6164c2af636f756e7465725f636f6e74726f6cc2ac6f75747075745f76616c756500ae6c656e6774685f636f756e74657286a96e6578745f68616c74c2a468616c74c2a7656e61626c6564c2aa6e6578745f76616c756500a576616c756500aa707265765f76616c756500a574696d657282a576616c756500a6706572696f6400a56e6f69736585a46d6f6465c2ae73686966745f7265676973746572cd0800a8656e76656c6f706585a7656e61626c6564c2a6766f6c756d6500a57374617274c2a576616c756500a6706572696f6400ae6c656e6774685f636f756e74657286a96e6578745f68616c74c2a468616c74c2a7656e61626c6564c2aa6e6578745f76616c756500a576616c756500aa707265765f76616c756500a574696d657282a576616c756500a6706572696f6400a3646d638da576616c756500ae73616d706c655f6164647265737300ad73616d706c655f6c656e67746800ac6c656e6774685f76616c756500af63757272656e745f6164647265737300ae73686966745f726567697374657200a96269745f636f756e7405ab726561645f62756666657200a7656e61626c6564c2ab6e6565645f627566666572c3a769735f6c6f6f70c2a3697271c2a574696d657282a576616c756500a6706572696f6400b273616d706c655f616363756d756c61746f7200ad6672616d655f636f756e74657287a6706572696f64c2a3697271c3ad77726974655f636f756e74657200a5626c6f636b00a96e65775f76616c756500ab636c6f636b5f6379636c6504a56379636c6501a963617274726964676583ac6d61707065725f737461746588b270726f6772616d5f706167655f7461626c65dc0100000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7fc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0b46368617261637465725f706167655f7461626c65dc0100000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1fc0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0af7372616d5f706167655f7461626c65dc0100c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0a47372616d90a47672616d90ab6d6972726f725f6d6f6465aa486f72697a6f6e74616cab6861735f62617474657279c2b66368617261637465725f6d617070696e675f6d6f6465a3526f6daa65787472615f6b696e64a0aa65787472615f626f6479c400";
    const MAPPER_SAVE_FIXTURE_HEX: &str = "85ae736368656d615f76657273696f6e02ac726f6d5f6964656e746974798fa6666f726d6174a4494e6573ab6d61707065725f7479706504af7375625f6d61707065725f7479706501ab6d6972726f725f6d6f6465aa486f72697a6f6e74616cab6861735f62617474657279c3ab747261696e65725f6c656e00ab7072675f726f6d5f6c656ecd8000ab6368725f726f6d5f6c656ecd2000ab7072675f72616d5f6c656e00b0736176655f7072675f72616d5f6c656e00ab6368725f72616d5f6c656e00b0736176655f6368725f72616d5f6c656e00ad7072675f726f6d5f6372633634cf9b3690a319de92d5ad6368725f726f6d5f6372633634cfc7e021a7a1a6dd3aad747261696e65725f637263363400a76f7074696f6e7381b06d6d63335f6972715f76617269616e74a57368617270a77072675f72616dc5040000070e151c232a31383f464d545b626970777e858c939aa1a8afb6bdc4cbd2d9e0e7eef5fc030a11181f262d343b424950575e656c737a81888f969da4abb2b9c0c7ced5dce3eaf1f8ff060d141b222930373e454c535a61686f767d848b9299a0a7aeb5bcc3cad1d8dfe6edf4fb020910171e252c333a41484f565d646b727980878e959ca3aab1b8bfc6cdd4dbe2e9f0f7fe050c131a21282f363d444b525960676e757c838a91989fa6adb4bbc2c9d0d7dee5ecf3fa01080f161d242b323940474e555c636a71787f868d949ba2a9b0b7bec5ccd3dae1e8eff6fd040b121920272e353c434a51585f666d747b828990979ea5acb3bac1c8cfd6dde4ebf2f900070e151c232a31383f464d545b626970777e858c939aa1a8afb6bdc4cbd2d9e0e7eef5fc030a11181f262d343b424950575e656c737a81888f969da4abb2b9c0c7ced5dce3eaf1f8ff060d141b222930373e454c535a61686f767d848b9299a0a7aeb5bcc3cad1d8dfe6edf4fb020910171e252c333a41484f565d646b727980878e959ca3aab1b8bfc6cdd4dbe2e9f0f7fe050c131a21282f363d444b525960676e757c838a91989fa6adb4bbc2c9d0d7dee5ecf3fa01080f161d242b323940474e555c636a71787f868d949ba2a9b0b7bec5ccd3dae1e8eff6fd040b121920272e353c434a51585f666d747b828990979ea5acb3bac1c8cfd6dde4ebf2f900070e151c232a31383f464d545b626970777e858c939aa1a8afb6bdc4cbd2d9e0e7eef5fc030a11181f262d343b424950575e656c737a81888f969da4abb2b9c0c7ced5dce3eaf1f8ff060d141b222930373e454c535a61686f767d848b9299a0a7aeb5bcc3cad1d8dfe6edf4fb020910171e252c333a41484f565d646b727980878e959ca3aab1b8bfc6cdd4dbe2e9f0f7fe050c131a21282f363d444b525960676e757c838a91989fa6adb4bbc2c9d0d7dee5ecf3fa01080f161d242b323940474e555c636a71787f868d949ba2a9b0b7bec5ccd3dae1e8eff6fd040b121920272e353c434a51585f666d747b828990979ea5acb3bac1c8cfd6dde4ebf2f900070e151c232a31383f464d545b626970777e858c939aa1a8afb6bdc4cbd2d9e0e7eef5fc030a11181f262d343b424950575e656c737a81888f969da4abb2b9c0c7ced5dce3eaf1f8ff060d141b222930373e454c535a61686f767d848b9299a0a7aeb5bcc3cad1d8dfe6edf4fb020910171e252c333a41484f565d646b727980878e959ca3aab1b8bfc6cdd4dbe2e9f0f7fe050c131a21282f363d444b525960676e757c838a91989fa6adb4bbc2c9d0d7dee5ecf3fa01080f161d242b323940474e555c636a71787f868d949ba2a9b0b7bec5ccd3dae1e8eff6fd040b121920272e353c434a51585f666d747b828990979ea5acb3bac1c8cfd6dde4ebf2f9a76368725f72616dc400";

    fn fixture_bytes(hex: &str) -> Vec<u8> {
        assert!(
            !hex.trim().is_empty(),
            "fixture hex must be populated before running persistence tests"
        );
        let hex = hex.trim();
        assert_eq!(hex.len() % 2, 0, "fixture hex length must be even");
        hex.as_bytes()
            .chunks_exact(2)
            .map(|chunk| {
                let text = std::str::from_utf8(chunk).expect("fixture hex should be valid utf-8");
                u8::from_str_radix(text, 16).expect("fixture hex should decode")
            })
            .collect()
    }

    fn decode_machine_state_fixture() -> (Vec<u8>, MachineStatePayload) {
        let bytes = fixture_bytes(MACHINE_STATE_FIXTURE_HEX);
        let payload = decode_payload::<MachineStatePayload>(&bytes)
            .expect("machine state fixture should decode");
        (bytes, payload)
    }

    fn decode_mapper_save_fixture() -> (Vec<u8>, MapperSavePayload) {
        let bytes = fixture_bytes(MAPPER_SAVE_FIXTURE_HEX);
        let payload =
            decode_payload::<MapperSavePayload>(&bytes).expect("mapper save fixture should decode");
        (bytes, payload)
    }

    fn export_machine_state_payload(core: &Core) -> MachineStatePayload {
        let bytes = core
            .export_machine_state()
            .expect("machine state should export");
        decode_payload(&bytes).expect("machine state should decode")
    }

    fn export_mapper_save_payload(core: &Core) -> MapperSavePayload {
        let bytes = core
            .export_mapper_save()
            .expect("mapper save should export")
            .expect("mapper save should exist");
        decode_payload(&bytes).expect("mapper save should decode")
    }

    #[test]
    fn machine_state_fixture_round_trip_preserves_core_owned_fields() {
        let (bytes, fixture) = decode_machine_state_fixture();
        assert_eq!(fixture.schema_version, PERSISTENCE_SCHEMA_VERSION);
        assert_eq!(
            fixture.rom_identity,
            Core::new(nrom_test_data()).unwrap().rom_identity()
        );
        assert_eq!(fixture.options, CoreOptions::default());

        let mut target = Core::new(nrom_test_data()).expect("target core should construct");
        target
            .import_machine_state(&bytes)
            .expect("fixture machine state should import");
        let exported = export_machine_state_payload(&target);

        assert_eq!(exported.schema_version, fixture.schema_version);
        assert_eq!(exported.rom_identity, fixture.rom_identity);
        assert_eq!(exported.options, fixture.options);
        assert_eq!(
            encode_payload(&exported).expect("exported payload should encode"),
            encode_payload(&fixture).expect("fixture payload should encode")
        );
    }

    #[test]
    fn machine_state_rejects_schema_mismatch() {
        let (_, mut payload) = decode_machine_state_fixture();
        payload.schema_version += 1;
        let bytes = encode_payload(&payload).expect("payload should encode");
        let mut target = Core::new(nrom_test_data()).expect("target core should construct");

        let error = target
            .import_machine_state(&bytes)
            .expect_err("schema mismatch should reject");
        assert!(
            error
                .to_string()
                .contains("unsupported persistence schema version")
        );
    }

    #[test]
    fn mapper_save_fixture_round_trip_preserves_battery_backed_ram() {
        let (bytes, fixture) = decode_mapper_save_fixture();
        assert_eq!(fixture.schema_version, PERSISTENCE_SCHEMA_VERSION);
        assert_eq!(
            fixture.rom_identity,
            Core::new(mmc6_test_data()).unwrap().rom_identity()
        );
        assert_eq!(fixture.prg_ram.len(), 0x0400);
        assert!(fixture.chr_ram.is_empty());

        let mut target = Core::new(mmc6_test_data()).expect("target core should construct");
        target
            .import_mapper_save(&bytes)
            .expect("fixture mapper save should import");
        let exported = export_mapper_save_payload(&target);

        assert_eq!(exported.schema_version, fixture.schema_version);
        assert_eq!(exported.rom_identity, fixture.rom_identity);
        assert_eq!(
            fixture.options.mmc3_irq_variant,
            Some(Mmc3IrqVariant::Sharp)
        );
        assert_eq!(exported.options, target.options());
        assert_eq!(exported.prg_ram, fixture.prg_ram);
        assert_eq!(exported.chr_ram, fixture.chr_ram);
    }

    #[test]
    fn mapper_save_rejects_schema_mismatch() {
        let (_, mut payload) = decode_mapper_save_fixture();
        payload.schema_version += 1;
        let bytes = encode_payload(&payload).expect("payload should encode");
        let mut target = Core::new(mmc6_test_data()).expect("target core should construct");

        let error = target
            .import_mapper_save(&bytes)
            .expect_err("schema mismatch should reject");
        assert!(
            error
                .to_string()
                .contains("unsupported persistence schema version")
        );
    }

    #[test]
    fn mapper_save_rejects_rom_identity_mismatch() {
        let (bytes, _) = decode_mapper_save_fixture();
        let mut different_rom = mmc6_test_data();
        different_rom.write_prog_rom(0, 1);
        let mut target = Core::new(different_rom).expect("target core should construct");

        let error = target
            .import_mapper_save(&bytes)
            .expect_err("different ROM contents should reject");
        assert!(error.to_string().contains("ROM identity mismatch"));
    }

    #[test]
    fn mapper_save_failed_import_does_not_mutate_existing_state() {
        let (_, mut payload) = decode_mapper_save_fixture();
        payload.prg_ram.pop();
        let bytes = encode_payload(&payload).expect("payload should encode");

        let mut target = Core::new(mmc6_test_data()).expect("target core should construct");
        let before = export_mapper_save_payload(&target);

        target
            .import_mapper_save(&bytes)
            .expect_err("invalid mapper save should reject");

        let after = export_mapper_save_payload(&target);
        assert_eq!(after.prg_ram, before.prg_ram);
        assert_eq!(after.chr_ram, before.chr_ram);
    }

    #[test]
    fn machine_state_rejects_runtime_option_mismatch() {
        let source = Core::new_with_options(
            nrom_test_data(),
            CoreOptions {
                mmc3_irq_variant: Some(Mmc3IrqVariant::Sharp),
            },
        )
        .expect("source core should construct");
        let payload = source
            .export_machine_state()
            .expect("machine state should export");

        let mut target = Core::new_with_options(
            nrom_test_data(),
            CoreOptions {
                mmc3_irq_variant: Some(Mmc3IrqVariant::Nec),
            },
        )
        .expect("target core should construct");

        let error = target
            .import_machine_state(&payload)
            .expect_err("mismatched options should reject");
        assert!(error.to_string().contains("runtime options mismatch"));
    }

    #[test]
    fn machine_state_rejects_rom_identity_mismatch() {
        let source = Core::new(nrom_test_data()).expect("source core should construct");
        let payload = source
            .export_machine_state()
            .expect("machine state should export");
        let mut different_rom = nrom_test_data();
        different_rom.write_prog_rom(0, 1);
        let mut target = Core::new(different_rom).expect("target core should construct");

        let error = target
            .import_machine_state(&payload)
            .expect_err("different ROM contents should reject");
        assert!(error.to_string().contains("ROM identity mismatch"));
    }

    #[test]
    fn machine_state_failed_import_does_not_mutate_existing_state() {
        let source = Core::new(nrom_test_data()).expect("source core should construct");
        let payload = source
            .export_machine_state()
            .expect("machine state should export");
        let mut tampered = decode_payload::<MachineStatePayload>(&payload)
            .expect("machine state payload should decode");
        tampered.cartridge.mapper_state.sram.push(1);
        let tampered_payload = encode_payload(&tampered).expect("tampered payload should encode");

        let mut target = Core::new(nrom_test_data()).expect("target core should construct");
        let before = target
            .export_machine_state()
            .expect("target state should export");

        target
            .import_machine_state(&tampered_payload)
            .expect_err("tampered payload should reject");

        let after = target
            .export_machine_state()
            .expect("target state should still export");
        assert_eq!(before, after);
    }

    #[test]
    fn machine_state_rejects_out_of_bounds_mapper_page_table() {
        let source = Core::new(nrom_test_data()).expect("source core should construct");
        let payload = source
            .export_machine_state()
            .expect("machine state should export");
        let mut tampered = decode_payload::<MachineStatePayload>(&payload)
            .expect("machine state payload should decode");
        tampered.cartridge.mapper_state.program_page_table[0] = Some(usize::MAX);
        let tampered_payload = encode_payload(&tampered).expect("tampered payload should encode");

        let mut target = Core::new(nrom_test_data()).expect("target core should construct");
        let before = target
            .export_machine_state()
            .expect("target state should export");

        let error = target
            .import_machine_state(&tampered_payload)
            .expect_err("out-of-bounds mapper page should reject");
        assert!(error.to_string().contains("out of bounds"));

        let after = target
            .export_machine_state()
            .expect("target state should still export");
        assert_eq!(before, after);
    }

    #[test]
    fn machine_state_rejects_invalid_cpu_opcode() {
        let source = Core::new(nrom_test_data()).expect("source core should construct");
        let payload = source
            .export_machine_state()
            .expect("machine state should export");
        let mut tampered = decode_payload::<MachineStatePayload>(&payload)
            .expect("machine state payload should decode");
        tampered.cpu.set_internal_opcode_for_test(0x100);
        let tampered_payload = encode_payload(&tampered).expect("tampered payload should encode");

        let mut target = Core::new(nrom_test_data()).expect("target core should construct");
        let before = target
            .export_machine_state()
            .expect("target state should export");

        let error = target
            .import_machine_state(&tampered_payload)
            .expect_err("invalid CPU opcode should reject");
        assert!(error.to_string().contains("CPU opcode overflow"));

        let after = target
            .export_machine_state()
            .expect("target state should still export");
        assert_eq!(before, after);
    }

    #[test]
    fn machine_state_rejects_invalid_ppu_sprite_index() {
        let source = Core::new(nrom_test_data()).expect("source core should construct");
        let payload = source
            .export_machine_state()
            .expect("machine state should export");
        let mut tampered = decode_payload::<MachineStatePayload>(&payload)
            .expect("machine state payload should decode");
        tampered
            .ppu
            .set_sprite_fetch_state_for_test(9, 0, 316, true);
        let tampered_payload = encode_payload(&tampered).expect("tampered payload should encode");

        let mut target = Core::new(nrom_test_data()).expect("target core should construct");
        let before = target
            .export_machine_state()
            .expect("target state should export");

        let error = target
            .import_machine_state(&tampered_payload)
            .expect_err("invalid PPU sprite index should reject");
        assert!(error.to_string().contains("PPU sprite index overflow"));

        let after = target
            .export_machine_state()
            .expect("target state should still export");
        assert_eq!(before, after);
    }

    #[test]
    fn machine_state_rejects_premature_terminal_ppu_sprite_index() {
        let source = Core::new(nrom_test_data()).expect("source core should construct");
        let payload = source
            .export_machine_state()
            .expect("machine state should export");
        let mut tampered = decode_payload::<MachineStatePayload>(&payload)
            .expect("machine state payload should decode");
        tampered
            .ppu
            .set_sprite_fetch_state_for_test(8, 0, 315, true);
        let tampered_payload = encode_payload(&tampered).expect("tampered payload should encode");

        let mut target = Core::new(nrom_test_data()).expect("target core should construct");
        let error = target
            .import_machine_state(&tampered_payload)
            .expect_err("premature terminal sprite index should reject");
        assert!(error.to_string().contains(
            "PPU sprite index terminal state is only valid after the final sprite fetch"
        ));
    }

    #[test]
    fn machine_state_accepts_terminal_ppu_sprite_index_after_final_fetch() {
        let source = Core::new(nrom_test_data()).expect("source core should construct");
        let payload = source
            .export_machine_state()
            .expect("machine state should export");
        let mut tampered = decode_payload::<MachineStatePayload>(&payload)
            .expect("machine state payload should decode");
        tampered
            .ppu
            .set_sprite_fetch_state_for_test(8, 0, 316, true);
        let tampered_payload = encode_payload(&tampered).expect("tampered payload should encode");

        let mut target = Core::new(nrom_test_data()).expect("target core should construct");
        target
            .import_machine_state(&tampered_payload)
            .expect("terminal sprite index should remain importable");
    }

    #[test]
    fn machine_state_rejects_invalid_apu_pulse_duty() {
        let source = Core::new(nrom_test_data()).expect("source core should construct");
        let payload = source
            .export_machine_state()
            .expect("machine state should export");
        let mut tampered = decode_payload::<MachineStatePayload>(&payload)
            .expect("machine state payload should decode");
        tampered.apu.set_pulse_duty_for_test(4, 0);
        let tampered_payload = encode_payload(&tampered).expect("tampered payload should encode");

        let mut target = Core::new(nrom_test_data()).expect("target core should construct");
        let before = target
            .export_machine_state()
            .expect("target state should export");

        let error = target
            .import_machine_state(&tampered_payload)
            .expect_err("invalid APU pulse duty should reject");
        assert!(error.to_string().contains("APU pulse duty mode overflow"));

        let after = target
            .export_machine_state()
            .expect("target state should still export");
        assert_eq!(before, after);
    }

    #[test]
    fn mapper_save_uses_battery_default_length_when_explicit_save_len_is_zero() {
        let core = Core::new(
            CartridgeData::new(CartridgeDataParts {
                format: RomFormat::INes,
                prog_rom: vec![0; 0x8000],
                char_rom: vec![0; 0x2000],
                pram_length: 0,
                save_pram_length: 0,
                vram_length: 0,
                save_vram_length: 0,
                mapper_type: 4,
                mirror_mode: MirrorMode::Horizontal,
                has_battery: true,
                sub_mapper_type: 0,
                trainer: Vec::new(),
            })
            .expect("test cartridge data should be valid"),
        )
        .expect("core should construct");

        let payload = core
            .export_mapper_save()
            .expect("mapper save should export")
            .expect("battery-backed mapper should expose persistent save");
        let decoded = decode_payload::<MapperSavePayload>(&payload)
            .expect("mapper save payload should decode");
        assert_eq!(decoded.prg_ram.len(), 0x2000);
    }

    #[test]
    fn mapper_save_uses_legacy_ines_prg_ram_length_when_explicit_save_len_is_zero() {
        let core = Core::new(
            CartridgeData::new(CartridgeDataParts {
                format: RomFormat::INes,
                prog_rom: vec![0; 0x20000],
                char_rom: vec![0; 0x2000],
                pram_length: 0x2000,
                save_pram_length: 0,
                vram_length: 0,
                save_vram_length: 0,
                mapper_type: 1,
                mirror_mode: MirrorMode::Horizontal,
                has_battery: true,
                sub_mapper_type: 0,
                trainer: Vec::new(),
            })
            .expect("test cartridge data should be valid"),
        )
        .expect("core should construct");

        let payload = core
            .export_mapper_save()
            .expect("mapper save should export")
            .expect("battery-backed iNES PRG RAM should expose persistent save");
        let decoded = decode_payload::<MapperSavePayload>(&payload)
            .expect("mapper save payload should decode");
        assert_eq!(decoded.prg_ram.len(), 0x2000);
        assert!(decoded.chr_ram.is_empty());
    }
}
