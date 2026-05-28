use crate::mapper::{MapperKind, lorom_rom_index};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnhancementChip {
    None,
    Sa1,
    SuperFxGsu1,
    SuperFxGsu2,
    Cx4,
    Dsp1Family,
}

impl EnhancementChip {
    pub(crate) fn is_superfx(self) -> bool {
        matches!(self, Self::SuperFxGsu1 | Self::SuperFxGsu2)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EnhancementState {
    None,
    Sa1(Sa1State),
    SuperFx(SuperFxState),
    Cx4(Cx4State),
    Dsp1(Dsp1State),
}

impl EnhancementState {
    pub(crate) fn from_chip(chip: EnhancementChip) -> Self {
        match chip {
            EnhancementChip::None => Self::None,
            EnhancementChip::Sa1 => Self::Sa1(Sa1State::new()),
            EnhancementChip::SuperFxGsu1 | EnhancementChip::SuperFxGsu2 => {
                Self::SuperFx(SuperFxState::new())
            }
            EnhancementChip::Cx4 => Self::Cx4(Cx4State::new()),
            EnhancementChip::Dsp1Family => Self::Dsp1(Dsp1State::new()),
        }
    }

    pub(crate) fn peek(
        &self,
        mapper_kind: MapperKind,
        address: u32,
        rom: &[u8],
        save_ram: &[u8],
    ) -> Option<u8> {
        match self {
            Self::None => None,
            Self::Sa1(state) => state.peek(address, rom, save_ram),
            Self::SuperFx(state) => state.read(address),
            Self::Cx4(state) => state.read(address),
            Self::Dsp1(state) => state.peek(mapper_kind, address),
        }
    }

    pub(crate) fn read(
        &mut self,
        mapper_kind: MapperKind,
        address: u32,
        rom: &[u8],
        save_ram: &[u8],
    ) -> Option<u8> {
        match self {
            Self::None => None,
            Self::Sa1(state) => state.read(address, rom, save_ram),
            Self::SuperFx(state) => state.read(address),
            Self::Cx4(state) => state.read(address),
            Self::Dsp1(state) => state.read(mapper_kind, address),
        }
    }

    pub(crate) fn write(
        &mut self,
        mapper_kind: MapperKind,
        address: u32,
        value: u8,
        rom: &[u8],
        save_ram: &mut [u8],
    ) -> bool {
        match self {
            Self::None => false,
            Self::Sa1(state) => state.write(address, value, rom, save_ram),
            Self::SuperFx(state) => state.write(address, value, save_ram),
            Self::Cx4(state) => state.write(address, value, rom),
            Self::Dsp1(state) => state.write(mapper_kind, address, value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sa1State {
    registers: ByteWindow,
    iram: ByteWindow,
    cbmode: bool,
    cb: u8,
    dbmode: bool,
    db: u8,
    ebmode: bool,
    eb: u8,
    fbmode: bool,
    fb: u8,
    sbm: u8,
    swen: bool,
    cwen: bool,
    bwp: u8,
    siwp: u8,
    ciwp: u8,
    vbr_auto_increment: bool,
    vbr_shift: u8,
    vbr_bits: u8,
    vbr_address: u32,
    dma_enabled: bool,
    dma_char_conversion: bool,
    dma_char_conversion_target: bool,
    dma_dest_bwram: bool,
    dma_source_device: u8,
    dma_conversion_size: u8,
    dma_conversion_depth: u8,
    dma_source_address: u32,
    dma_dest_address: u32,
    dma_length: u16,
    dma_line: u8,
    dma_bwram_conversion_active: bool,
    bitmap_register_file: [u8; 16],
    arithmetic_acm: bool,
    arithmetic_md: bool,
    ma: u16,
    mb: u16,
    mr: u64,
    arithmetic_overflow: bool,
}

const SA1_CXB: u16 = 0x2220;
const SA1_DXB: u16 = 0x2221;
const SA1_EXB: u16 = 0x2222;
const SA1_FXB: u16 = 0x2223;
const SA1_BMAPS: u16 = 0x2224;
const SA1_SBWE: u16 = 0x2226;
const SA1_CBWE: u16 = 0x2227;
const SA1_BWPA: u16 = 0x2228;
const SA1_SIWP: u16 = 0x2229;
const SA1_CIWP: u16 = 0x222A;
const SA1_MCNT: u16 = 0x2250;
const SA1_MAL: u16 = 0x2251;
const SA1_MAH: u16 = 0x2252;
const SA1_MBL: u16 = 0x2253;
const SA1_MBH: u16 = 0x2254;
const SA1_VBD: u16 = 0x2258;
const SA1_VDAL: u16 = 0x2259;
const SA1_VDAM: u16 = 0x225A;
const SA1_VDAH: u16 = 0x225B;
const SA1_DCNT: u16 = 0x2230;
const SA1_CDMA: u16 = 0x2231;
const SA1_DSAL: u16 = 0x2232;
const SA1_DSAM: u16 = 0x2233;
const SA1_DSAH: u16 = 0x2234;
const SA1_DDAL: u16 = 0x2235;
const SA1_DDAM: u16 = 0x2236;
const SA1_DDAH: u16 = 0x2237;
const SA1_DTCL: u16 = 0x2238;
const SA1_DTCH: u16 = 0x2239;
const SA1_BRF0: u16 = 0x2240;
const SA1_BRF7: u16 = 0x2247;
const SA1_BRF15: u16 = 0x224F;
const SA1_MR0: u16 = 0x2306;
const SA1_OF: u16 = 0x230B;
const SA1_VDPL: u16 = 0x230C;
const SA1_VDPH: u16 = 0x230D;
const SA1_MR_MASK: u64 = (1 << 40) - 1;

impl Sa1State {
    fn new() -> Self {
        let mut registers = ByteWindow::new(0x2200, 0x0200);
        registers.write(SA1_DXB, 0x01);
        registers.write(SA1_EXB, 0x02);
        registers.write(SA1_FXB, 0x03);
        registers.write(SA1_BWPA, 0x0F);

        Self {
            registers,
            iram: ByteWindow::new(0x3000, 0x0800),
            cbmode: false,
            cb: 0,
            dbmode: false,
            db: 1,
            ebmode: false,
            eb: 2,
            fbmode: false,
            fb: 3,
            sbm: 0,
            swen: false,
            cwen: false,
            bwp: 0x0F,
            siwp: 0,
            ciwp: 0,
            vbr_auto_increment: false,
            vbr_shift: 16,
            vbr_bits: 0,
            vbr_address: 0,
            dma_enabled: false,
            dma_char_conversion: false,
            dma_char_conversion_target: false,
            dma_dest_bwram: false,
            dma_source_device: 0,
            dma_conversion_size: 0,
            dma_conversion_depth: 0,
            dma_source_address: 0,
            dma_dest_address: 0,
            dma_length: 0,
            dma_line: 0,
            dma_bwram_conversion_active: false,
            bitmap_register_file: [0; 16],
            arithmetic_acm: false,
            arithmetic_md: false,
            ma: 0,
            mb: 0,
            mr: 0,
            arithmetic_overflow: false,
        }
    }

    fn peek(&self, address: u32, rom: &[u8], save_ram: &[u8]) -> Option<u8> {
        if !is_system_bank(address) {
            return None;
        }

        let address_offset = offset(address);
        if let Some(value) = self.read_arithmetic(address_offset) {
            return Some(value);
        }
        if let Some(value) = self.peek_variable_data(address_offset, rom, save_ram) {
            return Some(value);
        }

        self.registers
            .read(address_offset)
            .or_else(|| self.iram.read(address_offset))
    }

    fn read(&mut self, address: u32, rom: &[u8], save_ram: &[u8]) -> Option<u8> {
        let value = self.peek(address, rom, save_ram)?;
        if is_system_bank(address) && offset(address) == SA1_VDPH && self.vbr_auto_increment {
            self.increment_variable_data_address();
        }
        Some(value)
    }

    fn write(&mut self, address: u32, value: u8, rom: &[u8], save_ram: &mut [u8]) -> bool {
        if !is_system_bank(address) {
            return false;
        }

        let address_offset = offset(address);
        if self.write_arithmetic(address_offset, value) {
            return true;
        }

        if self.registers.write(address_offset, value) {
            self.write_mapper_register(address_offset, value, rom, save_ram);
            return true;
        }

        self.write_iram(address_offset, value)
    }

    pub(crate) fn sa1_banked_rom_index(&self, address: u32, rom_len: usize) -> Option<usize> {
        if rom_len == 0 {
            return None;
        }

        let bank = bank(address);
        let offset = offset(address);
        let (slot, slot_offset, lo_access) = match bank {
            0xC0..=0xFF => {
                let slot = usize::from((bank - 0xC0) >> 4);
                let slot_offset = usize::from(bank & 0x0F) * 0x10000 + usize::from(offset);
                (slot, slot_offset, false)
            }
            0x00..=0x3F | 0x80..=0xBF if offset >= 0x8000 => {
                let mirror_bank = bank & 0x3F;
                let slot = usize::from(mirror_bank >> 5);
                let slot_offset =
                    usize::from(mirror_bank & 0x1F) * 0x8000 + usize::from(offset - 0x8000);
                (slot, slot_offset, true)
            }
            _ => return None,
        };

        let (xmode, selected_bank) = match slot {
            0 => (self.cbmode, self.cb),
            1 => (self.dbmode, self.db),
            2 => (self.ebmode, self.eb),
            3 => (self.fbmode, self.fb),
            _ => unreachable!("SA-1 Super MMC slot is constrained to 0..=3"),
        };
        let base = if lo_access && !xmode {
            slot * 0x100000
        } else {
            usize::from(selected_bank) * 0x100000
        };

        Some((base + slot_offset) % rom_len)
    }

    pub(crate) fn sa1_bwram_index(&self, address: u32, ram_len: usize) -> Option<usize> {
        if ram_len == 0 {
            return None;
        }

        let linear = self.sa1_bwram_linear_address(address)?;

        Some(linear % ram_len)
    }

    pub(crate) fn read_sa1_bwram(&mut self, address: u32, save_ram: &[u8]) -> Option<u8> {
        let linear = self.sa1_bwram_linear_address(address)?;
        if save_ram.is_empty() {
            return None;
        }
        if self.dma_bwram_conversion_active {
            return Some(self.read_character_conversion_type1(linear, save_ram));
        }

        Some(save_ram[linear % save_ram.len()])
    }

    pub(crate) fn can_write_sa1_bwram(&self, address: u32) -> bool {
        let Some(linear) = self.sa1_bwram_linear_address(address) else {
            return false;
        };
        // BWPA is checked against the 256 KiB SA-1 BWRAM address space before SRAM mirroring.
        let protection_address = linear & 0x3FFFF;
        self.swen || self.cwen || protection_address >= (0x100usize << self.bwp)
    }

    fn sa1_bwram_linear_address(&self, address: u32) -> Option<usize> {
        let bank = bank(address);
        let offset = offset(address);
        match bank {
            0x00..=0x3F | 0x80..=0xBF if (0x6000..=0x7FFF).contains(&offset) => {
                Some(usize::from(self.sbm) * 0x2000 + usize::from(offset - 0x6000))
            }
            0x40..=0x4F => Some(usize::from(bank & 0x0F) * 0x10000 + usize::from(offset)),
            _ => None,
        }
    }

    fn write_iram(&mut self, address_offset: u16, value: u8) -> bool {
        let Some(index) = self.iram.index(address_offset) else {
            return false;
        };
        let page = (index >> 8) & 0x07;
        if self.siwp & (1u8 << page) != 0 {
            self.iram.bytes[index] = value;
        }
        true
    }

    fn write_mapper_register(
        &mut self,
        address_offset: u16,
        value: u8,
        rom: &[u8],
        save_ram: &mut [u8],
    ) {
        match address_offset {
            SA1_CXB => {
                self.cbmode = value & 0x80 != 0;
                self.cb = value & 0x07;
            }
            SA1_DXB => {
                self.dbmode = value & 0x80 != 0;
                self.db = value & 0x07;
            }
            SA1_EXB => {
                self.ebmode = value & 0x80 != 0;
                self.eb = value & 0x07;
            }
            SA1_FXB => {
                self.fbmode = value & 0x80 != 0;
                self.fb = value & 0x07;
            }
            SA1_BMAPS => self.sbm = value & 0x1F,
            SA1_SBWE => self.swen = value & 0x80 != 0,
            SA1_CBWE => self.cwen = value & 0x80 != 0,
            SA1_BWPA => self.bwp = value & 0x0F,
            SA1_SIWP => self.siwp = value,
            SA1_CIWP => self.ciwp = value,
            SA1_VBD => {
                self.vbr_auto_increment = value & 0x80 != 0;
                self.vbr_shift = value & 0x0F;
                if self.vbr_shift == 0 {
                    self.vbr_shift = 16;
                }
                if !self.vbr_auto_increment {
                    self.increment_variable_data_address();
                }
            }
            SA1_VDAL => self.vbr_address = (self.vbr_address & 0xFFFF00) | u32::from(value),
            SA1_VDAM => {
                self.vbr_address = (self.vbr_address & 0xFF00FF) | (u32::from(value) << 8);
            }
            SA1_VDAH => {
                self.vbr_address = (self.vbr_address & 0x00FFFF) | (u32::from(value) << 16);
                self.vbr_bits = 0;
            }
            SA1_DCNT => {
                self.dma_enabled = value & 0x80 != 0;
                self.dma_char_conversion = value & 0x20 != 0;
                self.dma_char_conversion_target = value & 0x10 != 0;
                self.dma_dest_bwram = value & 0x04 != 0;
                self.dma_source_device = value & 0x03;
            }
            SA1_CDMA => {
                if value & 0x80 != 0 {
                    self.dma_bwram_conversion_active = false;
                }
                self.dma_conversion_size = ((value >> 2) & 0x07).min(5);
                self.dma_conversion_depth = (value & 0x03).min(2);
            }
            SA1_DSAL => {
                self.dma_source_address = (self.dma_source_address & 0xFFFF00) | u32::from(value);
            }
            SA1_DSAM => {
                self.dma_source_address =
                    (self.dma_source_address & 0xFF00FF) | (u32::from(value) << 8);
            }
            SA1_DSAH => {
                self.dma_source_address =
                    (self.dma_source_address & 0x00FFFF) | (u32::from(value) << 16);
            }
            SA1_DDAL => {
                self.dma_dest_address = (self.dma_dest_address & 0xFFFF00) | u32::from(value);
            }
            SA1_DDAM => {
                self.dma_dest_address =
                    (self.dma_dest_address & 0xFF00FF) | (u32::from(value) << 8);
                // IRAM destinations trigger after the middle DDA byte; BWRAM waits for DDAH.
                if !self.dma_dest_bwram {
                    if self.dma_char_conversion && self.dma_char_conversion_target {
                        self.dma_bwram_conversion_active = self.dma_enabled;
                    } else {
                        self.execute_normal_dma(rom, save_ram);
                    }
                }
            }
            SA1_DDAH => {
                self.dma_dest_address =
                    (self.dma_dest_address & 0x00FFFF) | (u32::from(value) << 16);
                if self.dma_dest_bwram {
                    self.execute_normal_dma(rom, save_ram);
                }
            }
            SA1_DTCL => self.dma_length = (self.dma_length & 0xFF00) | u16::from(value),
            SA1_DTCH => self.dma_length = (self.dma_length & 0x00FF) | (u16::from(value) << 8),
            SA1_BRF0..=SA1_BRF15 => {
                self.bitmap_register_file[usize::from(address_offset - SA1_BRF0)] = value;
                if matches!(address_offset, SA1_BRF7 | SA1_BRF15) {
                    self.execute_character_conversion_type2();
                }
            }
            _ => {}
        }
    }

    fn peek_variable_data(&self, address_offset: u16, rom: &[u8], save_ram: &[u8]) -> Option<u8> {
        if !matches!(address_offset, SA1_VDPL | SA1_VDPH) {
            return None;
        }

        let data = u32::from(self.read_variable_data_byte(self.vbr_address, rom, save_ram))
            | (u32::from(self.read_variable_data_byte(
                self.vbr_address.wrapping_add(1),
                rom,
                save_ram,
            )) << 8)
            | (u32::from(self.read_variable_data_byte(
                self.vbr_address.wrapping_add(2),
                rom,
                save_ram,
            )) << 16);
        let shifted = data >> self.vbr_bits;
        Some(if address_offset == SA1_VDPL {
            shifted as u8
        } else {
            (shifted >> 8) as u8
        })
    }

    fn read_variable_data_byte(&self, address: u32, rom: &[u8], save_ram: &[u8]) -> u8 {
        let address = address & 0x00FF_FFFF;
        if is_sa1_rom_address(address) {
            return self
                .sa1_cpu_banked_rom_index(address, rom.len())
                .map(|index| rom[index])
                .unwrap_or(0xFF);
        }
        if !save_ram.is_empty()
            && ((address & 0x40E000) == 0x006000 || (address & 0xF00000) == 0x400000)
        {
            return save_ram[address as usize % save_ram.len()];
        }
        if (address & 0x40F800) == 0x000000 || (address & 0x40F800) == 0x003000 {
            return self.iram.bytes[address as usize & 0x07FF];
        }
        0xFF
    }

    fn increment_variable_data_address(&mut self) {
        let bits = self.vbr_bits + self.vbr_shift;
        self.vbr_address = self.vbr_address.wrapping_add(u32::from(bits >> 3)) & 0x00FF_FFFF;
        self.vbr_bits = bits & 0x07;
    }

    fn sa1_cpu_banked_rom_index(&self, address: u32, rom_len: usize) -> Option<usize> {
        if rom_len == 0 {
            return None;
        }

        let address = address & 0x00FF_FFFF;
        let translated = if (address & 0x408000) == 0x008000 {
            ((address & 0x800000) >> 2) | ((address & 0x3F0000) >> 1) | (address & 0x007FFF)
        } else {
            address
        };
        let lo_access = translated < 0x400000;
        let normalized = translated & 0x3FFFFF;
        let slot = (normalized >> 20) as usize;
        let slot_offset = normalized as usize & 0x0F_FFFF;
        let (xmode, selected_bank) = match slot {
            0 => (self.cbmode, self.cb),
            1 => (self.dbmode, self.db),
            2 => (self.ebmode, self.eb),
            3 => (self.fbmode, self.fb),
            _ => return None,
        };
        let base = if lo_access && !xmode {
            slot * 0x100000
        } else {
            usize::from(selected_bank) * 0x100000
        };
        Some((base + slot_offset) % rom_len)
    }

    fn execute_normal_dma(&mut self, rom: &[u8], save_ram: &mut [u8]) {
        if !self.dma_enabled || self.dma_char_conversion {
            return;
        }

        for offset in 0..u32::from(self.dma_length) {
            let source = self.dma_source_address.wrapping_add(offset) & 0x00FF_FFFF;
            let target = self.dma_dest_address.wrapping_add(offset) & 0x00FF_FFFF;
            let value = self.read_dma_source(source, rom, save_ram);
            if self.dma_dest_bwram {
                if !save_ram.is_empty() {
                    save_ram[target as usize % save_ram.len()] = value;
                }
            } else {
                self.iram.bytes[target as usize & 0x07FF] = value;
            }
        }

        self.dma_source_address = self
            .dma_source_address
            .wrapping_add(u32::from(self.dma_length))
            & 0x00FF_FFFF;
        self.dma_dest_address = self
            .dma_dest_address
            .wrapping_add(u32::from(self.dma_length))
            & 0x00FF_FFFF;
        self.dma_length = 0;
    }

    fn read_dma_source(&self, address: u32, rom: &[u8], save_ram: &[u8]) -> u8 {
        match self.dma_source_device {
            0 => self
                .sa1_cpu_banked_rom_index(address, rom.len())
                .map(|index| rom[index])
                .unwrap_or(0xFF),
            1 => {
                if save_ram.is_empty() {
                    0xFF
                } else {
                    save_ram[address as usize % save_ram.len()]
                }
            }
            2 => self.iram.bytes[address as usize & 0x07FF],
            _ => 0xFF,
        }
    }

    fn execute_character_conversion_type2(&mut self) {
        if !self.dma_enabled || !self.dma_char_conversion || self.dma_char_conversion_target {
            return;
        }

        let bytes_per_row = 2usize << (2 - self.dma_conversion_depth);
        let mut target = self.dma_dest_address as usize & 0x07FF;
        target &= !((1usize << (7 - self.dma_conversion_depth)) - 1);
        target += usize::from(self.dma_line & 0x08) * bytes_per_row;
        target += usize::from(self.dma_line & 0x07) * 2;

        let source_offset = usize::from(self.dma_line & 0x01) * 8;
        for byte_index in 0..bytes_per_row {
            let mut output = 0;
            for bit_index in 0..8 {
                let bit = (self.bitmap_register_file[source_offset + bit_index] >> byte_index) & 1;
                output |= bit << (7 - bit_index);
            }
            let plane_offset = ((byte_index & 0x06) << 3) + (byte_index & 0x01);
            self.iram.bytes[(target + plane_offset) & 0x07FF] = output;
        }

        self.dma_line = self.dma_line.wrapping_add(1) & 0x0F;
    }

    fn read_character_conversion_type1(&mut self, address: usize, save_ram: &[u8]) -> u8 {
        let character_mask = (1usize << (6 - self.dma_conversion_depth)) - 1;
        if address & character_mask == 0 {
            self.buffer_character_conversion_type1(address, save_ram);
        }

        let iram_index = (self.dma_dest_address as usize + (address & character_mask)) & 0x07FF;
        self.iram.bytes[iram_index]
    }

    fn buffer_character_conversion_type1(&mut self, address: usize, save_ram: &[u8]) {
        let bytes_per_row = 2usize << (2 - self.dma_conversion_depth);
        let bytes_per_line = (8usize << self.dma_conversion_size) >> self.dma_conversion_depth;
        let bwram_mask = save_ram.len() - 1;
        let tile = (address.wrapping_sub(self.dma_source_address as usize) & bwram_mask)
            >> (6 - self.dma_conversion_depth);
        let tile_y = tile >> self.dma_conversion_size;
        let tile_x = tile & ((1usize << self.dma_conversion_size) - 1);
        let mut bwram_address =
            self.dma_source_address as usize + tile_y * 8 * bytes_per_line + tile_x * bytes_per_row;

        for row in 0..8 {
            let mut data = 0u64;
            for byte_index in 0..bytes_per_row {
                data |= u64::from(save_ram[(bwram_address + byte_index) & bwram_mask])
                    << (byte_index * 8);
            }
            bwram_address += bytes_per_line;

            let mut output = [0u8; 8];
            for pixel in 0..8 {
                output[0] |= ((data & 1) as u8) << (7 - pixel);
                data >>= 1;
                output[1] |= ((data & 1) as u8) << (7 - pixel);
                data >>= 1;
                if self.dma_conversion_depth == 2 {
                    continue;
                }
                output[2] |= ((data & 1) as u8) << (7 - pixel);
                data >>= 1;
                output[3] |= ((data & 1) as u8) << (7 - pixel);
                data >>= 1;
                if self.dma_conversion_depth == 1 {
                    continue;
                }
                output[4] |= ((data & 1) as u8) << (7 - pixel);
                data >>= 1;
                output[5] |= ((data & 1) as u8) << (7 - pixel);
                data >>= 1;
                output[6] |= ((data & 1) as u8) << (7 - pixel);
                data >>= 1;
                output[7] |= ((data & 1) as u8) << (7 - pixel);
                data >>= 1;
            }

            for (byte_index, byte) in output.into_iter().take(bytes_per_row).enumerate() {
                let plane_offset = ((byte_index & 0x06) << 3) + (byte_index & 0x01);
                let target = (self.dma_dest_address as usize + row * 2 + plane_offset) & 0x07FF;
                self.iram.bytes[target] = byte;
            }
        }
    }

    fn read_arithmetic(&self, address_offset: u16) -> Option<u8> {
        match address_offset {
            SA1_MR0..=0x230A => Some((self.mr >> ((address_offset - SA1_MR0) * 8)) as u8),
            SA1_OF => Some(if self.arithmetic_overflow { 0x80 } else { 0x00 }),
            _ => None,
        }
    }

    fn write_arithmetic(&mut self, address_offset: u16, value: u8) -> bool {
        if !matches!(address_offset, SA1_MCNT..=SA1_MBH) {
            return false;
        }

        self.registers.write(address_offset, value);
        match address_offset {
            SA1_MCNT => {
                self.arithmetic_md = value & 0x01 != 0;
                self.arithmetic_acm = value & 0x02 != 0;
                if self.arithmetic_acm {
                    self.mr = 0;
                }
            }
            SA1_MAL => self.ma = (self.ma & 0xFF00) | u16::from(value),
            SA1_MAH => self.ma = (self.ma & 0x00FF) | (u16::from(value) << 8),
            SA1_MBL => self.mb = (self.mb & 0xFF00) | u16::from(value),
            SA1_MBH => {
                self.mb = (self.mb & 0x00FF) | (u16::from(value) << 8);
                self.execute_arithmetic();
            }
            _ => {}
        }
        true
    }

    fn execute_arithmetic(&mut self) {
        if self.arithmetic_acm {
            let product = i64::from(self.ma as i16) * i64::from(self.mb as i16);
            let sum = self.mr.wrapping_add(product as u64);
            self.arithmetic_overflow = (sum >> 40) != 0;
            self.mr = sum & SA1_MR_MASK;
            self.mb = 0;
        } else if self.arithmetic_md {
            self.execute_divide();
            self.ma = 0;
            self.mb = 0;
        } else {
            let product = i32::from(self.ma as i16) * i32::from(self.mb as i16);
            self.mr = u64::from(product as u32);
            self.mb = 0;
        }
    }

    fn execute_divide(&mut self) {
        if self.mb == 0 {
            self.mr = 0;
            return;
        }

        let dividend = i32::from(self.ma as i16);
        let divisor = i32::from(self.mb);
        let remainder = dividend.rem_euclid(divisor);
        let quotient = (dividend - remainder) / divisor;
        self.mr = (u64::from(remainder as u16) << 16) | u64::from(quotient as i16 as u16);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SuperFxState {
    registers: ByteWindow,
}

const SUPERFX_VCR: u16 = 0x303B;
const SUPERFX_SFR: u16 = 0x3030;
const SUPERFX_R15: u16 = 0x301E;
const SUPERFX_R15_HIGH: u16 = 0x301F;
const SUPERFX_SCBR: u16 = 0x3038;
const SUPERFX_GO_FLAG: u8 = 0x20;
const GSU_MAX_INTERPRETER_STEPS: usize = 256 * 1024;

impl SuperFxState {
    fn new() -> Self {
        let mut registers = ByteWindow::new(0x3000, 0x0500);
        registers.write(SUPERFX_VCR, 0x04);
        Self { registers }
    }

    fn read(&self, address: u32) -> Option<u8> {
        if is_system_bank(address) {
            self.registers.read(offset(address))
        } else {
            None
        }
    }

    fn write(&mut self, address: u32, value: u8, save_ram: &mut [u8]) -> bool {
        if !is_system_bank(address) {
            return false;
        }

        let address_offset = offset(address);
        if address_offset == SUPERFX_VCR {
            return self.registers.contains(address_offset);
        }

        let handled = self.registers.write(address_offset, value);
        if handled && address_offset == SUPERFX_R15_HIGH {
            self.run_program(save_ram);
        }
        handled
    }

    fn run_program(&mut self, save_ram: &mut [u8]) {
        let r15 = u16::from_le_bytes([
            self.registers.read(SUPERFX_R15).unwrap_or(0),
            self.registers.read(SUPERFX_R15 + 1).unwrap_or(0),
        ]);
        let screen_base = usize::from(self.registers.read(SUPERFX_SCBR).unwrap_or(0)) * 0x400;
        GsuInterpreter::new(r15, screen_base, save_ram).run();

        let sfr = self.registers.read(SUPERFX_SFR).unwrap_or(0) & !SUPERFX_GO_FLAG;
        self.registers.write(SUPERFX_SFR, sfr);
    }
}

struct GsuInterpreter<'a> {
    ram: &'a mut [u8],
    registers: [u16; 16],
    pc: u16,
    source: usize,
    destination: Option<usize>,
    alt_mode: u8,
    color: u8,
    zero: bool,
    screen_base: usize,
    halted: bool,
}

impl<'a> GsuInterpreter<'a> {
    fn new(entry: u16, screen_base: usize, ram: &'a mut [u8]) -> Self {
        let mut registers = [0; 16];
        registers[15] = entry;
        Self {
            ram,
            registers,
            pc: entry,
            source: 0,
            destination: None,
            alt_mode: 0,
            color: 0,
            zero: false,
            screen_base,
            halted: false,
        }
    }

    fn run(&mut self) {
        for _ in 0..GSU_MAX_INTERPRETER_STEPS {
            if self.halted {
                break;
            }
            if !self.step() {
                break;
            }
        }
    }

    fn step(&mut self) -> bool {
        let opcode = self.fetch();
        if self.alt_mode == 0 && matches!(opcode, 0x3D..=0x3F) {
            self.sync_program_counter();
            self.alt_mode = opcode - 0x3C;
            return true;
        }

        let alt_mode = std::mem::take(&mut self.alt_mode);
        match (alt_mode, opcode) {
            (_, 0x00) => {
                self.sync_program_counter();
                self.halted = true;
            }
            (_, 0x01 | 0x02) => self.sync_program_counter(),
            (0, 0x03) => {
                self.sync_program_counter();
                self.shift_right();
            }
            (_, 0x08) => {
                let relative = self.fetch() as i8;
                self.sync_program_counter();
                if !self.zero {
                    self.branch(relative);
                }
            }
            (_, 0x09) => {
                let relative = self.fetch() as i8;
                self.sync_program_counter();
                if self.zero {
                    self.branch(relative);
                }
            }
            (_, 0x10..=0x1F) => {
                self.sync_program_counter();
                self.destination = Some(usize::from(opcode & 0x0F));
            }
            (_, 0x20..=0x2F) => {
                let register = usize::from(opcode & 0x0F);
                if self.read_ram(self.pc) & 0xF0 == 0xB0 {
                    let operand = self.fetch();
                    self.sync_program_counter();
                    let source = usize::from(operand & 0x0F);
                    let value = self.registers[source];
                    self.set_register(register, value);
                } else {
                    self.sync_program_counter();
                    self.source = register;
                    self.destination = Some(register);
                }
            }
            (0, 0x3C) => {
                self.sync_program_counter();
                self.registers[12] = self.registers[12].wrapping_sub(1);
                self.zero = self.registers[12] == 0;
                if !self.zero {
                    self.pc = self.registers[13];
                    self.registers[15] = self.pc;
                }
            }
            (0, 0x30..=0x3B) => {
                self.sync_program_counter();
                self.store_word(usize::from(opcode & 0x0F));
            }
            (1, 0x30..=0x3F) => {
                self.sync_program_counter();
                self.store_byte(usize::from(opcode & 0x0F));
            }
            (0, 0x40..=0x4B) => {
                self.sync_program_counter();
                self.load_word(usize::from(opcode & 0x0F));
            }
            (1, 0x40..=0x4B) => {
                self.sync_program_counter();
                self.load_byte(usize::from(opcode & 0x0F));
            }
            (0, 0x4C) => {
                self.sync_program_counter();
                self.plot();
            }
            (0, 0x4D) => {
                self.sync_program_counter();
                self.swap_bytes();
            }
            (0, 0x4E) => {
                self.sync_program_counter();
                self.color = self.registers[0] as u8 & 0x0F;
            }
            (0, 0x4F) => {
                self.sync_program_counter();
                self.not();
            }
            (2, 0x50..=0x5F) => {
                self.sync_program_counter();
                self.add_immediate(u16::from(opcode & 0x0F));
            }
            (0, 0x50..=0x5F) => {
                self.sync_program_counter();
                self.add_register(usize::from(opcode & 0x0F));
            }
            (3, 0x60..=0x6F) => {
                self.sync_program_counter();
                self.compare_register(usize::from(opcode & 0x0F));
            }
            (2, 0x60..=0x6F) => {
                self.sync_program_counter();
                self.subtract_immediate(u16::from(opcode & 0x0F));
            }
            (0, 0x60..=0x6F) => {
                self.sync_program_counter();
                self.subtract_register(usize::from(opcode & 0x0F));
            }
            (2, 0x70..=0x7F) => {
                self.sync_program_counter();
                self.and_immediate(u16::from(opcode & 0x0F));
            }
            (0, 0x70..=0x7F) => {
                self.sync_program_counter();
                self.and_register(usize::from(opcode & 0x0F));
            }
            (0, 0x80..=0x8F) => {
                self.sync_program_counter();
                self.multiply_register(usize::from(opcode & 0x0F));
            }
            (2, 0x80..=0x8F) => {
                self.sync_program_counter();
                self.multiply_immediate(u16::from(opcode & 0x0F));
            }
            (0, 0x95) => {
                self.sync_program_counter();
                self.sign_extend();
            }
            (0, 0x96) => {
                self.sync_program_counter();
                self.arithmetic_shift_right();
            }
            (0, 0x98..=0x9D) => {
                self.jump_register(usize::from(opcode & 0x0F));
            }
            (0, 0x9E) => {
                self.sync_program_counter();
                self.low_byte();
            }
            (_, 0xA0..=0xAF) => {
                let value = self.fetch();
                self.sync_program_counter();
                self.load_register(usize::from(opcode & 0x0F), u16::from(value));
            }
            (_, 0xB0..=0xBF) => {
                self.sync_program_counter();
                self.source = usize::from(opcode & 0x0F);
            }
            (_, 0xD0..=0xDF) => {
                self.sync_program_counter();
                let register = usize::from(opcode & 0x0F);
                self.registers[register] = self.registers[register].wrapping_add(1);
                self.zero = self.registers[register] == 0;
                self.source = register;
            }
            (_, 0xE0..=0xEF) => {
                self.sync_program_counter();
                let register = usize::from(opcode & 0x0F);
                self.registers[register] = self.registers[register].wrapping_sub(1);
                self.zero = self.registers[register] == 0;
                self.source = register;
            }
            (0, 0xC0) => {
                self.sync_program_counter();
                self.high_byte();
            }
            (2, 0xC0..=0xCF) => {
                self.sync_program_counter();
                self.or_immediate(u16::from(opcode & 0x0F));
            }
            (1, 0xC0..=0xCF) => {
                self.sync_program_counter();
                self.xor_register(usize::from(opcode & 0x0F));
            }
            (3, 0xC0..=0xCF) => {
                self.sync_program_counter();
                self.xor_immediate(u16::from(opcode & 0x0F));
            }
            (0, 0xC1..=0xCF) => {
                self.sync_program_counter();
                self.or_register(usize::from(opcode & 0x0F));
            }
            (_, 0xF0..=0xFF) => {
                let low = self.fetch();
                let high = self.fetch();
                self.sync_program_counter();
                self.load_register(usize::from(opcode & 0x0F), u16::from_le_bytes([low, high]));
            }
            _ => return false,
        }
        true
    }

    fn fetch(&mut self) -> u8 {
        let value = self.read_ram(self.pc);
        self.pc = self.pc.wrapping_add(1);
        value
    }

    fn sync_program_counter(&mut self) {
        self.registers[15] = self.pc;
    }

    fn branch(&mut self, relative: i8) {
        self.pc = self.pc.wrapping_add_signed(i16::from(relative));
        self.registers[15] = self.pc;
    }

    fn jump_register(&mut self, register: usize) {
        self.pc = self.registers[register];
        self.registers[15] = self.pc;
    }

    fn set_register(&mut self, register: usize, value: u16) {
        self.registers[register] = value;
        self.zero = value == 0;
        self.source = register;
        self.destination = None;
    }

    fn load_register(&mut self, register: usize, value: u16) {
        self.registers[register] = value;
        self.zero = value == 0;
    }

    fn compare_register(&mut self, register: usize) {
        self.zero = self.registers[self.source] == self.registers[register];
        self.destination = None;
    }

    fn add_register(&mut self, register: usize) {
        let result = self.registers[self.source].wrapping_add(self.registers[register]);
        self.write_result(result);
    }

    fn add_immediate(&mut self, value: u16) {
        let result = self.registers[self.source].wrapping_add(value);
        self.write_result(result);
    }

    fn subtract_register(&mut self, register: usize) {
        let result = self.registers[self.source].wrapping_sub(self.registers[register]);
        self.write_result(result);
    }

    fn subtract_immediate(&mut self, value: u16) {
        let result = self.registers[self.source].wrapping_sub(value);
        self.write_result(result);
    }

    fn and_immediate(&mut self, value: u16) {
        let result = self.registers[self.source] & value;
        self.write_result(result);
    }

    fn and_register(&mut self, register: usize) {
        let result = self.registers[self.source] & self.registers[register];
        self.write_result(result);
    }

    fn or_register(&mut self, register: usize) {
        let result = self.registers[self.source] | self.registers[register];
        self.write_result(result);
    }

    fn or_immediate(&mut self, value: u16) {
        let result = self.registers[self.source] | value;
        self.write_result(result);
    }

    fn xor_register(&mut self, register: usize) {
        let result = self.registers[self.source] ^ self.registers[register];
        self.write_result(result);
    }

    fn xor_immediate(&mut self, value: u16) {
        let result = self.registers[self.source] ^ value;
        self.write_result(result);
    }

    fn multiply_register(&mut self, register: usize) {
        let result = self.registers[self.source].wrapping_mul(self.registers[register]);
        self.write_result(result);
    }

    fn multiply_immediate(&mut self, value: u16) {
        let result = self.registers[self.source].wrapping_mul(value);
        self.write_result(result);
    }

    fn not(&mut self) {
        let result = !self.registers[self.source];
        self.write_result(result);
    }

    fn shift_right(&mut self) {
        let result = self.registers[self.source] >> 1;
        self.write_result(result);
    }

    fn arithmetic_shift_right(&mut self) {
        let result = ((self.registers[self.source] as i16) >> 1) as u16;
        self.write_result(result);
    }

    fn high_byte(&mut self) {
        let result = self.registers[self.source] >> 8;
        self.write_result(result);
    }

    fn low_byte(&mut self) {
        let result = self.registers[self.source] & 0x00FF;
        self.write_result(result);
    }

    fn sign_extend(&mut self) {
        let result = i16::from(self.registers[self.source] as u8 as i8) as u16;
        self.write_result(result);
    }

    fn swap_bytes(&mut self) {
        let result = self.registers[self.source].swap_bytes();
        self.write_result(result);
    }

    fn write_result(&mut self, result: u16) {
        let destination = self.destination.take().unwrap_or(0);
        self.set_register(destination, result);
    }

    fn store_word(&mut self, register: usize) {
        let address = self.registers[register];
        let value = self.registers[0].to_le_bytes();
        self.write_ram(address, value[0]);
        self.write_ram(address.wrapping_add(1), value[1]);
        self.destination = None;
    }

    fn store_byte(&mut self, register: usize) {
        let address = self.registers[register];
        self.write_ram(address, self.registers[0] as u8);
        self.destination = None;
    }

    fn load_word(&mut self, register: usize) {
        let address = self.registers[register];
        let value = u16::from_le_bytes([
            self.read_ram(address),
            self.read_ram(address.wrapping_add(1)),
        ]);
        self.write_load_result(value);
    }

    fn load_byte(&mut self, register: usize) {
        let value = u16::from(self.read_ram(self.registers[register]));
        self.write_load_result(value);
    }

    fn write_load_result(&mut self, value: u16) {
        if let Some(destination) = self.destination.take() {
            self.registers[destination] = value;
            self.zero = value == 0;
        } else {
            self.set_register(0, value);
        }
    }

    fn plot(&mut self) {
        let x = usize::from(self.registers[1]);
        let y = usize::from(self.registers[2]);
        let tile_index = (y / 8) * 16 + (x / 8);
        let tile_base = self.screen_base + tile_index * 32;
        let row = y & 0x07;
        let bit = 0x80 >> (x & 0x07);
        for plane in 0..4 {
            let byte_offset = tile_base + row * 2 + (plane & 0x01) + (plane / 2) * 16;
            let mut value = self.read_ram_usize(byte_offset);
            if self.color & (1 << plane) != 0 {
                value |= bit;
            } else {
                value &= !bit;
            }
            self.write_ram_usize(byte_offset, value);
        }
        self.registers[1] = self.registers[1].wrapping_add(1);
        self.source = 1;
        self.destination = None;
    }

    fn read_ram(&self, address: u16) -> u8 {
        self.read_ram_usize(usize::from(address))
    }

    fn read_ram_usize(&self, address: usize) -> u8 {
        if self.ram.is_empty() {
            0
        } else {
            self.ram[address % self.ram.len()]
        }
    }

    fn write_ram(&mut self, address: u16, value: u8) {
        self.write_ram_usize(usize::from(address), value);
    }

    fn write_ram_usize(&mut self, address: usize, value: u8) {
        if !self.ram.is_empty() {
            self.ram[address % self.ram.len()] = value;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cx4State {
    ram: Box<[u8; CX4_RAM_LEN]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cx4Point {
    x: i16,
    y: i16,
    z: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cx4OamCursor {
    oam: usize,
    oam_hi: usize,
    size_offset: u8,
    sprite_slots: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cx4OamEntry {
    x: i16,
    y: i16,
    name: u8,
    attributes: u8,
    large: bool,
}

const CX4_RAM_START: u16 = 0x6000;
const CX4_RAM_LEN: usize = 0x2000;
const CX4_LOAD_TRIGGER: u16 = 0x7F47;
const CX4_COMMAND_TRIGGER: u16 = 0x7F4F;
const CX4_BUSY_STATUS: u16 = 0x7F5E;
const CX4_DATA_START: usize = 0x1F80;
const CX4_COMMAND_MODE: usize = 0x1F4D;
const CX4_LOAD_SOURCE: usize = 0x1F40;
const CX4_LOAD_LEN: usize = 0x1F43;
const CX4_LOAD_DEST: usize = 0x1F45;
const CX4_IMMEDIATE_DATA: [u8; 48] = [
    0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x00, 0xFF, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x00, 0x00,
    0xFF, 0xFF, 0x00, 0x00, 0x80, 0xFF, 0xFF, 0x7F, 0x00, 0x80, 0x00, 0xFF, 0x7F, 0x00, 0xFF, 0x7F,
    0xFF, 0x7F, 0xFF, 0xFF, 0x00, 0x00, 0x01, 0xFF, 0xFF, 0xFE, 0x00, 0x01, 0x00, 0xFF, 0xFE, 0x00,
];
const CX4_WAVE_DATA: [usize; 40] = [
    0x0000, 0x0002, 0x0004, 0x0006, 0x0008, 0x000A, 0x000C, 0x000E, 0x0200, 0x0202, 0x0204, 0x0206,
    0x0208, 0x020A, 0x020C, 0x020E, 0x0400, 0x0402, 0x0404, 0x0406, 0x0408, 0x040A, 0x040C, 0x040E,
    0x0600, 0x0602, 0x0604, 0x0606, 0x0608, 0x060A, 0x060C, 0x060E, 0x0800, 0x0802, 0x0804, 0x0806,
    0x0808, 0x080A, 0x080C, 0x080E,
];

impl Cx4State {
    fn new() -> Self {
        Self {
            ram: Box::new([0; CX4_RAM_LEN]),
        }
    }

    fn read(&self, address: u32) -> Option<u8> {
        if !is_system_bank(address) {
            return None;
        }

        let offset = offset(address);
        if offset == CX4_BUSY_STATUS {
            return Some(0);
        }

        self.index(offset).map(|index| self.ram[index])
    }

    fn write(&mut self, address: u32, value: u8, rom: &[u8]) -> bool {
        if !is_system_bank(address) {
            return false;
        }

        let offset = offset(address);
        let Some(index) = self.index(offset) else {
            return false;
        };

        self.ram[index] = value;
        match offset {
            CX4_LOAD_TRIGGER => self.load_rom_window(rom),
            CX4_COMMAND_TRIGGER => self.execute_command(value, rom),
            _ => {}
        }
        true
    }

    fn index(&self, address_offset: u16) -> Option<usize> {
        let relative = address_offset.checked_sub(CX4_RAM_START)? as usize;
        (relative < self.ram.len()).then_some(relative)
    }

    fn load_rom_window(&mut self, rom: &[u8]) {
        let source = self.read_u24(CX4_LOAD_SOURCE);
        let len = usize::from(self.read_u16(CX4_LOAD_LEN));
        let dest = usize::from(self.read_u16(CX4_LOAD_DEST) & 0x1FFF);

        for byte_index in 0..len {
            let Some(source_index) =
                lorom_rom_index(source.wrapping_add(byte_index as u32), rom.len())
            else {
                break;
            };
            let dest_index = (dest + byte_index) % self.ram.len();
            self.ram[dest_index] = rom[source_index];
        }
    }

    fn execute_command(&mut self, command: u8, rom: &[u8]) {
        if self.ram[CX4_COMMAND_MODE] == 0x0E && command < 0x40 && command & 0x03 == 0 {
            self.ram[CX4_DATA_START] = command >> 2;
            return;
        }

        match command {
            0x00 => self.command_sprite(rom),
            0x01 => self.command_draw_wireframe_clear(rom),
            0x05 => self.command_propulsion(),
            0x0D => self.command_set_vector_length(),
            0x10 => self.command_polar_to_rectangular(true),
            0x13 => self.command_polar_to_rectangular(false),
            0x15 => self.command_pythagorean(),
            0x1F => self.command_atan(),
            0x22 => self.command_trapezoid(),
            0x25 => self.command_multiply(),
            0x2D => self.command_transform_coordinates(),
            0x40 => self.command_sum(),
            0x54 => self.command_square(),
            0x5C => self.command_immediate_register_reset(),
            0x5E => self.command_immediate_register(0),
            0x60..=0x7C if command.is_multiple_of(2) => {
                self.command_immediate_register(usize::from((command - 0x5E) / 2) * 3)
            }
            0x89 => {
                self.ram[CX4_DATA_START] = 0x36;
                self.ram[CX4_DATA_START + 1] = 0x43;
                self.ram[CX4_DATA_START + 2] = 0x05;
            }
            _ => {}
        }
    }

    fn command_sprite(&mut self, rom: &[u8]) {
        match self.ram[CX4_COMMAND_MODE] {
            0x00 => self.command_build_oam(rom),
            0x03 => self.command_scale_rotate(0),
            0x05 => self.command_transform_lines(),
            0x07 => self.command_scale_rotate(64),
            0x08 => self.command_draw_wireframe(rom),
            0x0B => self.command_disintegrate(),
            0x0C => self.command_bitplane_wave(),
            _ => {}
        }
    }

    fn command_build_oam(&mut self, rom: &[u8]) {
        let oam = usize::from(self.ram[0x0626]) << 2;
        let mut clear = 0x01FDusize;
        while clear > oam {
            self.ram[clear] = 0xE0;
            if clear < 4 {
                break;
            }
            clear -= 4;
        }

        let global_x = self.read_u16(0x0621);
        let global_y = self.read_u16(0x0623);
        let oam_hi = 0x0200 + (usize::from(self.ram[0x0626]) >> 2);
        if self.ram[0x0620] == 0 {
            return;
        }

        let mut cursor = Cx4OamCursor {
            oam,
            oam_hi,
            size_offset: (self.ram[0x0626] & 0x03) * 2,
            sprite_slots: 128u8.saturating_sub(self.ram[0x0626]),
        };
        let mut source = 0x0220usize;
        for _ in 0..self.ram[0x0620] {
            if cursor.sprite_slots == 0 || source + 15 >= self.ram.len() {
                break;
            }

            let sprite_x = self.read_u16(source).wrapping_sub(global_x) as i16;
            let sprite_y = self.read_u16(source + 2).wrapping_sub(global_y) as i16;
            let name = self.ram[source + 5];
            let attributes = self.ram[source + 4] | self.ram[source + 6];
            let mut sprite_data = self.read_u24(source + 7);

            let sprite_count = cx4_rom_read(rom, sprite_data);
            if sprite_count != 0 {
                sprite_data = sprite_data.wrapping_add(1);
                for _ in 0..sprite_count {
                    if cursor.sprite_slots == 0 {
                        break;
                    }
                    let flags = cx4_rom_read(rom, sprite_data);
                    let mut x = i16::from(cx4_rom_read(rom, sprite_data.wrapping_add(1)) as i8);
                    if attributes & 0x40 != 0 {
                        x = -x - if flags & 0x20 != 0 { 16 } else { 8 };
                    }
                    x = x.wrapping_add(sprite_x);
                    if (-16..=272).contains(&i32::from(x)) {
                        let mut y = i16::from(cx4_rom_read(rom, sprite_data.wrapping_add(2)) as i8);
                        if attributes & 0x80 != 0 {
                            y = -y - if flags & 0x20 != 0 { 16 } else { 8 };
                        }
                        y = y.wrapping_add(sprite_y);
                        if (-16..=224).contains(&i32::from(y)) {
                            self.write_oam_entry(
                                &mut cursor,
                                Cx4OamEntry {
                                    x,
                                    y,
                                    name: name.wrapping_add(cx4_rom_read(
                                        rom,
                                        sprite_data.wrapping_add(3),
                                    )),
                                    attributes: attributes ^ (flags & 0xC0),
                                    large: flags & 0x20 != 0,
                                },
                            );
                        }
                    }
                    sprite_data = sprite_data.wrapping_add(4);
                }
            } else {
                self.write_oam_entry(
                    &mut cursor,
                    Cx4OamEntry {
                        x: sprite_x,
                        y: sprite_y,
                        name,
                        attributes,
                        large: true,
                    },
                );
            }

            source += 16;
        }
    }

    fn write_oam_entry(&mut self, cursor: &mut Cx4OamCursor, entry: Cx4OamEntry) {
        if cursor.sprite_slots == 0
            || cursor.oam + 3 >= self.ram.len()
            || cursor.oam_hi >= self.ram.len()
        {
            return;
        }

        self.ram[cursor.oam] = entry.x as u8;
        self.ram[cursor.oam + 1] = entry.y as u8;
        self.ram[cursor.oam + 2] = entry.name;
        self.ram[cursor.oam + 3] = entry.attributes;
        let mask = 0x03 << cursor.size_offset;
        self.ram[cursor.oam_hi] &= !mask;
        if entry.x & 0x0100 != 0 {
            self.ram[cursor.oam_hi] |= 0x01 << cursor.size_offset;
        }
        if entry.large {
            self.ram[cursor.oam_hi] |= 0x02 << cursor.size_offset;
        }

        cursor.oam += 4;
        cursor.sprite_slots = cursor.sprite_slots.saturating_sub(1);
        cursor.size_offset = (cursor.size_offset + 2) & 0x06;
        if cursor.size_offset == 0 {
            cursor.oam_hi += 1;
        }
    }

    fn command_propulsion(&mut self) {
        let divisor = u32::from(self.read_u16(CX4_DATA_START + 3));
        let quotient = 0x1_0000u32.checked_div(divisor).unwrap_or(0x1_0000);
        let output = (quotient * u32::from(self.read_u16(CX4_DATA_START + 1))) >> 8;
        self.write_u16(CX4_DATA_START, output as u16);
    }

    fn command_set_vector_length(&mut self) {
        let x = f64::from(self.read_i16(CX4_DATA_START));
        let y = f64::from(self.read_i16(CX4_DATA_START + 3));
        let distance = f64::from(self.read_i16(CX4_DATA_START + 6));
        let radius = (x * x + y * y).sqrt();
        if radius == 0.0 {
            self.write_u16(CX4_DATA_START + 9, 0);
            self.write_u16(CX4_DATA_START + 12, 0);
            return;
        }

        self.write_i16(CX4_DATA_START + 9, (x * distance / radius * 0.98) as i16);
        self.write_i16(CX4_DATA_START + 12, (y * distance / radius * 0.99) as i16);
    }

    fn command_polar_to_rectangular(&mut self, signed_radius: bool) {
        let angle =
            f64::from(self.read_u16(CX4_DATA_START) & 0x01FF) * std::f64::consts::TAU / 512.0;
        let raw_radius = i32::from(self.read_i16(CX4_DATA_START + 3));
        let radius = if signed_radius {
            (raw_radius << 1) >> 1
        } else {
            raw_radius
        } as f64;
        let scale = if signed_radius { 1.0 } else { 256.0 };
        let x = (radius * angle.cos() * scale) as i32;
        let mut y = (radius * angle.sin() * scale) as i32;
        if signed_radius {
            y -= y >> 6;
        }
        self.write_u24(CX4_DATA_START + 6, x as u32);
        self.write_u24(CX4_DATA_START + 9, y as u32);
    }

    fn command_pythagorean(&mut self) {
        let x = f64::from(self.read_i16(CX4_DATA_START));
        let y = f64::from(self.read_i16(CX4_DATA_START + 3));
        self.write_i16(CX4_DATA_START, (x.hypot(y)) as i16);
    }

    fn command_atan(&mut self) {
        let x = f64::from(self.read_i16(CX4_DATA_START));
        let y = f64::from(self.read_i16(CX4_DATA_START + 3));
        let angle = if x == 0.0 {
            if y > 0.0 { 0x80 } else { 0x180 }
        } else {
            let mut result = (y / x).atan() / std::f64::consts::TAU * 512.0;
            if x < 0.0 {
                result += 0x100 as f64;
            }
            (result as i16) & 0x01FF
        };
        self.write_u16(CX4_DATA_START + 6, angle as u16);
    }

    fn command_trapezoid(&mut self) {
        let angle1 = cx4_angle512(self.read_u16(CX4_DATA_START + 12) & 0x01FF);
        let angle2 = cx4_angle512(self.read_u16(CX4_DATA_START + 15) & 0x01FF);
        let tan1 = cx4_tan(angle1);
        let tan2 = cx4_tan(angle2);

        let initial_y = i32::from(self.read_i16(CX4_DATA_START + 3))
            - i32::from(self.read_i16(CX4_DATA_START + 9));
        let origin_x = i32::from(self.read_i16(CX4_DATA_START));
        let center_x = i32::from(self.read_i16(CX4_DATA_START + 6));
        let width = i32::from(self.read_i16(CX4_DATA_START + 19));

        for (line, y) in (0..225).zip(initial_y..) {
            let (left, right) = if y < 0 {
                (1, 0)
            } else {
                let left = (tan1 * f64::from(y)) as i32 - origin_x + center_x;
                let right = (tan2 * f64::from(y)) as i32 - origin_x + center_x + width;
                cx4_clip_trapezoid_span(left, right)
            };
            self.ram[0x0800 + line] = left;
            self.ram[0x0900 + line] = right;
        }
    }

    fn command_multiply(&mut self) {
        let left = self.read_u24(CX4_DATA_START);
        let right = self.read_u24(CX4_DATA_START + 3);
        self.write_u24(CX4_DATA_START, left.wrapping_mul(right));
    }

    fn command_transform_coordinates(&mut self) {
        let mut x = f64::from(self.read_i16(CX4_DATA_START + 1));
        let y = f64::from(self.read_i16(CX4_DATA_START + 4));
        let z = f64::from(self.read_i16(CX4_DATA_START + 7));
        let rotate_x = -cx4_angle128(self.ram[CX4_DATA_START + 9]);
        let rotate_y = -cx4_angle128(self.ram[CX4_DATA_START + 10]);
        let rotate_z = -cx4_angle128(self.ram[CX4_DATA_START + 11]);
        let scale = f64::from(self.read_u16(CX4_DATA_START + 16));

        let y2 = y * rotate_x.cos() - z * rotate_x.sin();
        let z2 = y * rotate_x.sin() + z * rotate_x.cos();

        let x2 = x * rotate_y.cos() + z2 * rotate_y.sin();
        let y = x2 * rotate_z.sin() + y2 * rotate_z.cos();
        x = x2 * rotate_z.cos() - y2 * rotate_z.sin();

        self.write_i16(CX4_DATA_START, (x * scale / 256.0) as i16);
        self.write_i16(CX4_DATA_START + 3, (y * scale / 256.0) as i16);
    }

    fn command_sum(&mut self) {
        let sum = self.ram[..0x800]
            .iter()
            .fold(0u16, |sum, value| sum.wrapping_add(u16::from(*value)));
        self.write_u16(CX4_DATA_START, sum);
    }

    fn command_square(&mut self) {
        let value = i64::from(self.read_i24(CX4_DATA_START));
        let squared = value * value;
        self.write_u24(CX4_DATA_START + 3, squared as u32);
        self.write_u24(CX4_DATA_START + 6, (squared >> 24) as u32);
    }

    fn command_transform_lines(&mut self) {
        let rotate_x = self.ram[CX4_DATA_START + 3];
        let rotate_y = self.ram[CX4_DATA_START + 6];
        let rotate_z = self.ram[CX4_DATA_START + 9];
        let scale = self.ram[CX4_DATA_START + 12];

        let vertex_count = usize::from(self.read_u16(CX4_DATA_START));
        let max_vertices = if self.ram.len() > 10 {
            (self.ram.len() - 11) / 0x10 + 1
        } else {
            0
        };
        for vertex in 0..vertex_count.min(max_vertices) {
            let base = vertex * 0x10;
            let (x, y) = cx4_transform_wireframe(
                self.read_i16(base + 1),
                self.read_i16(base + 5),
                self.read_i16(base + 9),
                rotate_x,
                rotate_y,
                rotate_z,
                scale,
            );
            self.write_i16(base + 1, x.wrapping_add(0x80));
            self.write_i16(base + 5, y.wrapping_add(0x50));
        }

        self.write_u16(0x0600, 23);
        self.write_u16(0x0602, 0x60);
        self.write_u16(0x0605, 0x40);
        self.write_u16(0x0608, 23);
        self.write_u16(0x060A, 0x60);
        self.write_u16(0x060D, 0x40);

        let line_count = usize::from(self.read_u16(0x0B00));
        let max_line_sources = if self.ram.len() > 0x0B03 {
            (self.ram.len() - 0x0B04) / 2 + 1
        } else {
            0
        };
        let max_line_outputs = if self.ram.len() > 0x0606 {
            (self.ram.len() - 0x0607) / 8 + 1
        } else {
            0
        };
        for line in 0..line_count.min(max_line_sources).min(max_line_outputs) {
            let source = 0x0B02 + line * 2;
            let output = 0x0600 + line * 8;
            let start = usize::from(self.ram[source]) << 4;
            let end = usize::from(self.ram[source + 1]) << 4;
            let (distance, step_x, step_y) = cx4_calc_wireframe(
                self.read_i16(start + 1),
                self.read_i16(start + 5),
                self.read_i16(end + 1),
                self.read_i16(end + 5),
            );
            self.write_u16(output, distance);
            self.write_i16(output + 2, step_x);
            self.write_i16(output + 5, step_y);
        }
    }

    fn command_draw_wireframe_clear(&mut self, rom: &[u8]) {
        for byte in self.ram[0x0300..0x0C00].iter_mut() {
            *byte = 0;
        }
        self.command_draw_wireframe(rom);
    }

    fn command_draw_wireframe(&mut self, rom: &[u8]) {
        let mut line = self.read_u24(CX4_DATA_START);
        for _ in 0..self.ram[0x0295] {
            let point1 = if cx4_rom_read(rom, line) == 0xFF
                && cx4_rom_read(rom, line.wrapping_add(1)) == 0xFF
            {
                let mut previous = line.wrapping_sub(5);
                while cx4_rom_read(rom, previous.wrapping_add(2)) == 0xFF
                    && cx4_rom_read(rom, previous.wrapping_add(3)) == 0xFF
                    && previous >= 5
                {
                    previous = previous.wrapping_sub(5);
                }
                u32::from(self.ram[CX4_DATA_START + 2]) << 16
                    | (u32::from(cx4_rom_read(rom, previous.wrapping_add(2))) << 8)
                    | u32::from(cx4_rom_read(rom, previous.wrapping_add(3)))
            } else {
                u32::from(self.ram[CX4_DATA_START + 2]) << 16
                    | (u32::from(cx4_rom_read(rom, line)) << 8)
                    | u32::from(cx4_rom_read(rom, line.wrapping_add(1)))
            };
            let point2 = u32::from(self.ram[CX4_DATA_START + 2]) << 16
                | (u32::from(cx4_rom_read(rom, line.wrapping_add(2))) << 8)
                | u32::from(cx4_rom_read(rom, line.wrapping_add(3)));
            let color = cx4_rom_read(rom, line.wrapping_add(4));
            self.draw_wireframe_line(
                cx4_rom_read_point(rom, point1),
                cx4_rom_read_point(rom, point2),
                color,
            );
            line = line.wrapping_add(5);
        }
    }

    fn draw_wireframe_line(&mut self, start: Cx4Point, end: Cx4Point, color: u8) {
        let (x1, y1) = cx4_transform_wireframe_2(
            start.x,
            start.y,
            start.z,
            self.ram[CX4_DATA_START + 6],
            self.ram[CX4_DATA_START + 7],
            self.ram[CX4_DATA_START + 8],
            self.ram[CX4_DATA_START + 16],
        );
        let (x2, y2) = cx4_transform_wireframe_2(
            end.x,
            end.y,
            end.z,
            self.ram[CX4_DATA_START + 6],
            self.ram[CX4_DATA_START + 7],
            self.ram[CX4_DATA_START + 8],
            self.ram[CX4_DATA_START + 16],
        );

        let mut x = (i32::from(x1) + 48) << 8;
        let mut y = (i32::from(y1) + 48) << 8;
        let end_x = (i32::from(x2) + 48) << 8;
        let end_y = (i32::from(y2) + 48) << 8;
        let (distance, step_x, step_y) = cx4_calc_wireframe(
            (x >> 8) as i16,
            (y >> 8) as i16,
            (end_x >> 8) as i16,
            (end_y >> 8) as i16,
        );

        for _ in 0..distance {
            if x > 0xFF && y > 0xFF && x < 0x6000 && y < 0x6000 {
                let pixel_x = (x >> 8) as usize;
                let pixel_y = (y >> 8) as usize;
                let address = ((pixel_y >> 3) << 8) - ((pixel_y >> 3) << 6)
                    + ((pixel_x >> 3) << 4)
                    + (pixel_y & 7) * 2
                    + 0x0300;
                let mask = 0x80 >> (pixel_x & 7);
                if address + 1 < self.ram.len() {
                    self.ram[address] &= !mask;
                    self.ram[address + 1] &= !mask;
                    if color & 0x01 != 0 {
                        self.ram[address] |= mask;
                    }
                    if color & 0x02 != 0 {
                        self.ram[address + 1] |= mask;
                    }
                }
            }
            x += i32::from(step_x);
            y += i32::from(step_y);
        }
    }

    fn command_scale_rotate(&mut self, row_padding: usize) {
        let angle = self.read_u16(CX4_DATA_START) & 0x01FF;
        let scale_x = cx4_scale_factor(self.read_u16(CX4_DATA_START + 15));
        let scale_y = cx4_scale_factor(self.read_u16(CX4_DATA_START + 18));
        let (a, b, c, d) = cx4_scale_rotate_matrix(angle, scale_x, scale_y);

        let width = i32::from(self.ram[CX4_DATA_START + 9] & !7);
        let height = i32::from(self.ram[CX4_DATA_START + 12] & !7);
        let clear_len = ((width as usize + row_padding / 4) * height as usize) / 2;
        for byte in self.ram.iter_mut().take(clear_len) {
            *byte = 0;
        }

        let center_x = i32::from(self.read_i16(CX4_DATA_START + 3));
        let center_y = i32::from(self.read_i16(CX4_DATA_START + 6));
        let mut line_x = (center_x << 12) - center_x * a - center_x * b;
        let mut line_y = (center_y << 12) - center_y * c - center_y * d;
        let mut output_index = 0i32;
        let mut mask = 0x80;

        for _ in 0..height {
            let mut source_x = line_x;
            let mut source_y = line_y;
            for _ in 0..width {
                let sample_x = source_x >> 12;
                let sample_y = source_y >> 12;
                let pixel = if (0..width).contains(&sample_x) && (0..height).contains(&sample_y) {
                    let packed_index = sample_y as usize * width as usize + sample_x as usize;
                    let mut pixel = self
                        .ram
                        .get(0x0600 + (packed_index >> 1))
                        .copied()
                        .unwrap_or(0);
                    if packed_index & 1 != 0 {
                        pixel >>= 4;
                    }
                    pixel &= 0x0F;
                    pixel
                } else {
                    0
                };

                if output_index >= 0 {
                    self.write_bitplane_pixel(output_index as usize, mask, pixel);
                }
                mask >>= 1;
                if mask == 0 {
                    mask = 0x80;
                    output_index += 32;
                }

                source_x += a;
                source_y += c;
            }

            output_index += 2 + row_padding as i32;
            if output_index & 0x10 != 0 {
                output_index &= !0x10;
            } else {
                output_index -= width * 4 + row_padding as i32;
            }
            line_x += b;
            line_y += d;
        }
    }

    fn command_disintegrate(&mut self) {
        let center_x = i32::from(self.read_i16(CX4_DATA_START));
        let center_y = i32::from(self.read_i16(CX4_DATA_START + 3));
        let scale_x = i32::from(self.read_i16(CX4_DATA_START + 6));
        let width = i32::from(self.ram[CX4_DATA_START + 9]);
        let height = i32::from(self.ram[CX4_DATA_START + 12]);
        let scale_y = i32::from(self.read_i16(CX4_DATA_START + 15));

        let clear_len = (width.max(0) as usize * height.max(0) as usize) / 2;
        for byte in self.ram.iter_mut().take(clear_len) {
            *byte = 0;
        }

        let mut source_index = 0x0600usize;
        let mut source_byte = self.ram[source_index];
        let mut source_low_nibble = true;
        let mut source_y = -center_y * scale_y + (center_y << 8);
        for _ in 0..height {
            let mut source_x = -center_x * scale_x + (center_x << 8);
            for _ in 0..width {
                let pixel = if source_low_nibble {
                    source_byte & 0x0F
                } else {
                    (source_byte >> 4) & 0x0F
                };
                if !source_low_nibble {
                    source_index += 1;
                    source_byte = self.ram.get(source_index).copied().unwrap_or(0);
                }
                source_low_nibble = !source_low_nibble;

                let sample_x = source_x >> 8;
                let sample_y = source_y >> 8;
                if (0..width).contains(&sample_x) && (0..height).contains(&sample_y) {
                    let sample_x = sample_x as usize;
                    let sample_y = sample_y as usize;
                    let output_index = (sample_y >> 3) * width as usize * 4
                        + (sample_x >> 3) * 32
                        + (sample_y & 7) * 2;
                    let mask = 0x80 >> (sample_x & 7);
                    self.write_bitplane_pixel(output_index, mask, pixel);
                }
                source_x += scale_x;
            }
            source_y += scale_y;
        }
    }

    fn write_bitplane_pixel(&mut self, output_index: usize, mask: u8, pixel: u8) {
        if output_index >= self.ram.len().saturating_sub(17) {
            return;
        }
        if pixel & 0x01 != 0 {
            self.ram[output_index] |= mask;
        }
        if pixel & 0x02 != 0 {
            self.ram[output_index + 1] |= mask;
        }
        if pixel & 0x04 != 0 {
            self.ram[output_index + 16] |= mask;
        }
        if pixel & 0x08 != 0 {
            self.ram[output_index + 17] |= mask;
        }
    }

    fn command_bitplane_wave(&mut self) {
        let mut dest = 0usize;
        let mut wave = usize::from(self.ram[CX4_DATA_START + 3]);
        for _ in 0..0x10 {
            self.apply_bitplane_wave_group(dest, 0x0A00, &mut wave);
            dest += 16;
            self.apply_bitplane_wave_group(dest, 0x0A10, &mut wave);
            dest += 16;
        }
    }

    fn apply_bitplane_wave_group(&mut self, dest: usize, source: usize, wave: &mut usize) {
        let mut mask = 0xC0C0u16;
        loop {
            let start_height = -i16::from(self.ram[0x0B00 + *wave] as i8) - 16;
            for (height, offset) in (start_height..).zip(CX4_WAVE_DATA) {
                let index = dest + offset;
                let mut value = self.read_u16(index) & !mask;
                if height >= 0 {
                    value |= if height < 8 {
                        mask & self.read_u16(source + height as usize * 2)
                    } else {
                        mask & 0xFF00
                    };
                }
                self.write_u16(index, value);
            }
            *wave = (*wave + 1) & 0x7F;
            mask = (mask >> 2) | (mask << 6);
            if mask == 0xC0C0 {
                break;
            }
        }
    }

    fn command_immediate_register_reset(&mut self) {
        self.write_u24(CX4_DATA_START, 0);
        self.command_immediate_register(0);
    }

    fn command_immediate_register(&mut self, start: usize) {
        let mut address = self.read_u24(CX4_DATA_START);
        for value in CX4_IMMEDIATE_DATA[start..].iter().copied() {
            let index = (address & 0x0FFF) as usize;
            if index < 0x0C00 {
                self.ram[index] = value;
            }
            address = address.wrapping_add(1);
        }
        self.write_u24(CX4_DATA_START, address);
    }

    fn read_u16(&self, index: usize) -> u16 {
        u16::from_le_bytes([self.ram[index], self.ram[index + 1]])
    }

    fn read_i16(&self, index: usize) -> i16 {
        i16::from_le_bytes([self.ram[index], self.ram[index + 1]])
    }

    fn read_u24(&self, index: usize) -> u32 {
        u32::from(self.ram[index])
            | (u32::from(self.ram[index + 1]) << 8)
            | (u32::from(self.ram[index + 2]) << 16)
    }

    fn read_i24(&self, index: usize) -> i32 {
        let value = self.read_u24(index) as i32;
        if value & 0x80_0000 != 0 {
            value | !0xFF_FFFF
        } else {
            value
        }
    }

    fn write_u16(&mut self, index: usize, value: u16) {
        let [low, high] = value.to_le_bytes();
        self.ram[index] = low;
        self.ram[index + 1] = high;
    }

    fn write_i16(&mut self, index: usize, value: i16) {
        self.write_u16(index, value as u16);
    }

    fn write_u24(&mut self, index: usize, value: u32) {
        self.ram[index] = value as u8;
        self.ram[index + 1] = (value >> 8) as u8;
        self.ram[index + 2] = (value >> 16) as u8;
    }
}

fn cx4_angle128(value: u8) -> f64 {
    f64::from(value) * std::f64::consts::TAU / 128.0
}

fn cx4_scale_factor(raw: u16) -> i32 {
    if raw & 0x8000 != 0 {
        0x7FFF
    } else {
        i32::from(raw)
    }
}

fn cx4_scale_rotate_matrix(angle: u16, scale_x: i32, scale_y: i32) -> (i32, i32, i32, i32) {
    match angle {
        0 => (scale_x, 0, 0, scale_y),
        128 => (0, -scale_y, scale_x, 0),
        256 => (-scale_x, 0, 0, -scale_y),
        384 => (0, scale_y, -scale_x, 0),
        _ => {
            let sin = cx4_sin512(angle);
            let cos = cx4_cos512(angle);
            (
                (cos * scale_x) >> 15,
                -((sin * scale_y) >> 15),
                (sin * scale_x) >> 15,
                (cos * scale_y) >> 15,
            )
        }
    }
}

fn cx4_sin512(value: u16) -> i32 {
    ((f64::from(value & 0x01FF) * std::f64::consts::TAU / 512.0).sin() * 32767.0).round() as i32
}

fn cx4_cos512(value: u16) -> i32 {
    ((f64::from(value & 0x01FF) * std::f64::consts::TAU / 512.0).cos() * 32767.0).round() as i32
}

fn cx4_transform_wireframe(
    x: i16,
    y: i16,
    z: i16,
    rotate_x: u8,
    rotate_y: u8,
    rotate_z: u8,
    scale: u8,
) -> (i16, i16) {
    let c4x = f64::from(x);
    let c4y = f64::from(y);
    let c4z = f64::from(z) - 0x95 as f64;

    let angle_x = -cx4_angle128(rotate_x);
    let y2 = c4y * angle_x.cos() - c4z * angle_x.sin();
    let z2 = c4y * angle_x.sin() + c4z * angle_x.cos();

    let angle_y = -cx4_angle128(rotate_y);
    let x2 = c4x * angle_y.cos() + z2 * angle_y.sin();
    let z = c4x * -angle_y.sin() + z2 * angle_y.cos();

    let angle_z = -cx4_angle128(rotate_z);
    let x = x2 * angle_z.cos() - y2 * angle_z.sin();
    let y = x2 * angle_z.sin() + y2 * angle_z.cos();

    let projection = f64::from(scale) / (0x90 as f64 * (z + 0x95 as f64)) * 0x95 as f64;
    (
        cx4_saturating_trunc_i16(x * projection),
        cx4_saturating_trunc_i16(y * projection),
    )
}

fn cx4_transform_wireframe_2(
    x: i16,
    y: i16,
    z: i16,
    rotate_x: u8,
    rotate_y: u8,
    rotate_z: u8,
    scale: u8,
) -> (i16, i16) {
    let c4x = f64::from(x);
    let c4y = f64::from(y);
    let c4z = f64::from(z);

    let angle_x = -cx4_angle128(rotate_x);
    let y2 = c4y * angle_x.cos() - c4z * angle_x.sin();
    let z2 = c4y * angle_x.sin() + c4z * angle_x.cos();

    let angle_y = -cx4_angle128(rotate_y);
    let x2 = c4x * angle_y.cos() + z2 * angle_y.sin();
    let _z = c4x * -angle_y.sin() + z2 * angle_y.cos();

    let angle_z = -cx4_angle128(rotate_z);
    let x = x2 * angle_z.cos() - y2 * angle_z.sin();
    let y = x2 * angle_z.sin() + y2 * angle_z.cos();

    let projection = f64::from(scale) / 256.0;
    (
        cx4_saturating_trunc_i16(x * projection),
        cx4_saturating_trunc_i16(y * projection),
    )
}

fn cx4_saturating_trunc_i16(value: f64) -> i16 {
    if value.is_nan() {
        0
    } else {
        value.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
    }
}

fn cx4_calc_wireframe(x1: i16, y1: i16, x2: i16, y2: i16) -> (u16, i16, i16) {
    let mut dx = i32::from(x2) - i32::from(x1);
    let mut dy = i32::from(y2) - i32::from(y1);

    let distance = if dx.abs() > dy.abs() {
        let distance = dx.abs() + 1;
        dy = 256 * dy / dx.abs();
        dx = if dx < 0 { -256 } else { 256 };
        distance
    } else if dy != 0 {
        let distance = dy.abs() + 1;
        dx = 256 * dx / dy.abs();
        dy = if dy < 0 { -256 } else { 256 };
        distance
    } else {
        0
    };

    (distance.max(1) as u16, dx as i16, dy as i16)
}

fn cx4_rom_read(rom: &[u8], address: u32) -> u8 {
    lorom_rom_index(address, rom.len())
        .map(|index| rom[index])
        .unwrap_or(0)
}

fn cx4_rom_read_be_i16(rom: &[u8], address: u32) -> i16 {
    i16::from_be_bytes([
        cx4_rom_read(rom, address),
        cx4_rom_read(rom, address.wrapping_add(1)),
    ])
}

fn cx4_rom_read_point(rom: &[u8], address: u32) -> Cx4Point {
    Cx4Point {
        x: cx4_rom_read_be_i16(rom, address),
        y: cx4_rom_read_be_i16(rom, address.wrapping_add(2)),
        z: cx4_rom_read_be_i16(rom, address.wrapping_add(4)),
    }
}

fn cx4_angle512(value: u16) -> f64 {
    f64::from(value) * std::f64::consts::TAU / 512.0
}

fn cx4_tan(angle: f64) -> f64 {
    let cosine = angle.cos();
    if cosine.abs() < f64::EPSILON {
        f64::from(i32::MIN)
    } else {
        angle.sin() / cosine
    }
}

fn cx4_clip_trapezoid_span(left: i32, right: i32) -> (u8, u8) {
    if left < 0 && right < 0 {
        return (1, 0);
    }
    if left > 255 && right > 255 {
        return (255, 254);
    }

    (left.clamp(0, 255) as u8, right.clamp(0, 255) as u8)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Dsp1State {
    data: u8,
    status: u8,
    phase: Dsp1Phase,
    command: u8,
    expected_input_words: usize,
    input_low_byte: u8,
    input_words: Vec<u16>,
    output_words: Vec<u16>,
    output_index: usize,
    matrices: [[[i16; 3]; 3]; 3],
    projection: Dsp1ProjectionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dsp1Phase {
    WaitingCommand,
    ReadingData,
    WritingData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dsp1Operation {
    Multiply,
    Multiply2,
    MemoryTest,
    MemorySize,
    Radius,
    Range,
    Range2,
    Inverse,
    ProjectionParameter,
    Raster,
    ProjectObject,
    Target,
    SetMatrix(Dsp1MatrixKind),
    ObjectiveMatrix(Dsp1MatrixKind),
    SubjectiveMatrix(Dsp1MatrixKind),
    ScalarProduct(Dsp1MatrixKind),
    Trigonometric,
    Rotate2d,
    Rotate3d,
    AttitudeDelta,
    VectorLength,
    MemoryDump,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dsp1MatrixKind {
    A,
    B,
    C,
}

impl Dsp1MatrixKind {
    fn index(self) -> usize {
        match self {
            Self::A => 0,
            Self::B => 1,
            Self::C => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Dsp1ProjectionState {
    fx: i16,
    fy: i16,
    fz: i16,
    lfe: i16,
    les: i16,
    aas: u16,
    azs: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Dsp1CommandSpec {
    reads: usize,
    writes: usize,
    operation: Dsp1Operation,
}

const DSP1_STATUS_DRC: u8 = 0x04;
const DSP1_STATUS_DRS: u8 = 0x10;
const DSP1_STATUS_RQM: u8 = 0x80;
const DSP1_RESET_STATUS: u8 = DSP1_STATUS_DRC | DSP1_STATUS_RQM;

impl Dsp1State {
    fn new() -> Self {
        Self {
            data: 0,
            status: DSP1_RESET_STATUS,
            phase: Dsp1Phase::WaitingCommand,
            command: 0,
            expected_input_words: 0,
            input_low_byte: 0,
            input_words: Vec::new(),
            output_words: Vec::new(),
            output_index: 0,
            matrices: [[[0; 3]; 3]; 3],
            projection: Dsp1ProjectionState::default(),
        }
    }

    fn peek(&self, mapper_kind: MapperKind, address: u32) -> Option<u8> {
        let register_offset = dsp1_register_offset(mapper_kind, address)?;
        Some(if register_offset & 1 == 0 {
            self.peek_data()
        } else {
            self.status
        })
    }

    fn read(&mut self, mapper_kind: MapperKind, address: u32) -> Option<u8> {
        let register_offset = dsp1_register_offset(mapper_kind, address)?;
        Some(if register_offset & 1 == 0 {
            self.read_data()
        } else {
            self.status
        })
    }

    fn write(&mut self, mapper_kind: MapperKind, address: u32, value: u8) -> bool {
        if let Some(register_offset) = dsp1_register_offset(mapper_kind, address) {
            if register_offset & 1 == 0 {
                self.write_data(value);
            }
            true
        } else {
            false
        }
    }

    fn peek_data(&self) -> u8 {
        if self.phase != Dsp1Phase::WritingData {
            return self.data;
        }

        let word = self
            .output_words
            .get(self.output_index)
            .copied()
            .unwrap_or(0x0080);
        if self.status & DSP1_STATUS_DRS == 0 {
            word as u8
        } else {
            (word >> 8) as u8
        }
    }

    fn read_data(&mut self) -> u8 {
        if self.phase != Dsp1Phase::WritingData {
            return self.data;
        }

        let word = self
            .output_words
            .get(self.output_index)
            .copied()
            .unwrap_or(0x0080);
        if self.status & DSP1_STATUS_DRS == 0 {
            self.status |= DSP1_STATUS_DRS;
            let value = word as u8;
            self.data = value;
            return value;
        }

        self.status &= !DSP1_STATUS_DRS;
        let value = (word >> 8) as u8;
        self.data = value;
        self.output_index += 1;
        if self.output_index >= self.output_words.len() {
            self.finish_command();
        }
        value
    }

    fn write_data(&mut self, value: u8) {
        self.data = value;
        match self.phase {
            Dsp1Phase::WaitingCommand => self.start_command(value),
            Dsp1Phase::ReadingData => self.write_input_byte(value),
            Dsp1Phase::WritingData => {}
        }
    }

    fn start_command(&mut self, value: u8) {
        if value & 0xC0 != 0 {
            return;
        }

        self.command = value & 0x3F;
        let spec = dsp1_command_spec(self.command);

        self.status = DSP1_STATUS_RQM;
        self.expected_input_words = spec.reads;
        self.input_low_byte = 0;
        self.input_words.clear();
        self.output_words.clear();
        self.output_index = 0;
        if spec.reads == 0 {
            self.execute_command(spec);
        } else {
            self.phase = Dsp1Phase::ReadingData;
        }
    }

    fn write_input_byte(&mut self, value: u8) {
        if self.status & DSP1_STATUS_DRS == 0 {
            self.input_low_byte = value;
            self.status |= DSP1_STATUS_DRS;
            return;
        }

        self.status &= !DSP1_STATUS_DRS;
        self.input_words
            .push(u16::from_le_bytes([self.input_low_byte, value]));
        if self.input_words.len() >= self.expected_input_words {
            self.execute_command(dsp1_command_spec(self.command));
        }
    }

    fn execute_command(&mut self, spec: Dsp1CommandSpec) {
        self.output_words = match spec.operation {
            Dsp1Operation::Multiply => {
                vec![dsp1_multiply(self.input_words[0], self.input_words[1], 0)]
            }
            Dsp1Operation::Multiply2 => {
                vec![dsp1_multiply(self.input_words[0], self.input_words[1], 1)]
            }
            Dsp1Operation::MemoryTest => vec![0x0000],
            Dsp1Operation::MemorySize => vec![0x0100],
            Dsp1Operation::Radius => dsp1_radius(&self.input_words),
            Dsp1Operation::Range => vec![dsp1_range(&self.input_words, 0)],
            Dsp1Operation::Range2 => vec![dsp1_range(&self.input_words, 1)],
            Dsp1Operation::Inverse => dsp1_inverse(&self.input_words),
            Dsp1Operation::ProjectionParameter => {
                self.projection = Dsp1ProjectionState::from_words(&self.input_words);
                self.projection.parameter()
            }
            Dsp1Operation::Raster => self.projection.raster(self.input_words[0]),
            Dsp1Operation::ProjectObject => self.projection.project(&self.input_words),
            Dsp1Operation::Target => self.projection.target(&self.input_words),
            Dsp1Operation::SetMatrix(kind) => {
                self.matrices[kind.index()] = dsp1_attitude_matrix(&self.input_words);
                Vec::new()
            }
            Dsp1Operation::ObjectiveMatrix(kind) => {
                dsp1_objective_matrix(&self.matrices[kind.index()], &self.input_words)
            }
            Dsp1Operation::SubjectiveMatrix(kind) => {
                dsp1_subjective_matrix(&self.matrices[kind.index()], &self.input_words)
            }
            Dsp1Operation::ScalarProduct(kind) => {
                vec![dsp1_scalar_product(
                    &self.matrices[kind.index()],
                    &self.input_words,
                )]
            }
            Dsp1Operation::Trigonometric => dsp1_trigonometric(&self.input_words),
            Dsp1Operation::Rotate2d => dsp1_rotate_2d(&self.input_words),
            Dsp1Operation::Rotate3d => dsp1_rotate_3d(&self.input_words),
            Dsp1Operation::AttitudeDelta => dsp1_attitude_delta(&self.input_words),
            Dsp1Operation::VectorLength => vec![dsp1_vector_length(&self.input_words)],
            Dsp1Operation::MemoryDump => vec![0; spec.writes],
            Dsp1Operation::Unsupported => vec![0; spec.writes],
        };
        self.output_index = 0;
        self.status &= !DSP1_STATUS_DRS;
        if self.output_words.is_empty() {
            self.finish_command();
        } else {
            self.phase = Dsp1Phase::WritingData;
        }
    }

    fn finish_command(&mut self) {
        self.data = 0x80;
        self.status = DSP1_RESET_STATUS;
        self.phase = Dsp1Phase::WaitingCommand;
        self.expected_input_words = 0;
        self.input_words.clear();
        self.output_words.clear();
        self.output_index = 0;
    }
}

fn dsp1_command_spec(command: u8) -> Dsp1CommandSpec {
    use Dsp1MatrixKind as Matrix;
    use Dsp1Operation as Op;

    let (reads, writes, operation) = match command {
        0x00 => (2, 1, Op::Multiply),
        0x01 | 0x05 | 0x31 | 0x35 => (4, 0, Op::SetMatrix(Matrix::A)),
        0x11 | 0x15 => (4, 0, Op::SetMatrix(Matrix::B)),
        0x21 | 0x25 => (4, 0, Op::SetMatrix(Matrix::C)),
        0x02 | 0x12 | 0x22 | 0x32 => (7, 4, Op::ProjectionParameter),
        0x03 | 0x33 => (3, 3, Op::SubjectiveMatrix(Matrix::A)),
        0x13 => (3, 3, Op::SubjectiveMatrix(Matrix::B)),
        0x23 => (3, 3, Op::SubjectiveMatrix(Matrix::C)),
        0x04 | 0x24 => (2, 2, Op::Trigonometric),
        0x06 | 0x16 | 0x26 | 0x36 => (3, 3, Op::ProjectObject),
        0x07 | 0x0F => (1, 1, Op::MemoryTest),
        0x08 => (3, 2, Op::Radius),
        0x0A | 0x1A | 0x2A | 0x3A => (1, 4, Op::Raster),
        0x0B | 0x3B => (3, 1, Op::ScalarProduct(Matrix::A)),
        0x1B => (3, 1, Op::ScalarProduct(Matrix::B)),
        0x2B => (3, 1, Op::ScalarProduct(Matrix::C)),
        0x0C | 0x2C => (3, 2, Op::Rotate2d),
        0x09 | 0x0D | 0x39 | 0x3D => (3, 3, Op::ObjectiveMatrix(Matrix::A)),
        0x19 | 0x1D => (3, 3, Op::ObjectiveMatrix(Matrix::B)),
        0x29 | 0x2D => (3, 3, Op::ObjectiveMatrix(Matrix::C)),
        0x0E | 0x1E | 0x2E | 0x3E => (2, 2, Op::Target),
        0x10 | 0x30 => (2, 2, Op::Inverse),
        0x14 | 0x34 => (6, 3, Op::AttitudeDelta),
        0x1C | 0x3C => (6, 3, Op::Rotate3d),
        0x18 => (4, 1, Op::Range),
        0x17 | 0x1F | 0x37 | 0x3F => (1, 1024, Op::MemoryDump),
        0x20 => (2, 1, Op::Multiply2),
        0x28 => (3, 1, Op::VectorLength),
        0x27 | 0x2F => (1, 1, Op::MemorySize),
        0x38 => (4, 1, Op::Range2),
        _ => (0, 0, Op::Unsupported),
    };

    Dsp1CommandSpec {
        reads,
        writes,
        operation,
    }
}

fn dsp1_multiply(left: u16, right: u16, round: i32) -> u16 {
    let product = i32::from(left as i16) * i32::from(right as i16);
    ((product >> 15) + round) as i16 as u16
}

fn dsp1_radius(input_words: &[u16]) -> Vec<u16> {
    let sum = input_words
        .iter()
        .take(3)
        .map(|value| {
            let value = i64::from(*value as i16);
            value * value
        })
        .sum::<i64>() as u32;
    vec![sum as u16, (sum >> 16) as u16]
}

fn dsp1_range(input_words: &[u16], round: i64) -> u16 {
    let sum = input_words
        .iter()
        .take(3)
        .map(|value| {
            let value = i64::from(*value as i16);
            value * value
        })
        .sum::<i64>();
    let radius = i64::from(input_words.get(3).copied().unwrap_or(0) as i16);
    (((sum - radius * radius) >> 15) + round) as i16 as u16
}

fn dsp1_inverse(input_words: &[u16]) -> Vec<u16> {
    let coefficient = input_words[0] as i16;
    let mut exponent = input_words[1] as i16;
    if coefficient == 0 {
        return vec![0x7FFF, 0x002F];
    }

    let sign = if coefficient < 0 { -1 } else { 1 };
    let mut normalized = i32::from(coefficient);
    if normalized < 0 {
        normalized = (-normalized).min(i32::from(i16::MAX));
    }
    while normalized < 0x4000 {
        normalized <<= 1;
        exponent = exponent.wrapping_sub(1);
    }

    let reciprocal = if normalized == 0x4000 {
        if sign > 0 {
            i16::MAX
        } else {
            exponent = exponent.wrapping_sub(1);
            -0x4000
        }
    } else {
        let value = (536_870_912.0 / f64::from(normalized)).round() as i16;
        if sign < 0 { -value } else { value }
    };

    vec![reciprocal as u16, 1_i16.wrapping_sub(exponent) as u16]
}

impl Dsp1ProjectionState {
    fn from_words(input_words: &[u16]) -> Self {
        Self {
            fx: input_words[0] as i16,
            fy: input_words[1] as i16,
            fz: input_words[2] as i16,
            lfe: input_words[3] as i16,
            les: input_words[4] as i16,
            aas: input_words[5],
            azs: input_words[6],
        }
    }

    fn parameter(self) -> Vec<u16> {
        let axes = self.axes();
        let center = self.center(&axes);
        vec![
            0,
            dsp1_saturating_i16(f64::from(self.les) * axes.normal[2]),
            dsp1_saturating_i16(center[0]),
            dsp1_saturating_i16(center[1]),
        ]
    }

    fn raster(self, screen_line: u16) -> Vec<u16> {
        let axes = self.axes();
        let line = f64::from(screen_line as i16);
        let depth = (f64::from(self.les) + line * axes.normal[2]).max(1.0);
        let scale = f64::from(self.les) / depth * 256.0;
        vec![
            dsp1_saturating_i16(scale * axes.horizontal[0]),
            dsp1_saturating_i16(scale * -axes.vertical[0]),
            dsp1_saturating_i16(scale * axes.horizontal[1]),
            dsp1_saturating_i16(scale * axes.vertical[1]),
        ]
    }

    fn project(self, input_words: &[u16]) -> Vec<u16> {
        let axes = self.axes();
        let center = self.center(&axes);
        let point = [
            f64::from(input_words[0] as i16),
            f64::from(input_words[1] as i16),
            f64::from(input_words[2] as i16),
        ];
        let relative = [
            point[0] - center[0],
            point[1] - center[1],
            point[2] - center[2],
        ];
        let depth = (f64::from(self.les) + dot3(relative, axes.normal)).max(1.0);
        let scale = f64::from(self.les) / depth;
        vec![
            dsp1_saturating_i16(dot3(relative, axes.horizontal) * scale),
            dsp1_saturating_i16(dot3(relative, axes.vertical) * scale),
            dsp1_saturating_i16(scale * 256.0),
        ]
    }

    fn target(self, input_words: &[u16]) -> Vec<u16> {
        let axes = self.axes();
        let center = self.center(&axes);
        let h = f64::from(input_words[0] as i16);
        let v = f64::from(input_words[1] as i16);
        let target = [
            center[0] + h * axes.horizontal[0] + v * axes.vertical[0],
            center[1] + h * axes.horizontal[1] + v * axes.vertical[1],
        ];
        vec![
            dsp1_saturating_i16(target[0]),
            dsp1_saturating_i16(target[1]),
        ]
    }

    fn center(self, axes: &Dsp1ProjectionAxes) -> [f64; 3] {
        let lfe = f64::from(self.lfe);
        [
            f64::from(self.fx) + lfe * axes.normal[0],
            f64::from(self.fy) + lfe * axes.normal[1],
            f64::from(self.fz) + lfe * axes.normal[2],
        ]
    }

    fn axes(self) -> Dsp1ProjectionAxes {
        let aas = dsp1_angle(self.aas);
        let azs = dsp1_angle(self.azs);
        let sin_aas = aas.sin();
        let cos_aas = aas.cos();
        let sin_azs = azs.sin();
        let cos_azs = azs.cos();

        Dsp1ProjectionAxes {
            normal: [-sin_azs * sin_aas, sin_azs * cos_aas, cos_azs],
            horizontal: [cos_aas, sin_aas, 0.0],
            vertical: [-cos_azs * sin_aas, cos_azs * cos_aas, -sin_azs],
        }
    }
}

struct Dsp1ProjectionAxes {
    normal: [f64; 3],
    horizontal: [f64; 3],
    vertical: [f64; 3],
}

fn dot3(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn dsp1_attitude_matrix(input_words: &[u16]) -> [[i16; 3]; 3] {
    let scale = f64::from((input_words[0] as i16) >> 1);
    let z_angle = dsp1_angle(input_words[1]);
    let y_angle = dsp1_angle(input_words[2]);
    let x_angle = dsp1_angle(input_words[3]);
    let sin_z = z_angle.sin();
    let cos_z = z_angle.cos();
    let sin_y = y_angle.sin();
    let cos_y = y_angle.cos();
    let sin_x = x_angle.sin();
    let cos_x = x_angle.cos();

    [
        [
            dsp1_saturating_i16_value(scale * cos_z * cos_y),
            dsp1_saturating_i16_value(-(scale * sin_z * cos_y)),
            dsp1_saturating_i16_value(scale * sin_y),
        ],
        [
            dsp1_saturating_i16_value(scale * sin_z * cos_x + scale * cos_z * sin_x * sin_y),
            dsp1_saturating_i16_value(scale * cos_z * cos_x - scale * sin_z * sin_x * sin_y),
            dsp1_saturating_i16_value(-(scale * sin_x * cos_y)),
        ],
        [
            dsp1_saturating_i16_value(scale * sin_z * sin_x - scale * cos_z * cos_x * sin_y),
            dsp1_saturating_i16_value(scale * cos_z * sin_x + scale * sin_z * cos_x * sin_y),
            dsp1_saturating_i16_value(scale * cos_x * cos_y),
        ],
    ]
}

fn dsp1_objective_matrix(matrix: &[[i16; 3]; 3], input_words: &[u16]) -> Vec<u16> {
    let vector = dsp1_vector3(input_words);
    matrix
        .iter()
        .map(|row| {
            let sum: i64 = (0..3)
                .map(|index| i64::from(vector[index]) * i64::from(row[index]))
                .sum();
            dsp1_saturating_i16_i64(sum >> 15)
        })
        .collect()
}

fn dsp1_subjective_matrix(matrix: &[[i16; 3]; 3], input_words: &[u16]) -> Vec<u16> {
    let vector = dsp1_vector3(input_words);
    (0..3)
        .map(|column| {
            let sum: i64 = (0..3)
                .map(|row| i64::from(vector[row]) * i64::from(matrix[row][column]))
                .sum();
            dsp1_saturating_i16_i64(sum >> 15)
        })
        .collect()
}

fn dsp1_scalar_product(matrix: &[[i16; 3]; 3], input_words: &[u16]) -> u16 {
    let vector = dsp1_vector3(input_words);
    let sum: i64 = (0..3)
        .map(|index| i64::from(vector[index]) * i64::from(matrix[0][index]))
        .sum();
    dsp1_saturating_i16_i64(sum >> 15)
}

fn dsp1_trigonometric(input_words: &[u16]) -> Vec<u16> {
    let angle = dsp1_angle(input_words[0]);
    let radius = f64::from(input_words[1] as i16);
    vec![
        dsp1_saturating_i16(radius * angle.sin()),
        dsp1_saturating_i16(radius * angle.cos()),
    ]
}

fn dsp1_rotate_2d(input_words: &[u16]) -> Vec<u16> {
    let angle = dsp1_angle(input_words[0]);
    let x = f64::from(input_words[1] as i16);
    let y = f64::from(input_words[2] as i16);
    vec![
        dsp1_saturating_i16(y * angle.sin() + x * angle.cos()),
        dsp1_saturating_i16(y * angle.cos() - x * angle.sin()),
    ]
}

fn dsp1_rotate_3d(input_words: &[u16]) -> Vec<u16> {
    let z_angle = dsp1_angle(input_words[0]);
    let y_angle = dsp1_angle(input_words[1]);
    let x_angle = dsp1_angle(input_words[2]);
    let x = f64::from(input_words[3] as i16);
    let y = f64::from(input_words[4] as i16);
    let z = f64::from(input_words[5] as i16);

    let x_after_z = y * z_angle.sin() + x * z_angle.cos();
    let y_after_z = y * z_angle.cos() - x * z_angle.sin();

    let z_after_y = x_after_z * y_angle.sin() + z * y_angle.cos();
    let x_after_y = x_after_z * y_angle.cos() - z * y_angle.sin();

    let y_after_x = z_after_y * x_angle.sin() + y_after_z * x_angle.cos();
    let z_after_x = z_after_y * x_angle.cos() - y_after_z * x_angle.sin();

    vec![
        dsp1_saturating_i16(x_after_y),
        dsp1_saturating_i16(y_after_x),
        dsp1_saturating_i16(z_after_x),
    ]
}

fn dsp1_attitude_delta(input_words: &[u16]) -> Vec<u16> {
    let z_rotation = input_words[0] as i16;
    let x_rotation = input_words[1] as i16;
    let y_rotation = input_words[2] as i16;
    let u_delta = f64::from(input_words[3] as i16);
    let f_delta = f64::from(input_words[4] as i16);
    let l_delta = f64::from(input_words[5] as i16);

    let x_angle = dsp1_angle(input_words[1]);
    let y_angle = dsp1_angle(input_words[2]);
    let sin_y = y_angle.sin();
    let cos_y = y_angle.cos();
    let cos_x = x_angle.cos();
    let tan_x = x_angle.tan();

    let z_delta = if cos_x.abs() < f64::EPSILON {
        (u_delta * cos_y - f_delta * sin_y).signum() * f64::from(i16::MAX)
    } else {
        (u_delta * cos_y - f_delta * sin_y) / cos_x
    };
    let x_delta = u_delta * sin_y + f_delta * cos_y;
    let y_delta = l_delta - (u_delta * cos_y + f_delta * sin_y) * tan_x;

    vec![
        z_rotation.wrapping_add(dsp1_saturating_i16_value(z_delta)) as u16,
        x_rotation.wrapping_add(dsp1_saturating_i16_value(x_delta)) as u16,
        y_rotation.wrapping_add(dsp1_saturating_i16_value(y_delta)) as u16,
    ]
}

fn dsp1_vector_length(input_words: &[u16]) -> u16 {
    let sum = input_words
        .iter()
        .take(3)
        .map(|value| {
            let value = f64::from(*value as i16);
            value * value
        })
        .sum::<f64>();
    dsp1_saturating_i16(sum.sqrt())
}

fn dsp1_vector3(input_words: &[u16]) -> [i16; 3] {
    [
        input_words[0] as i16,
        input_words[1] as i16,
        input_words[2] as i16,
    ]
}

fn dsp1_angle(value: u16) -> f64 {
    f64::from(value as i16) * std::f64::consts::TAU / 65536.0
}

fn dsp1_saturating_i16(value: f64) -> u16 {
    dsp1_saturating_i16_value(value) as u16
}

fn dsp1_saturating_i16_value(value: f64) -> i16 {
    value
        .round()
        .clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
}

fn dsp1_saturating_i16_i64(value: i64) -> u16 {
    value.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16 as u16
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ByteWindow {
    start: u16,
    bytes: Vec<u8>,
}

impl ByteWindow {
    fn new(start: u16, len: usize) -> Self {
        Self {
            start,
            bytes: vec![0; len],
        }
    }

    fn read(&self, address_offset: u16) -> Option<u8> {
        self.index(address_offset).map(|index| self.bytes[index])
    }

    fn write(&mut self, address_offset: u16, value: u8) -> bool {
        if let Some(index) = self.index(address_offset) {
            self.bytes[index] = value;
            true
        } else {
            false
        }
    }

    fn contains(&self, address_offset: u16) -> bool {
        self.index(address_offset).is_some()
    }

    fn index(&self, address_offset: u16) -> Option<usize> {
        let relative = address_offset.checked_sub(self.start)? as usize;
        (relative < self.bytes.len()).then_some(relative)
    }
}

fn dsp1_register_offset(mapper_kind: MapperKind, address: u32) -> Option<u16> {
    let bank = bank(address);
    let offset = offset(address);

    match mapper_kind {
        MapperKind::LoRom => {
            if matches!(bank, 0x20..=0x3F | 0xA0..=0xBF) && offset >= 0x8000 {
                Some(offset - 0x8000)
            } else {
                None
            }
        }
        MapperKind::HiRom => {
            if matches!(bank, 0x00..=0x1F | 0x80..=0x9F) && (0x6000..=0x7FFF).contains(&offset) {
                Some(offset - 0x6000)
            } else {
                None
            }
        }
        MapperKind::Sa1 => None,
    }
}

fn is_system_bank(address: u32) -> bool {
    matches!(bank(address), 0x00..=0x3F | 0x80..=0xBF)
}

fn is_sa1_rom_address(address: u32) -> bool {
    (address & 0x408000) == 0x008000 || (address & 0xC00000) == 0xC00000
}

fn bank(address: u32) -> u8 {
    ((address >> 16) & 0xFF) as u8
}

fn offset(address: u32) -> u16 {
    (address & 0xFFFF) as u16
}
