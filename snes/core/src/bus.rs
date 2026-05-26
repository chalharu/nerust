use crate::cartridge::ADDRESS_MASK;
use crate::{Cartridge, memory::Memory, ppu1::Ppu1, ppu2::Ppu2};

const CPU_IO_REGISTER_COUNT: usize = 0x20;
const DMA_REGISTER_COUNT: usize = 0x80;
const VBLANK_STUB_PERIOD: u8 = 11;
const VBLANK_STUB_ACTIVE_START: u8 = 5;

pub(crate) trait CpuBus {
    fn read(&mut self, addr: u32) -> u8;
    fn write(&mut self, addr: u32, data: u8);
    fn tick(&mut self) {}
}

pub(crate) struct Bus {
    cartridge: Cartridge,
    pub(crate) memory: Memory,
    pub(crate) ppu1: Ppu1,
    pub(crate) ppu2: Ppu2,
    cpu_io_registers: [u8; CPU_IO_REGISTER_COUNT],
    dma_registers: [u8; DMA_REGISTER_COUNT],
    video_phase: u8,
}

impl Bus {
    pub(crate) fn new(cartridge: Cartridge) -> Self {
        debug_assert_eq!(cartridge.mapper_kind(), crate::MapperKind::LoRom);

        Self {
            cartridge,
            memory: Memory::new(),
            ppu1: Ppu1::new(),
            ppu2: Ppu2::new(),
            cpu_io_registers: [0; CPU_IO_REGISTER_COUNT],
            dma_registers: [0; DMA_REGISTER_COUNT],
            video_phase: 0,
        }
    }

    pub(crate) fn cartridge(&self) -> &Cartridge {
        &self.cartridge
    }

    pub(crate) fn reset_ephemeral_state(&mut self) {
        self.video_phase = 0;
    }

    pub(crate) fn tick_video_stub(&mut self) {
        self.video_phase = (self.video_phase + 1) % VBLANK_STUB_PERIOD;
    }

    pub(crate) fn peek(&self, address: u32) -> u8 {
        self.peek_resolved(address & ADDRESS_MASK)
    }

    pub(crate) fn read(&mut self, address: u32) -> u8 {
        self.read_resolved(address & ADDRESS_MASK)
    }

    pub(crate) fn write(&mut self, address: u32, value: u8) {
        self.write_resolved(address & ADDRESS_MASK, value);
    }

    fn in_vblank(&self) -> bool {
        self.video_phase >= VBLANK_STUB_ACTIVE_START
    }

    fn read_resolved(&mut self, address: u32) -> u8 {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = (address & 0xFFFF) as u16;

        if let Some(value) = self.memory.read_cpu_bus(bank, offset) {
            return value;
        }

        match (bank, offset) {
            (0x00..=0x3F | 0x80..=0xBF, 0x2100..=0x213F) => self.read_ppu_register(offset),
            (0x00..=0x3F | 0x80..=0xBF, 0x2180..=0x2183) => {
                self.memory.read_mmio(offset).unwrap_or(0)
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4200..=0x421F) => self.read_cpu_io(offset),
            (0x00..=0x3F | 0x80..=0xBF, 0x4300..=0x437F) => {
                self.dma_registers[usize::from(offset - 0x4300)]
            }
            _ => self.cartridge.read(address).unwrap_or(0),
        }
    }

    fn peek_resolved(&self, address: u32) -> u8 {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = (address & 0xFFFF) as u16;

        if let Some(value) = self.memory.peek_cpu_bus(bank, offset) {
            return value;
        }

        match (bank, offset) {
            (0x00..=0x3F | 0x80..=0xBF, 0x2100..=0x213F) => self.peek_ppu_register(offset),
            (0x00..=0x3F | 0x80..=0xBF, 0x2180..=0x2183) => {
                self.memory.peek_mmio(offset).unwrap_or(0)
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4210) => {
                if self.in_vblank() {
                    0x80
                } else {
                    0x00
                }
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4212) => {
                if self.in_vblank() {
                    0x80
                } else {
                    0x00
                }
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4218) => self.cpu_io_registers[0x18],
            (0x00..=0x3F | 0x80..=0xBF, 0x4200..=0x421F) => {
                self.cpu_io_registers[usize::from(offset - 0x4200)]
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4300..=0x437F) => {
                self.dma_registers[usize::from(offset - 0x4300)]
            }
            _ => self.cartridge.read(address).unwrap_or(0),
        }
    }

    fn write_resolved(&mut self, address: u32, value: u8) {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = (address & 0xFFFF) as u16;

        if self.memory.write_cpu_bus(bank, offset, value) {
            return;
        }

        match (bank, offset) {
            (0x00..=0x3F | 0x80..=0xBF, 0x2100..=0x213F) => {
                let _ = self.write_ppu_register(offset, value);
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x2180..=0x2183) => {
                let _ = self.memory.write_mmio(offset, value);
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4200..=0x421F) => {
                self.cpu_io_registers[usize::from(offset - 0x4200)] = value;
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x4300..=0x437F) => {
                self.dma_registers[usize::from(offset - 0x4300)] = value;
            }
            _ => {}
        }
    }

    fn read_ppu_register(&mut self, offset: u16) -> u8 {
        self.ppu1
            .read(offset)
            .or_else(|| self.ppu2.read(offset))
            .unwrap_or(0)
    }

    fn peek_ppu_register(&self, offset: u16) -> u8 {
        self.ppu1
            .peek(offset)
            .or_else(|| self.ppu2.peek(offset))
            .unwrap_or(0)
    }

    fn write_ppu_register(&mut self, offset: u16, value: u8) -> bool {
        self.ppu1.write(offset, value) || self.ppu2.write(offset, value)
    }

    fn read_cpu_io(&mut self, offset: u16) -> u8 {
        match offset {
            0x4210 => {
                if self.in_vblank() {
                    0x80
                } else {
                    0x00
                }
            }
            0x4212 => {
                if self.in_vblank() {
                    0x80
                } else {
                    0x00
                }
            }
            0x4218 => self.cpu_io_registers[0x18],
            _ => self.cpu_io_registers[usize::from(offset - 0x4200)],
        }
    }
}

impl CpuBus for Bus {
    fn read(&mut self, addr: u32) -> u8 {
        Bus::read(self, addr)
    }

    fn write(&mut self, addr: u32, data: u8) {
        Bus::write(self, addr, data);
    }

    fn tick(&mut self) {
        self.tick_video_stub();
    }
}

#[cfg(test)]
mod tests {
    use super::{Bus, VBLANK_STUB_ACTIVE_START, VBLANK_STUB_PERIOD};
    use crate::Cartridge;

    const HEADER_OFFSET: usize = 0x7FC0;
    const RESET_VECTOR_OFFSET: usize = 0x7FFC;

    fn test_cartridge() -> Cartridge {
        let mut rom = vec![0; 0x8000];
        rom[HEADER_OFFSET..HEADER_OFFSET + 21].copy_from_slice(b"WRAM BUS TEST        ");
        rom[0x7FD5] = 0x30;
        rom[RESET_VECTOR_OFFSET..RESET_VECTOR_OFFSET + 2]
            .copy_from_slice(&0x8000_u16.to_le_bytes());
        Cartridge::from_bytes(&rom).unwrap()
    }

    #[test]
    fn low_ram_mirrors_and_full_wram_alias_each_other() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x000123, 0x5A);
        assert_eq!(bus.read(0x7E0123), 0x5A);

        bus.write(0x7E1ABC, 0xC3);
        assert_eq!(bus.read(0x001ABC), 0xC3);

        bus.write(0x7F0001, 0x99);
        assert_eq!(bus.read(0x7F0001), 0x99);
    }

    #[test]
    fn vblank_stub_allows_wait_loops_to_observe_both_edges() {
        let mut bus = Bus::new(test_cartridge());

        assert_eq!(bus.read(0x004210), 0x00);
        assert_eq!(bus.read(0x004212), 0x00);
        for _ in 0..VBLANK_STUB_ACTIVE_START {
            bus.tick_video_stub();
        }
        assert_eq!(bus.read(0x004210), 0x80);
        assert_eq!(bus.read(0x004212), 0x80);
        for _ in 0..(VBLANK_STUB_PERIOD - VBLANK_STUB_ACTIVE_START) {
            bus.tick_video_stub();
        }
        assert_eq!(bus.read(0x004210), 0x00);
        assert_eq!(bus.read(0x004218), 0x00);
    }
}
