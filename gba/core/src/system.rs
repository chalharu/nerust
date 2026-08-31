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
        let mut bus = GbaMemoryBus::new();
        bus.set_cartridge(cart);
        let mut cpu = GbaCpu::post_bios();
        cpu.reset(&mut bus);
        Some(Self {
            cpu,
            bus,
            tick: 0,
            cpu_cycles_remaining: 0,
        })
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

    /// CPUとバスを1 T-cycleだけ進行する。
    pub fn step_tcycle(&mut self) -> bool {
        if self.cpu_cycles_remaining == 0 {
            self.cpu_cycles_remaining = self.cpu.step(&mut self.bus).max(1);
        }
        self.cpu_cycles_remaining -= 1;
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

    #[test]
    fn step_tcycle_advances_exactly_one_cycle() {
        let mut system = GbaSystem::new();
        assert_eq!(system.tick, 0);
        system.step_tcycle();
        assert_eq!(system.tick, 1);
        assert!(system.cpu_cycles_remaining > 0);
    }
}
