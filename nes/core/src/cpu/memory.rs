use super::CpuCartridgeBus as Cartridge;
use crate::{
    Apu, ControllerHub, OpenBus, OpenBusReadResult, Ppu, cpu::Register, interrupt::Interrupt,
};

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub(crate) struct Memory {
    #[serde(with = "nerust_serialize::array::BigArray")]
    wram: [u8; 2048],
    openbus: OpenBus,
}

impl Memory {
    pub(crate) fn new() -> Self {
        Self {
            wram: [0; 2048],
            openbus: OpenBus::new(),
        }
    }

    pub(crate) fn read_next(
        &mut self,
        register: &mut Register,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
        interrupt: &mut Interrupt,
    ) -> u8 {
        let pc = register.get_pc();
        register.set_pc(pc.wrapping_add(1));
        self.read(pc as usize, ppu, cartridge, hub, apu, interrupt)
    }

    pub(crate) fn read(
        &mut self,
        address: usize,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
        interrupt: &mut Interrupt,
    ) -> u8 {
        let result = match address {
            0..=0x1FFF => OpenBusReadResult::new(self.wram[address & 0x07FF], 0xFF),
            0x2000..=0x3FFF => {
                let mut ppu_cartridge = crate::cartridge_bus::cpu_ppu_cartridge_bus(cartridge);
                ppu.read_register(address, &mut ppu_cartridge, interrupt)
            }
            0x4015 => apu.read_register(address, interrupt),
            0x4016 | 0x4017 => hub.read_port(address & 1),
            0x4000..=0x4014 | 0x4018..=0x401F => OpenBusReadResult::new(0, 0), // TODO: I/O registers
            0x4020..=0x5FFF => cartridge.read(address),
            0x6000..=0xFFFF => cartridge.read(address),
            _ => {
                log::error!("unhandled cpu memory read at address: 0x{:04X}", address);
                OpenBusReadResult::new(0, 0)
            }
        };
        let value = self.openbus.unite(result);
        cartridge.notify_cpu_read(address, value, interrupt);
        value
    }

    pub(crate) fn peek_work_ram(&self, address: usize) -> Option<u8> {
        match address {
            0..=0x1FFF => Some(self.wram[address & 0x07FF]),
            _ => None,
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "CPU bus reads need access to every attached device"
    )]
    pub(crate) fn read_dummy_cross(
        &mut self,
        address: usize,
        new_address: usize,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
        interrupt: &mut Interrupt,
    ) {
        let _ = self.read(
            (address & 0xFF00) | (new_address & 0xFF),
            ppu,
            cartridge,
            hub,
            apu,
            interrupt,
        );
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "CPU bus writes need access to every attached device"
    )]
    pub(crate) fn write(
        &mut self,
        address: usize,
        value: u8,
        ppu: &mut Ppu,
        cartridge: &mut dyn Cartridge,
        hub: &mut dyn ControllerHub,
        apu: &mut Apu,
        interrupt: &mut Interrupt,
    ) {
        match address {
            0..=0x1FFF => self.wram[address & 0x07FF] = value,
            0x2000..=0x3FFF => {
                let mut ppu_cartridge = crate::cartridge_bus::cpu_ppu_cartridge_bus(cartridge);
                ppu.write_register(address, value, &mut ppu_cartridge, interrupt)
            }
            0x4000..=0x4013 => apu.write_register(address, value, interrupt),
            0x4014 => {
                interrupt.oam_dma = Some(value);
                cartridge.notify_oam_dma(interrupt);
            }
            0x4015 => apu.write_register(address, value, interrupt),
            0x4016 => hub.write_strobe(value),
            0x4017 => apu.write_register(address, value, interrupt),
            0x4018..=0x401F => (), // TODO: I/O registers
            0x4020..=0x5FFF => cartridge.write(address, value, interrupt),
            0x6000..=0xFFFF => cartridge.write(address, value, interrupt),
            _ => {
                log::error!("unhandled cpu memory write at address: 0x{:04X}", address);
            }
        }
    }
}
