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
mod cartridge_data;
mod cartridge_error;
pub mod controller;
mod cpu;
mod mapper;
mod mapper_state;
mod persistence;
mod ppu;
mod ppu_bus_event;
mod ppu_memory_access;
mod status;

use self::apu::Core as Apu;
use self::cart_device::Cartridge;
use self::controller::Controller;
use self::cpu::Core as Cpu;
use self::ppu::Core as Ppu;
use crc::{CRC_64_XZ, Crc, Digest};
use nerust_screen_traits::Screen;
use nerust_sound_traits::MixerInput;

const CRC64_LEGACY_ECMA: Crc<u64> = Crc::<u64>::new(&CRC_64_XZ);

struct Crc64Hasher(Digest<'static, u64>);

impl Crc64Hasher {
    fn new() -> Self {
        Self(CRC64_LEGACY_ECMA.digest())
    }
}

fn crc64(bytes: &[u8]) -> u64 {
    let mut hasher = Crc64Hasher::new();
    hasher.0.update(bytes);
    hasher.0.finalize()
}

pub use self::cartridge_data::{CartridgeData, CartridgeDataParts, RomFormat};
pub use self::cartridge_error::CartridgeError;
pub use self::persistence::{PersistenceError, RomIdentity};
pub use self::status::mirror_mode::MirrorMode;

pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

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

impl Core {
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
            save_prg_ram_len: cartridge_data.save_pram_length(),
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
        let payload = persistence::MapperSavePayloadMessage {
            schema_version: persistence::PERSISTENCE_SCHEMA_VERSION,
            rom_identity: Some(persistence::rom_identity_to_proto(self.rom_identity())),
            options: None,
            persistent_memory: Some(self.cartridge.export_mapper_save_proto()),
        };
        Ok(Some(persistence::encode_message(&payload)?))
    }

    pub fn import_mapper_save(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let payload = persistence::decode_message::<persistence::MapperSavePayloadMessage>(bytes)?;
        persistence::validate_schema_version(payload.schema_version)?;
        let rom_identity =
            persistence::rom_identity_from_proto(payload.rom_identity.as_ref().ok_or_else(
                || persistence::PersistenceError::Validation("missing ROM identity".into()),
            )?)?;
        if self.rom_identity() != rom_identity {
            return Err(
                persistence::PersistenceError::Validation("ROM identity mismatch".into()).into(),
            );
        }
        self.cartridge
            .import_mapper_save_proto(payload.persistent_memory.as_ref().ok_or_else(|| {
                persistence::PersistenceError::Validation("missing mapper persistent memory".into())
            })?)?;
        Ok(())
    }

    pub fn export_machine_state(&self) -> Result<Vec<u8>, Error> {
        let payload = persistence::MachineStatePayloadMessage {
            schema_version: persistence::PERSISTENCE_SCHEMA_VERSION,
            rom_identity: Some(persistence::rom_identity_to_proto(self.rom_identity())),
            options: Some(persistence::options_to_proto(self.options)),
            cpu: Some(self.cpu.export_state_proto()),
            ppu: Some(self.ppu.export_state_proto()),
            apu: Some(self.apu.export_state_proto()),
            cartridge: Some(self.cartridge.export_runtime_proto()?),
        };
        Ok(persistence::encode_message(&payload)?)
    }

    pub fn import_machine_state(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let payload =
            persistence::decode_message::<persistence::MachineStatePayloadMessage>(bytes)?;
        persistence::validate_schema_version(payload.schema_version)?;
        let rom_identity =
            persistence::rom_identity_from_proto(payload.rom_identity.as_ref().ok_or_else(
                || persistence::PersistenceError::Validation("missing ROM identity".into()),
            )?)?;
        let options =
            persistence::options_from_proto(payload.options.as_ref().ok_or_else(|| {
                persistence::PersistenceError::Validation("missing core options".into())
            })?)?;
        self.validate_persistence_target(rom_identity, options)?;
        let mut cpu = Cpu::new();
        cpu.import_state_proto(payload.cpu.as_ref().ok_or_else(|| {
            persistence::PersistenceError::Validation("missing CPU state".into())
        })?)?;

        let mut ppu = Ppu::new();
        ppu.import_state_proto(payload.ppu.as_ref().ok_or_else(|| {
            persistence::PersistenceError::Validation("missing PPU state".into())
        })?)?;

        let mut apu = Apu::new(cpu.interrupt_mut());
        apu.import_state_proto(payload.apu.as_ref().ok_or_else(|| {
            persistence::PersistenceError::Validation("missing APU state".into())
        })?)?;
        // APU construction performs frame-counter initialization against the CPU interrupt
        // lines, so restore the saved CPU interrupt state again after rebuilding the APU.
        cpu.import_state_proto(payload.cpu.as_ref().ok_or_else(|| {
            persistence::PersistenceError::Validation("missing CPU state".into())
        })?)?;

        let cartridge_data = self.cartridge.data_ref().clone();
        let mut cartridge = crate::cartridge::try_from_with_options(cartridge_data, self.options)?;
        cartridge.import_runtime_proto(payload.cartridge.as_ref().ok_or_else(|| {
            persistence::PersistenceError::Validation("missing cartridge runtime".into())
        })?)?;

        self.cpu = cpu;
        self.ppu = ppu;
        self.apu = apu;
        self.cartridge = cartridge;
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
        loop {
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
        for _ in 0..3 {
            if self
                .ppu
                .step(screen, self.cartridge.as_mut(), self.cpu.interrupt_mut())
            {
                result = true;
            }
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

    fn validate_persistence_target(
        &self,
        identity: RomIdentity,
        options: CoreOptions,
    ) -> Result<(), persistence::PersistenceError> {
        if self.rom_identity() != identity {
            return Err(persistence::PersistenceError::Validation(
                "ROM identity mismatch".into(),
            ));
        }
        if self.options != options {
            return Err(persistence::PersistenceError::Validation(
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
mod persistence_tests {
    use super::*;

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
        let mut tampered =
            persistence::decode_message::<persistence::MachineStatePayloadMessage>(&payload)
                .expect("machine state payload should decode");
        tampered
            .cpu
            .as_mut()
            .expect("CPU state should exist")
            .cycles = 42;
        tampered
            .ppu
            .as_mut()
            .expect("PPU state should exist")
            .vram
            .pop();
        let tampered_payload =
            persistence::encode_message(&tampered).expect("tampered payload should encode");

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
        let mut tampered =
            persistence::decode_message::<persistence::MachineStatePayloadMessage>(&payload)
                .expect("machine state payload should decode");
        tampered
            .cartridge
            .as_mut()
            .expect("cartridge runtime should exist")
            .mapper_state
            .as_mut()
            .expect("mapper state should exist")
            .program_page_table[0] = i32::MAX;
        let tampered_payload =
            persistence::encode_message(&tampered).expect("tampered payload should encode");

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
        let decoded =
            persistence::decode_message::<persistence::MapperSavePayloadMessage>(&payload)
                .expect("mapper save payload should decode");
        let persistent = decoded
            .persistent_memory
            .expect("mapper persistent memory should exist");
        assert_eq!(persistent.prg_ram.len(), 0x2000);
    }
}
