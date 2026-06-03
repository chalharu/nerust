// Copyright (c) 2026 chalharu
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

mod apu;
mod bus;
mod cartridge;
mod cpu;
mod enhancement;
mod mapper;
mod memory;
mod ppu1;
mod ppu2;
mod scheduler;

pub use cartridge::{Cartridge, CartridgeError, CartridgeHeader};
pub use cpu::{CpuState, CpuStatus, Registers};
pub use enhancement::EnhancementChip;
pub use mapper::MapperKind;
pub use ppu1::Mode7Registers;

use bus::Bus;
use cpu::{Cpu, CpuFault};
use nerust_sound_traits::MixerInput;
use scheduler::Scheduler;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentedBackdropLine {
    pub inidisp: u8,
    pub color0: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentedBg1Line {
    pub hofs: u16,
    pub vofs: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentedMainScreenLine {
    pub tm: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentedColorWindowLine {
    pub wh0: u8,
    pub wh1: u8,
    pub wh2: u8,
    pub wh3: u8,
}

pub struct Core {
    cpu: Cpu,
    bus: Bus,
    scheduler: Scheduler,
}

impl Core {
    pub fn new(cartridge: Cartridge) -> Self {
        Self {
            cpu: Cpu::new(),
            bus: Bus::new(cartridge),
            scheduler: Scheduler::new(),
        }
    }

    pub fn from_rom_bytes(bytes: &[u8]) -> Result<Self, CoreError> {
        Ok(Self::new(Cartridge::from_bytes(bytes)?))
    }

    pub fn from_rom_bytes_with_msu1_data(
        bytes: &[u8],
        msu1_data: &[u8],
    ) -> Result<Self, CoreError> {
        let mut cartridge = Cartridge::from_bytes(bytes)?;
        cartridge.load_msu1_data(msu1_data);
        Ok(Self::new(cartridge))
    }

    pub fn from_rom_bytes_with_msu1_sidecars(
        bytes: &[u8],
        msu1_data: Option<&[u8]>,
        msu1_audio_tracks: &[u16],
    ) -> Result<Self, CoreError> {
        let mut cartridge = Cartridge::from_bytes(bytes)?;
        if let Some(msu1_data) = msu1_data {
            cartridge.load_msu1_data(msu1_data);
        }
        cartridge.set_msu1_audio_tracks(msu1_audio_tracks.iter().copied());
        Ok(Self::new(cartridge))
    }

    pub fn step(&mut self) -> Result<(), CoreError> {
        if self.cpu.current_state() == CpuState::Stopped {
            return Ok(());
        }
        let start_cycles = self.cpu.cycles();
        self.bus.tick_cpu_cycle();
        self.cpu.step(&mut self.bus);
        self.scheduler
            .advance(self.cpu.cycles().wrapping_sub(start_cycles));
        if let Some(fault) = self.cpu.take_fault() {
            return Err(fault.into());
        }
        Ok(())
    }

    /// Runs at least `cycles` CPU cycles, stopping early if the CPU stops.
    ///
    /// Execution is instruction-bounded and may overshoot the requested budget
    /// by the remaining cycles in the current instruction.
    pub fn run_for_cycles(&mut self, cycles: u64) -> Result<(), CoreError> {
        self.scheduler
            .run_for_cycles(&mut self.cpu, &mut self.bus, cycles)
            .map_err(Into::into)
    }

    pub fn run_for_cycles_with_audio<M: MixerInput>(
        &mut self,
        cycles: u64,
        mixer: &mut M,
    ) -> Result<(), CoreError> {
        self.scheduler
            .run_for_cycles_with_audio(&mut self.cpu, &mut self.bus, cycles, mixer)
            .map_err(Into::into)
    }

    pub fn reset_cpu(&mut self) {
        self.cpu.reset();
        self.bus.reset_ephemeral_state();
        self.scheduler.reset();
    }

    pub fn registers(&self) -> &Registers {
        self.cpu.registers()
    }

    pub fn cycles(&self) -> u64 {
        self.cpu.cycles()
    }

    pub fn master_cycles(&self) -> u64 {
        self.scheduler.master_cycles()
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

    pub fn export_save_ram(&self) -> Option<Vec<u8>> {
        let save_ram = self.bus.cartridge().save_ram();
        (!save_ram.is_empty()).then(|| save_ram.to_vec())
    }

    pub fn load_save_ram(&mut self, save_ram: &[u8]) -> Result<(), CoreError> {
        self.bus.cartridge_mut().load_save_ram(save_ram)?;
        Ok(())
    }

    pub fn load_msu1_data(&mut self, data: &[u8]) {
        self.bus.cartridge_mut().load_msu1_data(data);
    }

    pub fn set_msu1_audio_tracks<I>(&mut self, tracks: I)
    where
        I: IntoIterator<Item = u16>,
    {
        self.bus.cartridge_mut().set_msu1_audio_tracks(tracks);
    }

    pub fn set_standard_controller_buttons(&mut self, port: usize, buttons: u16) -> bool {
        self.bus.set_standard_controller_buttons(port, buttons)
    }

    pub fn peek(&self, address: u32) -> u8 {
        self.bus.peek(address)
    }

    pub fn peek_wram(&self, address: usize) -> u8 {
        self.bus.memory.peek_wram(address)
    }

    pub fn peek_apu_ram(&self, address: u16) -> u8 {
        self.bus.peek_apu_ram(address)
    }

    pub fn peek_vram(&self, address: usize) -> u8 {
        self.bus.ppu1.peek_vram(address)
    }

    pub fn peek_cgram(&self, index: usize) -> u8 {
        self.bus.ppu2.peek_cgram(index)
    }

    pub fn fixed_color(&self) -> u16 {
        self.bus.ppu2.fixed_color()
    }

    pub fn peek_oam(&self, address: usize) -> u8 {
        self.bus.ppu1.peek_oam(address)
    }

    pub fn bg1_hofs(&self) -> u16 {
        self.bus.ppu1.bg1_hofs()
    }

    pub fn bg1_vofs(&self) -> u16 {
        self.bus.ppu1.bg1_vofs()
    }

    pub fn bg2_hofs(&self) -> u16 {
        self.bus.ppu1.bg2_hofs()
    }

    pub fn bg2_vofs(&self) -> u16 {
        self.bus.ppu1.bg2_vofs()
    }

    pub fn bg3_hofs(&self) -> u16 {
        self.bus.ppu1.bg3_hofs()
    }

    pub fn bg3_vofs(&self) -> u16 {
        self.bus.ppu1.bg3_vofs()
    }

    pub fn bg4_hofs(&self) -> u16 {
        self.bus.ppu1.bg4_hofs()
    }

    pub fn bg4_vofs(&self) -> u16 {
        self.bus.ppu1.bg4_vofs()
    }

    pub fn mode7_registers(&self) -> Mode7Registers {
        self.bus.ppu1.mode7_registers()
    }

    pub fn presented_backdrop_line(&self, line: usize) -> Option<PresentedBackdropLine> {
        self.bus.presented_backdrop_line(line)
    }

    pub fn presented_bg1_line(&self, line: usize) -> Option<PresentedBg1Line> {
        self.bus.presented_bg1_line(line)
    }

    pub fn presented_bg2_line(&self, line: usize) -> Option<PresentedBg1Line> {
        self.bus.presented_bg2_line(line)
    }

    pub fn presented_bg3_line(&self, line: usize) -> Option<PresentedBg1Line> {
        self.bus.presented_bg3_line(line)
    }

    pub fn presented_bg4_line(&self, line: usize) -> Option<PresentedBg1Line> {
        self.bus.presented_bg4_line(line)
    }

    pub fn presented_main_screen_line(&self, line: usize) -> Option<PresentedMainScreenLine> {
        self.bus.presented_main_screen_line(line)
    }

    pub fn presented_color_window_line(&self, line: usize) -> Option<PresentedColorWindowLine> {
        self.bus.presented_color_window_line(line)
    }
}

#[cfg(test)]
mod tests {
    use super::{Core, CpuState, CpuStatus, MapperKind};

    const HEADER_OFFSET: usize = 0x7FC0;
    const RESET_VECTOR_OFFSET: usize = 0x7FFC;
    const IRQ_VECTOR_OFFSET: usize = 0x7FFE;
    const HIROM_HEADER_OFFSET: usize = 0xFFC0;
    const HIROM_RESET_VECTOR_OFFSET: usize = 0xFFFC;

    fn build_lorom(reset_vector: u16) -> Vec<u8> {
        let mut rom = vec![0; 0x10000];
        rom[HEADER_OFFSET..HEADER_OFFSET + 21].copy_from_slice(b"TEST CORE ROM        ");
        rom[0x7FD5] = 0x30;
        rom[0x7FD7] = 0x08;
        rom[RESET_VECTOR_OFFSET..RESET_VECTOR_OFFSET + 2]
            .copy_from_slice(&reset_vector.to_le_bytes());
        rom
    }

    fn build_hirom(reset_vector: u16) -> Vec<u8> {
        let mut rom = vec![0; 0x20000];
        rom[HIROM_HEADER_OFFSET..HIROM_HEADER_OFFSET + 21]
            .copy_from_slice(b"TEST HIROM CORE      ");
        rom[0xFFD5] = 0x31;
        rom[0xFFD7] = 0x09;
        rom[HIROM_RESET_VECTOR_OFFSET..HIROM_RESET_VECTOR_OFFSET + 2]
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
    fn core_reset_fetches_the_hirom_reset_vector_and_program_byte() {
        let mut rom = build_hirom(0x8000);
        rom[0x8000] = 0xDB;

        let mut core = Core::from_rom_bytes(&rom).unwrap();
        run_until_stopped(&mut core, 16);

        assert_eq!(core.current_state(), CpuState::Stopped);
        assert_eq!(core.current_opcode(), 0xDB);
        assert_eq!(core.registers().pc(), 0x8001);
        assert_eq!(core.cartridge().header().mapper_kind(), MapperKind::HiRom);
    }

    #[test]
    fn core_exports_and_restores_cartridge_save_ram() {
        let mut rom = build_lorom(0x8000);
        rom[0x7FD8] = 0x03;
        let mut core = Core::from_rom_bytes(&rom).unwrap();
        let mut save_ram = vec![0x5A; core.export_save_ram().unwrap().len()];
        save_ram[0x0123] = 0xC3;

        core.load_save_ram(&save_ram).unwrap();

        assert_eq!(core.export_save_ram().unwrap()[0x0123], 0xC3);
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
    fn core_run_for_cycles_matches_step_loop_until_stopped() {
        let program = [0xEA, 0xEA, 0xDB];
        let mut rom = build_lorom(0x8000);
        rom[..program.len()].copy_from_slice(&program);

        let mut stepped = Core::from_rom_bytes(&rom).unwrap();
        let mut scheduled = Core::from_rom_bytes(&rom).unwrap();

        run_until_stopped(&mut stepped, 512);
        scheduled.run_for_cycles(512).unwrap();

        assert_eq!(scheduled.current_state(), CpuState::Stopped);
        assert_eq!(scheduled.current_opcode(), stepped.current_opcode());
        assert_eq!(scheduled.registers(), stepped.registers());
        assert_eq!(scheduled.cycles(), stepped.cycles());
        assert_eq!(scheduled.master_cycles(), scheduled.cycles());
    }

    #[test]
    fn core_run_for_cycles_fast_forwards_self_branch_idle_loop() {
        let mut rom = build_lorom(0x8000);
        rom[0x0000..0x0002].copy_from_slice(&[0x80, 0xFE]);

        let mut core = Core::from_rom_bytes(&rom).unwrap();
        core.run_for_cycles(10_000).unwrap();

        assert_eq!(core.current_state(), CpuState::Running);
        assert_eq!(core.current_opcode(), 0x80);
        assert_eq!(core.registers().pc(), 0x8000);
        assert!(core.cycles() >= 10_000);
        assert_eq!(core.master_cycles(), core.cycles());
    }

    #[test]
    fn core_run_for_cycles_fast_forwards_absolute_jmp_idle_loop() {
        let mut rom = build_lorom(0x8000);
        rom[0x0000..0x0003].copy_from_slice(&[0x4C, 0x00, 0x80]);

        let mut core = Core::from_rom_bytes(&rom).unwrap();
        core.run_for_cycles(10_000).unwrap();

        assert_eq!(core.current_state(), CpuState::Running);
        assert_eq!(core.current_opcode(), 0x4C);
        assert_eq!(core.registers().pc(), 0x8000);
        assert!(core.cycles() >= 10_000);
        assert_eq!(core.master_cycles(), core.cycles());
    }

    #[test]
    fn core_run_for_cycles_fast_forwards_wai_until_interrupt_event() {
        let mut rom = build_lorom(0x8000);
        rom[0x0000..0x0006].copy_from_slice(&[0xA9, 0x80, 0x8D, 0x00, 0x42, 0xCB]);

        let mut core = Core::from_rom_bytes(&rom).unwrap();
        core.run_for_cycles(1_000).unwrap();

        assert_eq!(core.current_state(), CpuState::Waiting);
        assert_eq!(core.current_opcode(), 0xCB);
        assert!(core.cycles() >= 1_000);
        assert_eq!(core.master_cycles(), core.cycles());
    }

    #[test]
    fn core_executes_bootstrap_rom_across_cpu_ppu_and_memory() {
        let program = [
            0x18, 0xFB, 0xC2, 0x30, 0xE2, 0x20, 0xA2, 0xEF, 0x01, 0x9A, 0xA9, 0x8F, 0x8D, 0x00,
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
        assert_eq!(core.bus.ppu2.inidisp(), 0x8F);
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
    fn core_vcounter_irq_wakes_wai_and_returns_through_timeup_handler() {
        let mut rom = build_lorom(0x8000);
        rom[IRQ_VECTOR_OFFSET..IRQ_VECTOR_OFFSET + 2].copy_from_slice(&0x9000u16.to_le_bytes());
        rom[0x0000..0x000C].copy_from_slice(&[
            0xA0, 0x28, 0x8C, 0x09, 0x42, 0xA9, 0x20, 0x8D, 0x00, 0x42, 0x58, 0xCB,
        ]);
        rom[0x000C] = 0xDB; // STP after WAI returns
        rom[0x1000..0x1004].copy_from_slice(&[0xAD, 0x11, 0x42, 0x40]); // LDA $4211 ; RTI

        let mut core = Core::from_rom_bytes(&rom).unwrap();
        run_until_stopped(&mut core, 20_000);

        assert_eq!(core.current_state(), CpuState::Stopped);
        assert_eq!(core.current_opcode(), 0xDB);
        assert_eq!(core.registers().pc(), 0x800D);
        assert_eq!(core.peek(0x004211), 0x00);
    }

    #[test]
    fn core_combined_hv_irq_wakes_wai_and_returns_through_timeup_handler() {
        let mut rom = build_lorom(0x8000);
        rom[IRQ_VECTOR_OFFSET..IRQ_VECTOR_OFFSET + 2].copy_from_slice(&0x9000u16.to_le_bytes());
        rom[0x0000..0x0012].copy_from_slice(&[
            0xA0, 0x14, 0x8C, 0x09, 0x42, // LDY #20 ; STY VTIME
            0xA0, 0x89, 0x8C, 0x07, 0x42, // LDY #137 ; STY HTIME
            0xA9, 0x30, 0x8D, 0x00, 0x42, // LDA #$30 ; STA NMITIMEN
            0x58, 0xCB, 0xDB, // CLI ; WAI ; STP
        ]);
        rom[0x1000..0x1004].copy_from_slice(&[0xAD, 0x11, 0x42, 0x40]); // LDA $4211 ; RTI

        let mut core = Core::from_rom_bytes(&rom).unwrap();
        run_until_stopped(&mut core, 20_000);

        assert_eq!(core.current_state(), CpuState::Stopped);
        assert_eq!(core.current_opcode(), 0xDB);
        assert_eq!(core.registers().pc(), 0x8012);
        assert_eq!(core.peek(0x004211), 0x00);
    }

    #[test]
    fn core_recurring_hcounter_irq_allows_progress_after_rti() {
        let mut rom = build_lorom(0x8000);
        rom[IRQ_VECTOR_OFFSET..IRQ_VECTOR_OFFSET + 2].copy_from_slice(&0x9000u16.to_le_bytes());
        rom[0x0000..0x000F].copy_from_slice(&[
            0xA9, 0x01, 0x8D, 0x07, 0x42, // LDA #$01 ; STA HTIMEL
            0xA9, 0x10, 0x8D, 0x00, 0x42, // LDA #$10 ; STA NMITIMEN
            0x58, 0xCB, 0xE6, 0x00, 0xDB, // CLI ; WAI ; INC $00 ; STP
        ]);
        rom[0x1000..0x1004].copy_from_slice(&[0xAD, 0x11, 0x42, 0x40]); // LDA $4211 ; RTI

        let mut core = Core::from_rom_bytes(&rom).unwrap();
        run_until_stopped(&mut core, 1024);

        assert_eq!(core.current_state(), CpuState::Stopped);
        assert_eq!(core.current_opcode(), 0xDB);
        assert_eq!(core.peek(0x7E0000), 0x01);
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

    #[test]
    fn core_can_wait_for_auto_joy_hvbjoy_pulse_and_read_zeroed_joy1() {
        let mut rom = build_lorom(0x8000);
        rom[0x0000..0x001A].copy_from_slice(&[
            0xA9, 0x01, // LDA #$01
            0x8D, 0x00, 0x42, // STA $4200
            0xAD, 0x12, 0x42, // wait_set: LDA $4212
            0x29, 0x01, // AND #$01
            0xF0, 0xF9, // BEQ wait_set
            0xAD, 0x12, 0x42, // wait_clear: LDA $4212
            0x29, 0x01, // AND #$01
            0xD0, 0xF9, // BNE wait_clear
            0xAD, 0x18, 0x42, // LDA $4218
            0x8D, 0x00, 0x00, // STA $0000
            0xDB, // STP
        ]);

        let mut core = Core::from_rom_bytes(&rom).unwrap();
        run_until_stopped(&mut core, 120_000);

        assert_eq!(core.current_state(), CpuState::Stopped);
        assert_eq!(core.current_opcode(), 0xDB);
        assert_eq!(core.peek(0x7E0000), 0x00);
        assert_eq!(core.peek(0x004218), 0x00);
    }

    #[test]
    fn core_can_pulse_joyout_and_observe_the_seventeenth_joyser0_read() {
        let mut rom = build_lorom(0x8000);
        let program = [
            0xA9, 0x01, // LDA #$01
            0x8D, 0x16, 0x40, // STA $4016
            0xA9, 0x00, // LDA #$00
            0x8D, 0x16, 0x40, // STA $4016
            0xAD, 0x16, 0x40, // 1
            0xAD, 0x16, 0x40, // 2
            0xAD, 0x16, 0x40, // 3
            0xAD, 0x16, 0x40, // 4
            0xAD, 0x16, 0x40, // 5
            0xAD, 0x16, 0x40, // 6
            0xAD, 0x16, 0x40, // 7
            0xAD, 0x16, 0x40, // 8
            0xAD, 0x16, 0x40, // 9
            0xAD, 0x16, 0x40, // 10
            0xAD, 0x16, 0x40, // 11
            0xAD, 0x16, 0x40, // 12
            0xAD, 0x16, 0x40, // 13
            0xAD, 0x16, 0x40, // 14
            0xAD, 0x16, 0x40, // 15
            0xAD, 0x16, 0x40, // 16
            0xAD, 0x16, 0x40, // 17
            0x8D, 0x00, 0x00, // STA $0000
            0xDB, // STP
        ];
        rom[..program.len()].copy_from_slice(&program);

        let mut core = Core::from_rom_bytes(&rom).unwrap();
        run_until_stopped(&mut core, 256);

        assert_eq!(core.current_state(), CpuState::Stopped);
        assert_eq!(core.current_opcode(), 0xDB);
        assert_eq!(core.peek(0x7E0000), 0x01);
    }
}
