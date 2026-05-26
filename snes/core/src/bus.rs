use crate::cartridge::ADDRESS_MASK;
use crate::{Cartridge, memory::Memory, ppu1::Ppu1, ppu2::Ppu2};

const CPU_IO_REGISTER_COUNT: usize = 0x20;
const DMA_REGISTER_COUNT: usize = 0x80;
const VBLANK_STUB_PERIOD: u16 = 1024;
const VBLANK_STUB_ACTIVE_START: u16 = 768;

pub(crate) trait CpuBus {
    fn read(&mut self, addr: u32) -> u8;
    fn write(&mut self, addr: u32, data: u8);
    fn tick(&mut self) {}
    /// Returns `true` and clears the pending-NMI flag when an NMI is waiting
    /// for the CPU to service.  Returns `false` otherwise.
    fn poll_nmi(&mut self) -> bool {
        false
    }
}

pub(crate) struct Bus {
    cartridge: Cartridge,
    pub(crate) memory: Memory,
    pub(crate) ppu1: Ppu1,
    pub(crate) ppu2: Ppu2,
    cpu_io_registers: [u8; CPU_IO_REGISTER_COUNT],
    dma_registers: [u8; DMA_REGISTER_COUNT],
    video_phase: u16,
    /// RDNMI flag (bit 7 of $4210): set on vblank entry, cleared by reading $4210.
    nmi_flag: bool,
    /// Pending NMI for the CPU: set when the NMI flag rises while NMI is enabled
    /// in NMITIMEN (bit 7 of $4200), cleared when the CPU takes the interrupt.
    nmi_pending: bool,
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
            nmi_flag: false,
            nmi_pending: false,
        }
    }

    pub(crate) fn cartridge(&self) -> &Cartridge {
        &self.cartridge
    }

    pub(crate) fn reset_ephemeral_state(&mut self) {
        self.video_phase = 0;
        self.nmi_flag = false;
        self.nmi_pending = false;
    }

    pub(crate) fn tick_video_stub(&mut self) {
        let was_in_vblank = self.in_vblank();
        self.video_phase = (self.video_phase + 1) % VBLANK_STUB_PERIOD;
        // Rising edge of vblank: latch the NMI flag and optionally queue a
        // pending NMI for the CPU (when NMITIMEN bit 7 is set).
        if !was_in_vblank && self.in_vblank() {
            self.nmi_flag = true;
            if self.nmi_enabled() {
                self.nmi_pending = true;
            }
        }
    }

    fn nmi_enabled(&self) -> bool {
        // NMITIMEN ($4200) bit 7 enables VBlank NMI
        self.cpu_io_registers[0x00] & 0x80 != 0
    }

    /// Consume and return the pending-NMI flag.  Called by the CPU each cycle
    /// while in WAI state.
    pub(crate) fn poll_nmi(&mut self) -> bool {
        core::mem::take(&mut self.nmi_pending)
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
                if self.nmi_flag {
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
            // MDMAEN ($420B): store then execute selected DMA channels immediately.
            (0x00..=0x3F | 0x80..=0xBF, 0x420B) => {
                self.cpu_io_registers[usize::from(offset - 0x4200)] = value;
                if value != 0 {
                    self.execute_dma(value);
                    self.cpu_io_registers[usize::from(offset - 0x4200)] = 0;
                }
            }
            // NMITIMEN ($4200): track whether NMI is enabled; raise a pending NMI
            // immediately if the NMI flag is already latched (i.e. we are mid-vblank
            // and the program enables NMI after clearing RDNMI).
            (0x00..=0x3F | 0x80..=0xBF, 0x4200) => {
                let was_enabled = self.cpu_io_registers[0x00] & 0x80 != 0;
                self.cpu_io_registers[0x00] = value;
                let now_enabled = value & 0x80 != 0;
                if !was_enabled && now_enabled && self.nmi_flag {
                    self.nmi_pending = true;
                }
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

    /// Execute all DMA channels whose bit is set in `mdmaen`, lowest first.
    fn execute_dma(&mut self, mdmaen: u8) {
        for channel in 0..8u8 {
            if mdmaen & (1 << channel) != 0 {
                self.execute_dma_channel(channel);
            }
        }
    }

    fn dma_read_abus(&mut self, address: u32) -> u8 {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = (address & 0xFFFF) as u16;

        if !dma_abus_accessible(bank, offset) {
            return 0;
        }

        self.memory
            .read_cpu_bus(bank, offset)
            .or_else(|| self.cartridge.read(address))
            .unwrap_or(0)
    }

    fn dma_write_abus(&mut self, address: u32, value: u8) {
        let bank = ((address >> 16) & 0xFF) as u8;
        let offset = (address & 0xFFFF) as u16;

        if dma_abus_accessible(bank, offset) {
            let _ = self.memory.write_cpu_bus(bank, offset, value);
        }
    }

    /// Execute a single general-purpose DMA channel.
    ///
    /// Register layout per channel (base = channel * 0x10):
    ///   +0  DMAP  – bit7=direction(0=A→B), bits4:3=addr mode(00=inc, 01/11=fixed, 10=dec),
    ///               bits2-0=pattern
    ///   +1  BBAD  – B-bus address offset from $2100
    ///   +2  A1TL  – A-bus source address low
    ///   +3  A1TH  – A-bus source address high
    ///   +4  A1B   – A-bus source bank
    ///   +5  DASL  – byte count low  (0+0 ⇒ 65536)
    ///   +6  DASH  – byte count high
    fn execute_dma_channel(&mut self, channel: u8) {
        let base = usize::from(channel) * 0x10;

        let dmap = self.dma_registers[base];
        let bbad = self.dma_registers[base + 0x1];
        let a1t_lo = self.dma_registers[base + 0x2];
        let a1t_hi = self.dma_registers[base + 0x3];
        let a1b = self.dma_registers[base + 0x4];
        let das_lo = self.dma_registers[base + 0x5];
        let das_hi = self.dma_registers[base + 0x6];

        // DMAP decode
        let b_to_a = dmap & 0x80 != 0; // direction: 0=A→B (CPU→PPU), 1=B→A (PPU→CPU)
        let fixed = dmap & 0x08 != 0; // no A-bus address change
        let decrement = dmap & 0x10 != 0; // decrement A-bus address (only when !fixed)
        let pattern = (dmap & 0x07) as usize;

        // A-bus starting address (24-bit, bank does not wrap during transfer)
        let mut a_addr: u32 = (u32::from(a1b) << 16) | (u32::from(a1t_hi) << 8) | u32::from(a1t_lo);

        // Byte count: 0 means 65536
        let mut remaining: u32 = if das_lo == 0 && das_hi == 0 {
            0x10000
        } else {
            (u32::from(das_hi) << 8) | u32::from(das_lo)
        };

        // Per-pattern B-bus address offsets (cycled during transfer).
        // Patterns 6 and 7 are aliases of 2 and 3 respectively.
        let offsets: &[u8] = match pattern {
            0 => &[0],
            1 => &[0, 1],
            2 | 6 => &[0, 0],
            3 | 7 => &[0, 0, 1, 1],
            4 => &[0, 1, 2, 3],
            5 => &[0, 1, 0, 1],
            _ => &[0],
        };

        let mut pidx: usize = 0;

        while remaining > 0 {
            let b_addr = 0x2100 | u16::from(bbad.wrapping_add(offsets[pidx]));

            if b_to_a {
                let val = self.dma_read_bbus(b_addr);
                self.dma_write_abus(a_addr, val);
            } else {
                let val = self.dma_read_abus(a_addr);
                self.dma_write_bbus(b_addr, val);
            }

            if !fixed {
                // Keep transfer within the source bank
                let new_offset = if decrement {
                    (a_addr as u16).wrapping_sub(1)
                } else {
                    (a_addr as u16).wrapping_add(1)
                };
                a_addr = (a_addr & 0xFF_0000) | u32::from(new_offset);
            }

            pidx = (pidx + 1) % offsets.len();
            remaining -= 1;
        }

        // Write back updated A1T (bank is unchanged) and zero DAS.
        self.dma_registers[base + 0x2] = a_addr as u8;
        self.dma_registers[base + 0x3] = (a_addr >> 8) as u8;
        self.dma_registers[base + 0x5] = 0;
        self.dma_registers[base + 0x6] = 0;
    }

    /// Write one byte to the B-bus (PPU / WRAM-port / ignored).
    fn dma_write_bbus(&mut self, b_addr: u16, value: u8) {
        match b_addr {
            0x2100..=0x213F => {
                let _ = self.write_ppu_register(b_addr, value);
            }
            0x2180..=0x2183 => {
                let _ = self.memory.write_mmio(b_addr, value);
            }
            _ => {} // unknown B-bus address: silently discard
        }
    }

    /// Read one byte from the B-bus (PPU / WRAM-port / open-bus 0).
    fn dma_read_bbus(&mut self, b_addr: u16) -> u8 {
        match b_addr {
            0x2100..=0x213F => self.read_ppu_register(b_addr),
            0x2180..=0x2183 => self.memory.read_mmio(b_addr).unwrap_or(0),
            _ => 0,
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
            // RDNMI ($4210): returns NMI flag in bit 7 and clears it on read.
            0x4210 => {
                let val = if self.nmi_flag { 0x80 } else { 0x00 };
                self.nmi_flag = false;
                val
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

    fn poll_nmi(&mut self) -> bool {
        Bus::poll_nmi(self)
    }
}

fn dma_abus_accessible(bank: u8, offset: u16) -> bool {
    !matches!(
        (bank, offset),
        (
            0x00..=0x3F | 0x80..=0xBF,
            0x2100..=0x21FF | 0x4000..=0x41FF | 0x4200..=0x421F | 0x4300..=0x437F,
        )
    )
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

    // -----------------------------------------------------------------------
    // DMA tests
    // -----------------------------------------------------------------------

    /// Configure helpers: write a DMA channel's register block.
    ///
    /// `dmap`  – DMAP byte (bit7=dir, bits4:3=addr mode, bits2-0=pattern)
    /// `bbad`  – B-bus address offset from $2100
    /// `a_addr` – 24-bit A-bus source address
    /// `count` – DAS byte count (0 means 65536)
    fn setup_dma_channel(bus: &mut Bus, channel: u8, dmap: u8, bbad: u8, a_addr: u32, count: u16) {
        let base = 0x00_4300 + (u32::from(channel) * 0x10);
        bus.write(base, dmap);
        bus.write(base + 0x1, bbad);
        bus.write(base + 0x2, a_addr as u8);
        bus.write(base + 0x3, (a_addr >> 8) as u8);
        bus.write(base + 0x4, (a_addr >> 16) as u8);
        bus.write(base + 0x5, count as u8);
        bus.write(base + 0x6, (count >> 8) as u8);
    }

    fn setup_dma_ch0(bus: &mut Bus, dmap: u8, bbad: u8, a_addr: u32, count: u16) {
        setup_dma_channel(bus, 0, dmap, bbad, a_addr, count);
    }

    /// DMA ch0, pattern 1 (two-register: VMDATAL/VMDATAH), increment source.
    /// Transfers 4 bytes from WRAM[$7E:0100] to VRAM word 0 via $2118/$2119.
    /// Verifies VRAM contents, VMADD advanced, A1T updated, DAS zeroed.
    #[test]
    fn dma_pattern1_increment_writes_to_vram_and_updates_registers() {
        let mut bus = Bus::new(test_cartridge());

        // Place source data in WRAM
        bus.write(0x7E_0100, 0x11);
        bus.write(0x7E_0101, 0x22);
        bus.write(0x7E_0102, 0x33);
        bus.write(0x7E_0103, 0x44);

        // VMAIN = 0x80: increment after high-byte write, step = 1 word
        bus.write(0x00_2115, 0x80);
        // VMADD = 0
        bus.write(0x00_2116, 0x00);
        bus.write(0x00_2117, 0x00);

        // DMAP=0x01: A→B, increment, pattern 1 (+0,+1)
        // BBAD=0x18: VMDATA ($2118)
        setup_dma_ch0(&mut bus, 0x01, 0x18, 0x7E_0100, 4);

        // Trigger MDMAEN – channel 0
        bus.write(0x00_420B, 0x01);

        // VRAM word 0 (bytes 0-1) and word 1 (bytes 2-3)
        assert_eq!(bus.ppu1.peek_vram(0), 0x11, "VRAM[0] low");
        assert_eq!(bus.ppu1.peek_vram(1), 0x22, "VRAM[0] high");
        assert_eq!(bus.ppu1.peek_vram(2), 0x33, "VRAM[1] low");
        assert_eq!(bus.ppu1.peek_vram(3), 0x44, "VRAM[1] high");

        // VMADD incremented once per word → 2 words transferred
        assert_eq!(bus.ppu1.vmadd(), 2, "VMADD after DMA");

        // A1T updated to 0x0104 (started 0x0100, incremented 4 times)
        assert_eq!(bus.read(0x00_4302), 0x04, "A1TL post-DMA");
        assert_eq!(bus.read(0x00_4303), 0x01, "A1TH post-DMA");
        // A1B unchanged
        assert_eq!(bus.read(0x00_4304), 0x7E, "A1B unchanged");
        // DAS zeroed
        assert_eq!(bus.read(0x00_4305), 0x00, "DASL zeroed");
        assert_eq!(bus.read(0x00_4306), 0x00, "DASH zeroed");
    }

    /// DMA ch0, pattern 0, fixed source → WRAM port ($2180).
    /// One byte repeated into 4 consecutive WRAM locations; WMADD advances.
    #[test]
    fn dma_fixed_source_pattern0_to_wram_port_repeats_byte() {
        let mut bus = Bus::new(test_cartridge());

        // Source byte in WRAM
        bus.write(0x7E_0200, 0x42);

        // WMADD = 0
        bus.write(0x00_2181, 0x00);
        bus.write(0x00_2182, 0x00);
        bus.write(0x00_2183, 0x00);

        // DMAP=0x08: A→B, fixed, pattern 0
        // BBAD=0x80: WMDATA ($2180)
        setup_dma_ch0(&mut bus, 0x08, 0x80, 0x7E_0200, 4);

        bus.write(0x00_420B, 0x01);

        // Each of the 4 WRAM bytes should be the repeated source value
        assert_eq!(bus.memory.peek_wram(0), 0x42, "WRAM[0]");
        assert_eq!(bus.memory.peek_wram(1), 0x42, "WRAM[1]");
        assert_eq!(bus.memory.peek_wram(2), 0x42, "WRAM[2]");
        assert_eq!(bus.memory.peek_wram(3), 0x42, "WRAM[3]");

        // WMADD advanced 4 times
        assert_eq!(bus.memory.wmadd(), 4, "WMADD after fixed DMA");

        // A1T unchanged (fixed transfer)
        assert_eq!(bus.read(0x00_4302), 0x00, "A1TL fixed unchanged");
        assert_eq!(bus.read(0x00_4303), 0x02, "A1TH fixed unchanged");
        // DAS zeroed
        assert_eq!(bus.read(0x00_4305), 0x00, "DASL zeroed");
        assert_eq!(bus.read(0x00_4306), 0x00, "DASH zeroed");
    }

    /// DMA ch0, pattern 0, increment source → CGDATA ($2122).
    /// Writing 2 bytes commits one CGRAM color entry.
    #[test]
    fn dma_pattern0_increment_writes_to_cgram() {
        let mut bus = Bus::new(test_cartridge());

        // Source: two palette bytes
        bus.write(0x7E_0300, 0xAB);
        bus.write(0x7E_0301, 0x5C);

        // CGADD = color 0
        bus.write(0x00_2121, 0x00);

        // DMAP=0x00: A→B, increment, pattern 0 (single register)
        // BBAD=0x22: CGDATA ($2122)
        setup_dma_ch0(&mut bus, 0x00, 0x22, 0x7E_0300, 2);

        bus.write(0x00_420B, 0x01);

        // First write latches low byte; second write commits the pair
        assert_eq!(bus.ppu2.peek_cgram(0), 0xAB, "CGRAM color0 low");
        assert_eq!(bus.ppu2.peek_cgram(1), 0x5C, "CGRAM color0 high");

        // A1T updated to 0x0302
        assert_eq!(bus.read(0x00_4302), 0x02, "A1TL post-CGRAM DMA");
        assert_eq!(bus.read(0x00_4303), 0x03, "A1TH post-CGRAM DMA");
        // DAS zeroed
        assert_eq!(bus.read(0x00_4305), 0x00);
        assert_eq!(bus.read(0x00_4306), 0x00);
    }

    /// MDMAEN=0 must not touch any DMA channel.
    #[test]
    fn mdmaen_zero_does_not_execute_any_channel() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x7E_0000, 0xFF);

        // Configure ch0 to write to WRAM port but do NOT trigger
        bus.write(0x00_2181, 0x10);
        bus.write(0x00_2182, 0x00);
        bus.write(0x00_2183, 0x00);
        setup_dma_ch0(&mut bus, 0x08, 0x80, 0x7E_0000, 8);

        bus.write(0x00_420B, 0x00); // trigger with no channels set

        // WRAM at $10 must be untouched
        assert_eq!(bus.memory.peek_wram(0x10), 0x00, "WRAM untouched");
        // WMADD stays at 0x10
        assert_eq!(bus.memory.wmadd(), 0x10, "WMADD untouched");
    }

    #[test]
    fn dma_pattern4_wraps_bbus_address_within_21xx_page() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x7E_0500, 0xAA);
        bus.write(0x7E_0501, 0xBB);
        bus.write(0x7E_0502, 0xCC);
        bus.write(0x7E_0503, 0xDD);

        setup_dma_ch0(&mut bus, 0x04, 0xFF, 0x7E_0500, 4);
        bus.write(0x00_420B, 0x01);

        assert_eq!(bus.ppu2.inidisp(), 0xBB, "wrapped write reaches $2100");
        assert_eq!(
            bus.ppu1.peek(0x2101),
            Some(0xCC),
            "wrapped write reaches $2101"
        );
        assert_eq!(
            bus.ppu1.peek(0x2102),
            Some(0xDD),
            "wrapped write reaches $2102"
        );
    }

    #[test]
    fn dma_b_to_a_ignores_abus_mmio_destinations() {
        let mut bus = Bus::new(test_cartridge());

        bus.write(0x00_2121, 0x00);
        bus.write(0x00_2122, 0x02);
        bus.write(0x00_2122, 0x00);
        bus.write(0x00_2121, 0x00);

        bus.write(0x7E_0400, 0x99);
        bus.write(0x00_2181, 0x20);
        bus.write(0x00_2182, 0x00);
        bus.write(0x00_2183, 0x00);

        setup_dma_ch0(&mut bus, 0x80, 0x3B, 0x00_420B, 1);
        setup_dma_channel(&mut bus, 1, 0x00, 0x80, 0x7E_0400, 1);

        bus.write(0x00_420B, 0x01);

        assert_eq!(bus.read(0x00_420B), 0x00, "MDMAEN self-clears after DMA");
        assert_eq!(
            bus.memory.peek_wram(0x20),
            0x00,
            "channel 1 was not spuriously triggered"
        );
    }

    // -----------------------------------------------------------------------
    // NMI / RDNMI tests
    // -----------------------------------------------------------------------

    #[test]
    fn rdnmi_flag_is_set_on_vblank_entry_and_cleared_by_read() {
        let mut bus = Bus::new(test_cartridge());

        // No vblank yet: RDNMI reads 0x00 and flag stays clear
        assert_eq!(bus.read(0x004210), 0x00);
        assert!(!bus.nmi_flag);

        // Tick until vblank starts
        for _ in 0..VBLANK_STUB_ACTIVE_START {
            bus.tick_video_stub();
        }
        assert!(bus.nmi_flag, "nmi_flag should be set on vblank entry");

        // First read returns 0x80 and clears the flag
        assert_eq!(bus.read(0x004210), 0x80);
        assert!(!bus.nmi_flag, "nmi_flag should be cleared after read");

        // Second read returns 0x00 (flag already cleared)
        assert_eq!(bus.read(0x004210), 0x00);
    }

    #[test]
    fn nmi_pending_is_raised_when_vblank_starts_while_nmi_enabled() {
        let mut bus = Bus::new(test_cartridge());

        // Enable NMI via NMITIMEN ($4200 bit 7)
        bus.write(0x004200, 0x80);
        assert!(!bus.nmi_pending);

        // Tick into vblank
        for _ in 0..VBLANK_STUB_ACTIVE_START {
            bus.tick_video_stub();
        }
        assert!(
            bus.nmi_pending,
            "nmi_pending should be set when NMI is enabled at vblank"
        );

        // poll_nmi consumes the pending flag
        assert!(bus.poll_nmi());
        assert!(!bus.nmi_pending);
        assert!(!bus.poll_nmi(), "second poll should return false");
    }

    #[test]
    fn nmi_not_pending_when_nmi_disabled_at_vblank() {
        let mut bus = Bus::new(test_cartridge());

        // NMI disabled (NMITIMEN bit 7 = 0, default)
        for _ in 0..VBLANK_STUB_ACTIVE_START {
            bus.tick_video_stub();
        }
        assert!(bus.nmi_flag);
        assert!(
            !bus.nmi_pending,
            "nmi_pending should NOT be set when NMI is disabled"
        );
    }

    #[test]
    fn enabling_nmi_while_nmi_flag_is_set_raises_pending_nmi() {
        let mut bus = Bus::new(test_cartridge());

        // Tick into vblank without NMI enabled
        for _ in 0..VBLANK_STUB_ACTIVE_START {
            bus.tick_video_stub();
        }
        assert!(bus.nmi_flag);
        assert!(!bus.nmi_pending);

        // Now enable NMI – should immediately queue pending NMI
        bus.write(0x004200, 0x80);
        assert!(
            bus.nmi_pending,
            "enabling NMI mid-vblank should queue pending NMI"
        );
    }

    #[test]
    fn rdnmi_peek_reflects_nmi_flag_without_clearing() {
        let mut bus = Bus::new(test_cartridge());

        for _ in 0..VBLANK_STUB_ACTIVE_START {
            bus.tick_video_stub();
        }
        assert!(bus.nmi_flag);

        // Peek is non-destructive
        assert_eq!(bus.peek(0x004210), 0x80);
        assert!(bus.nmi_flag, "peek must not clear the NMI flag");
        assert_eq!(bus.peek(0x004210), 0x80);
    }
}
