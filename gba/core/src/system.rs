use crate::cartridge::Cartridge;
use crate::cpu::GbaCpu;
use crate::memory::GbaMemoryBus;

pub struct GbaSystem {
    pub cpu: GbaCpu,
    pub bus: GbaMemoryBus,
    tick: u64,
}

impl GbaSystem {
    pub fn new() -> Self {
        let mut cpu = GbaCpu::post_bios();
        let mut bus = GbaMemoryBus::new();
        cpu.reset(&mut bus);
        Self { cpu, bus, tick: 0 }
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
        Some(Self { cpu, bus, tick: 0 })
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

    /// 1 T-cycle 進行。CPUが消費したサイクル数だけ bus.tick() を進める。
    pub fn step_tcycle(&mut self) -> bool {
        let cycles = self.cpu.step(&mut self.bus);
        let mut frame_done = false;
        for _ in 0..cycles {
            self.tick = self.tick.wrapping_add(1);
            if self.bus.tick() {
                frame_done = true;
            }
        }
        frame_done
    }
}

impl Default for GbaSystem {
    fn default() -> Self {
        Self::new()
    }
}
