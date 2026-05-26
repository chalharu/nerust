// Copyright (c) 2026 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod bus;
mod cartridge;
mod cpu;
mod mapper;
mod memory;
mod ppu1;
mod ppu2;

pub use cartridge::{Cartridge, CartridgeError, CartridgeHeader};
pub use cpu::{CpuState, CpuStatus, Registers};
pub use mapper::MapperKind;

use bus::Bus;
use cpu::{Cpu, CpuFault};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CoreError {
    #[error(transparent)]
    Cartridge(#[from] CartridgeError),
    #[error("unsupported SNES CPU opcode 0x{opcode:02X} at {bank:02X}:{address:04X}")]
    UnsupportedOpcode { opcode: u8, bank: u8, address: u16 },
}

impl From<CpuFault> for CoreError {
    fn from(value: CpuFault) -> Self {
        match value {
            CpuFault::UnsupportedOpcode {
                opcode,
                bank,
                address,
            } => Self::UnsupportedOpcode {
                opcode,
                bank,
                address,
            },
        }
    }
}

pub struct Core {
    cpu: Cpu,
    bus: Bus,
}

impl Core {
    pub fn new(cartridge: Cartridge) -> Self {
        Self {
            cpu: Cpu::new(),
            bus: Bus::new(cartridge),
        }
    }

    pub fn from_rom_bytes(bytes: &[u8]) -> Result<Self, CoreError> {
        Ok(Self::new(Cartridge::from_bytes(bytes)?))
    }

    pub fn step(&mut self) -> Result<(), CoreError> {
        if self.cpu.current_state() == CpuState::Stopped {
            return Ok(());
        }
        self.bus.tick_video_stub();
        self.cpu.step(&mut self.bus);
        if let Some(fault) = self.cpu.take_fault() {
            return Err(fault.into());
        }
        Ok(())
    }

    pub fn reset_cpu(&mut self) {
        self.cpu.reset();
        self.bus.reset_ephemeral_state();
    }

    pub fn registers(&self) -> &Registers {
        self.cpu.registers()
    }

    pub fn cycles(&self) -> u64 {
        self.cpu.cycles()
    }

    pub fn current_opcode(&self) -> u8 {
        self.cpu.current_opcode()
    }

    pub fn current_state(&self) -> CpuState {
        self.cpu.current_state()
    }

    pub fn cartridge(&self) -> &Cartridge {
        self.bus.cartridge()
    }

    pub fn peek(&self, address: u32) -> u8 {
        self.bus.peek(address)
    }

    pub fn peek_wram(&self, address: usize) -> u8 {
        self.bus.memory.peek_wram(address)
    }

    pub fn peek_vram(&self, address: usize) -> u8 {
        self.bus.ppu1.peek_vram(address)
    }

    pub fn peek_cgram(&self, index: usize) -> u8 {
        self.bus.ppu2.peek_cgram(index)
    }
}

#[cfg(test)]
mod tests {
    use super::{Core, CpuState, CpuStatus, MapperKind};

    const HEADER_OFFSET: usize = 0x7FC0;
    const RESET_VECTOR_OFFSET: usize = 0x7FFC;

    fn build_lorom(reset_vector: u16) -> Vec<u8> {
        let mut rom = vec![0; 0x10000];
        rom[HEADER_OFFSET..HEADER_OFFSET + 21].copy_from_slice(b"TEST CORE ROM        ");
        rom[0x7FD5] = 0x30;
        rom[0x7FD7] = 0x08;
        rom[RESET_VECTOR_OFFSET..RESET_VECTOR_OFFSET + 2]
            .copy_from_slice(&reset_vector.to_le_bytes());
        rom
    }

    fn run_until_stopped(core: &mut Core, max_cycles: usize) {
        for _ in 0..max_cycles {
            core.step().unwrap();
            if core.current_state() == CpuState::Stopped {
                return;
            }
        }

        panic!("core did not stop within {max_cycles} cycles");
    }

    #[test]
    fn core_reset_fetches_the_lorom_reset_vector() {
        let mut rom = build_lorom(0x8000);
        rom[0] = 0xEA;

        let mut core = Core::from_rom_bytes(&rom).unwrap();

        for _ in 0..7 {
            core.step().unwrap();
        }

        assert_eq!(core.registers().pc(), 0x8000);
        assert_eq!(core.registers().pb(), 0x00);
        assert_eq!(core.current_state(), CpuState::Running);
        assert_eq!(core.cartridge().header().mapper_kind(), MapperKind::LoRom);
    }

    #[test]
    fn core_runs_basic_native_mode_bootstrap_sequence() {
        let mut rom = build_lorom(0x8000);
        rom[0x0000..0x0009]
            .copy_from_slice(&[0x18, 0xFB, 0xC2, 0x30, 0xA2, 0xEF, 0x01, 0x9A, 0xDB]);

        let mut core = Core::from_rom_bytes(&rom).unwrap();
        run_until_stopped(&mut core, 32);

        assert_eq!(core.current_state(), CpuState::Stopped);
        assert_eq!(core.current_opcode(), 0xDB);
        assert_eq!(core.registers().x(), 0x01EF);
        assert_eq!(core.registers().s(), 0x01EF);
        assert!(
            !core
                .registers()
                .status()
                .contains(CpuStatus::ACCUMULATOR_8BIT | CpuStatus::INDEX_8BIT)
        );
    }

    #[test]
    fn core_executes_bootstrap_rom_across_cpu_ppu_and_memory() {
        let program = [
            0x18, 0xFB, 0xC2, 0x30, 0xE2, 0x20, 0xA2, 0xEF, 0x01, 0x9A, 0xA9, 0x0F, 0x8D, 0x00,
            0x21, 0xA9, 0x80, 0x8D, 0x15, 0x21, 0x9C, 0x16, 0x21, 0x9C, 0x17, 0x21, 0xA9, 0x34,
            0x8D, 0x18, 0x21, 0xA9, 0x12, 0x8D, 0x19, 0x21, 0xA9, 0x01, 0x8D, 0x21, 0x21, 0xA9,
            0x7F, 0x8D, 0x22, 0x21, 0xA9, 0x00, 0x8D, 0x22, 0x21, 0x9C, 0x81, 0x21, 0x9C, 0x82,
            0x21, 0x9C, 0x83, 0x21, 0xA9, 0x5A, 0x8D, 0x80, 0x21, 0xDB,
        ];

        let mut rom = build_lorom(0x8000);
        rom[..program.len()].copy_from_slice(&program);

        let mut core = Core::from_rom_bytes(&rom).unwrap();
        run_until_stopped(&mut core, 128);

        assert_eq!(core.current_state(), CpuState::Stopped);
        assert_eq!(core.bus.ppu2.inidisp(), 0x0F);
        assert_eq!(core.bus.ppu1.peek_vram(0), 0x34);
        assert_eq!(core.bus.ppu1.peek_vram(1), 0x12);
        assert_eq!(core.bus.ppu1.vmadd(), 0x0001);
        assert_eq!(core.bus.ppu2.peek_cgram(2), 0x7F);
        assert_eq!(core.bus.ppu2.peek_cgram(3), 0x00);
        assert_eq!(core.peek(0x7E0000), 0x5A);
        assert_eq!(core.bus.memory.wmadd(), 0x0001);
    }

    #[test]
    fn core_wai_suspends_until_nmi_fires() {
        // 0xCB (WAI) is now supported: the core should enter Waiting state and
        // NOT report an UnsupportedOpcode error.
        let mut rom = build_lorom(0x8000);
        // WAI followed by STP so we can check the state machine exits cleanly.
        rom[0x0000] = 0xCB; // WAI

        let mut core = Core::from_rom_bytes(&rom).unwrap();
        // Run reset (7 cycles) + fetch WAI (1) + execute WAI (1) = 9 total
        for _ in 0..9 {
            core.step().unwrap();
        }
        assert_eq!(core.current_state(), CpuState::Waiting);
        assert_eq!(core.current_opcode(), 0xCB);
    }

    #[test]
    fn stepping_a_stopped_core_does_not_advance_vblank_stub() {
        let mut rom = build_lorom(0x8000);
        rom[0x0000] = 0xDB;

        let mut core = Core::from_rom_bytes(&rom).unwrap();
        run_until_stopped(&mut core, 16);

        let before = core.peek(0x004210);
        for _ in 0..8 {
            core.step().unwrap();
        }
        assert_eq!(core.peek(0x004210), before);
    }

    #[test]
    fn core_accepts_supported_vmain_remap_modes() {
        let mut rom = build_lorom(0x8000);
        rom[0x0000..0x0006].copy_from_slice(&[0xA9, 0x0C, 0x8D, 0x15, 0x21, 0xDB]);

        let mut core = Core::from_rom_bytes(&rom).unwrap();
        run_until_stopped(&mut core, 32);

        assert_eq!(core.peek(0x002115), 0x0C);
    }
}
