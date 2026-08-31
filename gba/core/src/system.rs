use crate::cartridge::Cartridge;
use crate::cpu::GbaCpu;
use crate::memory::GbaMemoryBus;

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
        if !self.bus.is_halted() && self.cpu_cycles_remaining == 0 {
            self.cpu_cycles_remaining = self.cpu.step(&mut self.bus).max(1);
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
    fn run_frame_advances_one_lcd_frame() {
        let mut system = GbaSystem::new();
        assert_eq!(
            system.run_frame().len(),
            crate::ppu::WIDTH * crate::ppu::HEIGHT
        );
        assert_eq!(system.tick, 280896);
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
