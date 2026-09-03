use crate::cartridge::Cartridge;
use crate::cpu::GbaCpu;
use crate::memory::GbaMemoryBus;

const IRQ_ENTRY_CYCLES: u32 = 22;

pub struct GbaSystem {
    pub cpu: GbaCpu,
    pub bus: GbaMemoryBus,
    tick: u64,
    cpu_cycles_remaining: u32,
}

impl GbaSystem {
    pub fn new() -> Self {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        cpu.reset(&mut bus);
        Self {
            cpu,
            bus,
            tick: 0,
            cpu_cycles_remaining: 0,
        }
    }

    pub fn from_rom(rom: Vec<u8>) -> Option<Self> {
        let cart = Cartridge::new(rom)?;
        if !cart.header.logo_valid || !cart.header.fixed_valid || !cart.header.complement_valid {
            return None;
        }
        Some(Self::from_cartridge(cart))
    }

    /// Load third-party test ROMs that intentionally use a non-standard logo byte.
    /// Fixed-byte and complement validation remain mandatory.
    pub fn from_test_rom(rom: Vec<u8>) -> Option<Self> {
        let cart = Cartridge::new(rom)?;
        if !cart.header.fixed_valid || !cart.header.complement_valid {
            return None;
        }
        Some(Self::from_cartridge(cart))
    }

    fn from_cartridge(cart: Cartridge) -> Self {
        let mut bus = GbaMemoryBus::new();
        bus.set_cartridge(cart);
        let mut cpu = GbaCpu::post_bios();
        cpu.reset(&mut bus);
        Self {
            cpu,
            bus,
            tick: 0,
            cpu_cycles_remaining: 0,
        }
    }

    pub fn bus(&self) -> &GbaMemoryBus {
        &self.bus
    }

    pub fn bus_mut(&mut self) -> &mut GbaMemoryBus {
        &mut self.bus
    }

    pub fn cpu(&self) -> &GbaCpu {
        &self.cpu
    }

    pub fn cpu_mut(&mut self) -> &mut GbaCpu {
        &mut self.cpu
    }

    pub fn frame_buffer(&self) -> &[u32] {
        self.bus.frame_buffer()
    }

    pub fn run_frame(&mut self) -> &[u32] {
        while !self.step_tcycle() {}
        self.frame_buffer()
    }

    /// CPUとバスを1 T-cycleだけ進行する。
    pub fn step_tcycle(&mut self) -> bool {
        if !self.bus.is_halted() && !self.bus.dma_active() && self.cpu_cycles_remaining == 0 {
            if self.bus.hle_bios_active() {
                self.cpu_cycles_remaining = self.bus.step_hle_bios().max(1);
            } else {
                let irq_source_pc = self.cpu.registers().pc();
                let irq_entry_cycles = IRQ_ENTRY_CYCLES
                    + u32::from(
                        self.bus
                            .nonsequential_cycles_for(irq_source_pc, 4)
                            .saturating_sub(1),
                    );
                if self.cpu.service_irq(&mut self.bus) {
                    self.cpu_cycles_remaining = irq_entry_cycles;
                } else {
                    self.cpu_cycles_remaining = self.cpu.step(&mut self.bus).max(1);
                }
            }
        }
        self.cpu_cycles_remaining = self.cpu_cycles_remaining.saturating_sub(1);
        self.tick = self.tick.wrapping_add(1);
        self.bus.tick()
    }
}

impl Default for GbaSystem {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::header::finalize_test_gba_rom;

    fn start_cpu_set(system: &mut GbaSystem, source: u32, destination: u32, len_mode: u32) {
        let registers = system.cpu.registers_mut();
        registers.set_r(0, source);
        registers.set_r(1, destination);
        registers.set_r(2, len_mode);
        crate::bios::handle_swi(registers, &mut system.bus, 0x0B);
    }

    #[test]
    fn step_tcycle_advances_exactly_one_cycle() {
        let mut system = GbaSystem::new();
        assert_eq!(system.tick, 0);
        system.step_tcycle();
        assert_eq!(system.tick, 1);
        assert!(system.cpu_cycles_remaining > 0);
    }

    #[test]
    fn halted_system_does_not_execute_cpu() {
        let mut system = GbaSystem::new();
        let pc = system.cpu.registers().pc();
        system.bus.enter_halt(1);
        for _ in 0..10 {
            system.step_tcycle();
        }
        assert_eq!(system.cpu.registers().pc(), pc);
    }

    #[test]
    fn hle_bios_operation_blocks_caller_until_complete() {
        let mut system = GbaSystem::new();
        system.bus.write32(0x03000000, 0x12345678);
        start_cpu_set(&mut system, 0x03000000, 0x03000004, (1 << 26) | 1);
        let caller_pc = system.cpu.registers().pc();

        while system.bus.hle_bios_active() {
            system.step_tcycle();
            assert_eq!(system.cpu.registers().pc(), caller_pc);
        }

        assert_eq!(system.bus.read32(0x03000004), 0x12345678);
    }

    #[test]
    fn hle_bios_operation_resumes_after_halt() {
        let mut system = GbaSystem::new();
        system.bus.write16(0x04000200, 1);
        system.bus.write32(0x03000000, 1);
        start_cpu_set(&mut system, 0x03000000, 0x04000300, (1 << 26) | 1);

        while !system.bus.is_halted() {
            system.step_tcycle();
        }
        assert!(system.bus.hle_bios_active());
        for _ in 0..4 {
            system.step_tcycle();
        }
        assert!(system.bus.hle_bios_active());

        system.bus.request_interrupt(1);
        while system.bus.hle_bios_active() {
            system.step_tcycle();
        }
        assert!(!system.bus.is_halted());
    }

    #[test]
    fn dma_preempts_hle_bios_transfer() {
        let mut system = GbaSystem::new();
        for index in 0..8 {
            system
                .bus
                .write32(0x03000000 + index * 4, 0x10000000 + index);
        }
        start_cpu_set(&mut system, 0x03000000, 0x03000040, (1 << 26) | 8);

        while system.bus.read32(0x03000040) == 0 {
            system.step_tcycle();
        }

        for index in 0..4 {
            system
                .bus
                .write32(0x03000100 + index * 4, 0xA0000000 + index);
        }
        system.bus.write32(0x040000D4, 0x03000100);
        system.bus.write32(0x040000D8, 0x02000000);
        system.bus.write32(0x040000DC, 0x84000004);

        while !system.bus.dma_active() {
            system.step_tcycle();
        }
        assert!(system.bus.dma_active());
        assert_eq!(system.bus.read32(0x03000040), 0x10000000);
        assert_eq!(system.bus.read32(0x0300005C), 0);

        while system.bus.dma_active() || system.bus.hle_bios_active() {
            system.step_tcycle();
        }
        assert_eq!(system.bus.read32(0x0200000C), 0xA0000003);
        for index in 0..8 {
            assert_eq!(
                system.bus.read32(0x03000040 + index * 4),
                0x10000000 + index
            );
        }
    }

    #[test]
    fn run_frame_advances_one_lcd_frame() {
        let mut system = GbaSystem::new();
        assert_eq!(
            system.run_frame().len(),
            crate::ppu::WIDTH * crate::ppu::HEIGHT
        );
        assert_eq!(system.tick, 280896);
    }

    #[test]
    fn irq_entry_cycles_use_nonsequential_source_wait() {
        let bus = GbaMemoryBus::new();
        assert_eq!(bus.nonsequential_cycles_for(0x03000000, 4), 1);
        assert_eq!(bus.nonsequential_cycles_for(0x02000000, 4), 6);
        assert_eq!(bus.nonsequential_cycles_for(0x08000000, 4), 8);
    }

    #[test]
    fn test_rom_loader_allows_only_logo_mismatch() {
        let mut rom = vec![0; 0x200];
        finalize_test_gba_rom(&mut rom);
        rom[0x61] ^= 0x07;
        assert!(GbaSystem::from_rom(rom.clone()).is_none());
        assert!(GbaSystem::from_test_rom(rom.clone()).is_some());

        rom[0xB2] = 0;
        assert!(GbaSystem::from_test_rom(rom).is_none());
    }
}
