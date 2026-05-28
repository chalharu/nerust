use crate::enhancement::{EnhancementChip, EnhancementState};
use crate::mapper::{HiRomMapper, LoRomMapper, Mapper, MapperKind, Sa1Mapper, superfx_ram_index};

const COPIER_HEADER_LEN: usize = 512;
const LOROM_HEADER_OFFSET: usize = 0x7FC0;
const LOROM_RESET_VECTOR_OFFSET: usize = 0x7FFC;
const HIROM_HEADER_OFFSET: usize = 0xFFC0;
const HIROM_RESET_VECTOR_OFFSET: usize = 0xFFFC;
const HEADER_TITLE_LEN: usize = 21;
const LOROM_MAP_MODE_MASK: u8 = 0x2F;
const LOROM_MAP_MODE_VALUE: u8 = 0x20;
const SA1_MAP_MODE_VALUE: u8 = 0x23;
const HIROM_MAP_MODE_MASK: u8 = 0x2F;
const HIROM_MAP_MODE_VALUE: u8 = 0x21;
const MAX_RAM_SIZE_CODE: u8 = 0x08;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CartridgeError {
    #[error(
        "ROM size must be an even multiple of 32 KiB, optionally plus a 512-byte copier header"
    )]
    InvalidRomSize,
    #[error("ROM is too small to contain a supported SNES header")]
    MissingHeader,
    #[error("unsupported SNES map mode 0x{0:02X}")]
    UnsupportedMapMode(u8),
    #[error("unsupported SNES cartridge RAM size code 0x{0:02X}")]
    UnsupportedRamSizeCode(u8),
    #[error("invalid SNES save RAM size: expected {expected} bytes, got {actual}")]
    InvalidSaveRamSize { expected: usize, actual: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CartridgeHeader {
    title: String,
    map_mode: u8,
    chipset: u8,
    expansion_chip_subtype: Option<u8>,
    enhancement_chip: EnhancementChip,
    rom_size_code: u8,
    ram_size_code: u8,
    reset_vector: u16,
    has_copier_header: bool,
    mapper_kind: MapperKind,
}

impl CartridgeHeader {
    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn map_mode(&self) -> u8 {
        self.map_mode
    }

    pub fn chipset(&self) -> u8 {
        self.chipset
    }

    pub fn expansion_chip_subtype(&self) -> Option<u8> {
        self.expansion_chip_subtype
    }

    pub fn enhancement_chip(&self) -> EnhancementChip {
        self.enhancement_chip
    }

    pub fn rom_size_code(&self) -> u8 {
        self.rom_size_code
    }

    pub fn ram_size_code(&self) -> u8 {
        self.ram_size_code
    }

    pub fn reset_vector(&self) -> u16 {
        self.reset_vector
    }

    pub fn has_copier_header(&self) -> bool {
        self.has_copier_header
    }

    pub fn mapper_kind(&self) -> MapperKind {
        self.mapper_kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cartridge {
    rom: Box<[u8]>,
    save_ram: Box<[u8]>,
    header: CartridgeHeader,
    mapper: Mapper,
    enhancement: EnhancementState,
}

impl Cartridge {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CartridgeError> {
        let (rom, has_copier_header) = strip_copier_header(bytes)?;
        if rom.len() <= LOROM_RESET_VECTOR_OFFSET + 1 {
            return Err(CartridgeError::MissingHeader);
        }

        let Some((header, mapper)) = Self::parse_header(rom, has_copier_header) else {
            return Err(CartridgeError::UnsupportedMapMode(
                rom[LOROM_HEADER_OFFSET + 0x15],
            ));
        };

        let save_ram = vec![0; ram_size_bytes(header.ram_size_code)?].into_boxed_slice();

        let enhancement = EnhancementState::from_chip(header.enhancement_chip());

        Ok(Self {
            rom: rom.to_vec().into_boxed_slice(),
            save_ram,
            header,
            mapper,
            enhancement,
        })
    }

    fn parse_header(rom: &[u8], has_copier_header: bool) -> Option<(CartridgeHeader, Mapper)> {
        Self::parse_header_at(
            rom,
            has_copier_header,
            LOROM_HEADER_OFFSET,
            LOROM_RESET_VECTOR_OFFSET,
            MapperKind::Sa1,
            Mapper::Sa1(Sa1Mapper),
        )
        .or_else(|| {
            Self::parse_header_at(
                rom,
                has_copier_header,
                LOROM_HEADER_OFFSET,
                LOROM_RESET_VECTOR_OFFSET,
                MapperKind::LoRom,
                Mapper::LoRom(LoRomMapper),
            )
        })
        .or_else(|| {
            Self::parse_header_at(
                rom,
                has_copier_header,
                HIROM_HEADER_OFFSET,
                HIROM_RESET_VECTOR_OFFSET,
                MapperKind::HiRom,
                Mapper::HiRom(HiRomMapper),
            )
        })
    }

    fn parse_header_at(
        rom: &[u8],
        has_copier_header: bool,
        header_offset: usize,
        reset_vector_offset: usize,
        mapper_kind: MapperKind,
        mapper: Mapper,
    ) -> Option<(CartridgeHeader, Mapper)> {
        if rom.len() <= reset_vector_offset + 1 {
            return None;
        }
        let map_mode = rom[header_offset + 0x15];
        let chipset = rom[header_offset + 0x16];
        if !Self::supported_map_mode(map_mode, chipset, mapper_kind) {
            return None;
        }

        let title_bytes = &rom[header_offset..header_offset + HEADER_TITLE_LEN];
        let title = String::from_utf8_lossy(title_bytes)
            .trim_end_matches(char::from(0))
            .trim_end()
            .to_owned();
        let reset_vector =
            u16::from_le_bytes([rom[reset_vector_offset], rom[reset_vector_offset + 1]]);
        let expansion_chip_subtype = expansion_chip_subtype(rom, header_offset, chipset);
        let enhancement_chip = enhancement_chip_for_header(
            map_mode,
            chipset,
            expansion_chip_subtype,
            rom[header_offset + 0x17],
        );

        Some((
            CartridgeHeader {
                title,
                map_mode,
                chipset,
                expansion_chip_subtype,
                enhancement_chip,
                rom_size_code: rom[header_offset + 0x17],
                ram_size_code: rom[header_offset + 0x18],
                reset_vector,
                has_copier_header,
                mapper_kind,
            },
            mapper,
        ))
    }

    fn supported_map_mode(map_mode: u8, chipset: u8, mapper_kind: MapperKind) -> bool {
        match mapper_kind {
            MapperKind::LoRom => map_mode & LOROM_MAP_MODE_MASK == LOROM_MAP_MODE_VALUE,
            MapperKind::HiRom => map_mode & HIROM_MAP_MODE_MASK == HIROM_MAP_MODE_VALUE,
            MapperKind::Sa1 => {
                map_mode & LOROM_MAP_MODE_MASK == SA1_MAP_MODE_VALUE && is_sa1_chipset(chipset)
            }
        }
    }

    pub fn header(&self) -> &CartridgeHeader {
        &self.header
    }

    pub fn read(&self, address: u32) -> Option<u8> {
        self.peek(address)
    }

    pub(crate) fn read_mut(&mut self, address: u32) -> Option<u8> {
        if let Some(value) = self.enhancement.read(
            self.header.mapper_kind(),
            address,
            &self.rom,
            &self.save_ram,
        ) {
            return Some(value);
        }
        self.read_mapped(address)
    }

    fn peek(&self, address: u32) -> Option<u8> {
        if let Some(value) = self.enhancement.peek(
            self.header.mapper_kind(),
            address,
            &self.rom,
            &self.save_ram,
        ) {
            return Some(value);
        }
        self.read_mapped(address)
    }

    fn read_mapped(&self, address: u32) -> Option<u8> {
        if let EnhancementState::Sa1(state) = &self.enhancement {
            if let Some(index) = state.sa1_bwram_index(address, self.save_ram.len()) {
                return Some(self.save_ram[index]);
            }
            return state
                .sa1_banked_rom_index(address, self.rom.len())
                .map(|index| self.rom[index]);
        }

        if self.header.enhancement_chip().is_superfx()
            && let Some(index) = superfx_ram_index(address, self.save_ram.len())
        {
            return Some(self.save_ram[index]);
        }

        self.mapper.read(&self.rom, &self.save_ram, address)
    }

    pub fn write(&mut self, address: u32, value: u8) -> bool {
        if self.enhancement.write(
            self.header.mapper_kind(),
            address,
            value,
            &self.rom,
            &mut self.save_ram,
        ) {
            return true;
        }
        if self.header.enhancement_chip().is_superfx()
            && let Some(index) = superfx_ram_index(address, self.save_ram.len())
        {
            self.save_ram[index] = value;
            return true;
        }
        if let EnhancementState::Sa1(state) = &self.enhancement
            && let Some(index) = state.sa1_bwram_index(address, self.save_ram.len())
        {
            if state.can_write_sa1_bwram(address) {
                self.save_ram[index] = value;
            }
            return true;
        }

        self.mapper.write_ram(&mut self.save_ram, address, value)
    }

    pub fn rom_len(&self) -> usize {
        self.rom.len()
    }

    pub fn save_ram(&self) -> &[u8] {
        &self.save_ram
    }

    pub fn load_save_ram(&mut self, save_ram: &[u8]) -> Result<(), CartridgeError> {
        if save_ram.len() != self.save_ram.len() {
            return Err(CartridgeError::InvalidSaveRamSize {
                expected: self.save_ram.len(),
                actual: save_ram.len(),
            });
        }

        self.save_ram.copy_from_slice(save_ram);
        Ok(())
    }
}

fn cartridge_coprocessor(chipset: u8) -> u8 {
    chipset >> 4
}

fn cartridge_features(chipset: u8) -> u8 {
    chipset & 0x0F
}

fn is_sa1_chipset(chipset: u8) -> bool {
    matches!(chipset, 0x34 | 0x35)
}

fn has_coprocessor(chipset: u8) -> bool {
    cartridge_features(chipset) >= 0x03
}

fn expansion_chip_subtype(rom: &[u8], header_offset: usize, chipset: u8) -> Option<u8> {
    if !has_coprocessor(chipset) || cartridge_coprocessor(chipset) != 0x0F {
        return None;
    }
    header_offset
        .checked_sub(1)
        .and_then(|offset| rom.get(offset))
        .copied()
}

fn enhancement_chip_for_header(
    map_mode: u8,
    chipset: u8,
    expansion_chip_subtype: Option<u8>,
    rom_size_code: u8,
) -> EnhancementChip {
    if !has_coprocessor(chipset) {
        return EnhancementChip::None;
    }

    match cartridge_coprocessor(chipset) {
        0x0 => EnhancementChip::Dsp1Family,
        0x1 => {
            if chipset == 0x1A || rom_size_code > 0x0A {
                EnhancementChip::SuperFxGsu2
            } else {
                EnhancementChip::SuperFxGsu1
            }
        }
        0x3 if map_mode & LOROM_MAP_MODE_MASK == SA1_MAP_MODE_VALUE && is_sa1_chipset(chipset) => {
            EnhancementChip::Sa1
        }
        0xF if matches!(expansion_chip_subtype, Some(0x03 | 0x10)) => EnhancementChip::Cx4,
        _ => EnhancementChip::None,
    }
}

fn strip_copier_header(bytes: &[u8]) -> Result<(&[u8], bool), CartridgeError> {
    match bytes.len() % 0x8000 {
        0 => Ok((bytes, false)),
        COPIER_HEADER_LEN => Ok((&bytes[COPIER_HEADER_LEN..], true)),
        _ => Err(CartridgeError::InvalidRomSize),
    }
}

fn ram_size_bytes(code: u8) -> Result<usize, CartridgeError> {
    if code == 0 {
        return Ok(0);
    }
    if code > MAX_RAM_SIZE_CODE {
        return Err(CartridgeError::UnsupportedRamSizeCode(code));
    }

    Ok(1024usize << code)
}

#[cfg(test)]
mod tests {
    use super::{Cartridge, CartridgeError};
    use crate::{EnhancementChip, MapperKind};

    const HEADER_OFFSET: usize = 0x7FC0;
    const RESET_VECTOR_OFFSET: usize = 0x7FFC;
    const HIROM_HEADER_OFFSET: usize = 0xFFC0;
    const HIROM_RESET_VECTOR_OFFSET: usize = 0xFFFC;

    fn build_lorom() -> Vec<u8> {
        let mut rom = vec![0; 0x10000];
        rom[HEADER_OFFSET..HEADER_OFFSET + 21].copy_from_slice(b"CPU TEST HEADER      ");
        rom[0x7FD5] = 0x30;
        rom[0x7FD7] = 0x08;
        rom[0x7FD8] = 0x03;
        rom[RESET_VECTOR_OFFSET..RESET_VECTOR_OFFSET + 2]
            .copy_from_slice(&0x8000_u16.to_le_bytes());
        rom[0x0000] = 0xEA;
        rom[0x8000] = 0xA2;
        rom
    }

    fn build_lorom_with_header(
        title: &str,
        map_mode: u8,
        chipset: u8,
        expansion_chip_subtype: Option<u8>,
        rom_size_code: u8,
    ) -> Vec<u8> {
        let mut rom = build_lorom();
        write_title(&mut rom, HEADER_OFFSET, title);
        rom[HEADER_OFFSET + 0x15] = map_mode;
        rom[HEADER_OFFSET + 0x16] = chipset;
        rom[HEADER_OFFSET + 0x17] = rom_size_code;
        if let Some(subtype) = expansion_chip_subtype {
            rom[HEADER_OFFSET - 1] = subtype;
        }
        rom
    }

    fn build_sa1_rom(rom_len: usize, ram_size_code: u8) -> Vec<u8> {
        let mut rom = build_lorom_with_header("SA1 MAPPER", 0x23, 0x34, None, 0x0C);
        rom.resize(rom_len, 0);
        rom[HEADER_OFFSET + 0x18] = ram_size_code;
        rom
    }

    fn build_hirom() -> Vec<u8> {
        let mut rom = vec![0; 0x20000];
        rom[HIROM_HEADER_OFFSET..HIROM_HEADER_OFFSET + 21]
            .copy_from_slice(b"HIROM TEST HEADER    ");
        rom[0xFFD5] = 0x31;
        rom[0xFFD7] = 0x09;
        rom[0xFFD8] = 0x03;
        rom[HIROM_RESET_VECTOR_OFFSET..HIROM_RESET_VECTOR_OFFSET + 2]
            .copy_from_slice(&0x8000_u16.to_le_bytes());
        rom[0x8000] = 0xEA;
        rom[0x10000] = 0xA2;
        rom
    }

    fn build_hirom_with_header(
        title: &str,
        map_mode: u8,
        chipset: u8,
        expansion_chip_subtype: Option<u8>,
        rom_size_code: u8,
    ) -> Vec<u8> {
        let mut rom = build_hirom();
        write_title(&mut rom, HIROM_HEADER_OFFSET, title);
        rom[HIROM_HEADER_OFFSET + 0x15] = map_mode;
        rom[HIROM_HEADER_OFFSET + 0x16] = chipset;
        rom[HIROM_HEADER_OFFSET + 0x17] = rom_size_code;
        if let Some(subtype) = expansion_chip_subtype {
            rom[HIROM_HEADER_OFFSET - 1] = subtype;
        }
        rom
    }

    fn write_title(rom: &mut [u8], header_offset: usize, title: &str) {
        let mut title_bytes = [b' '; 21];
        for (target, source) in title_bytes.iter_mut().zip(title.as_bytes()) {
            *target = *source;
        }
        rom[header_offset..header_offset + title_bytes.len()].copy_from_slice(&title_bytes);
    }

    #[test]
    fn parses_lorom_header_and_supports_copier_header_stripping() {
        let rom = build_lorom();
        let mut with_copier_header = vec![0; 512];
        with_copier_header.extend_from_slice(&rom);

        let cartridge = Cartridge::from_bytes(&with_copier_header).unwrap();

        assert_eq!(cartridge.header().title(), "CPU TEST HEADER");
        assert_eq!(cartridge.header().map_mode(), 0x30);
        assert_eq!(cartridge.header().chipset(), 0x00);
        assert_eq!(cartridge.header().expansion_chip_subtype(), None);
        assert_eq!(cartridge.header().enhancement_chip(), EnhancementChip::None);
        assert_eq!(cartridge.header().rom_size_code(), 0x08);
        assert_eq!(cartridge.header().ram_size_code(), 0x03);
        assert_eq!(cartridge.header().reset_vector(), 0x8000);
        assert!(cartridge.header().has_copier_header());
        assert_eq!(cartridge.header().mapper_kind(), MapperKind::LoRom);
        assert_eq!(cartridge.save_ram().len(), 8 * 1024);
        assert_eq!(cartridge.read(0x008000), Some(0xEA));
        assert_eq!(cartridge.read(0x018000), Some(0xA2));
        assert_eq!(cartridge.read(0x808000), Some(0xEA));
    }

    #[test]
    fn parses_hirom_header_and_maps_64k_rom_banks() {
        let cartridge = Cartridge::from_bytes(&build_hirom()).unwrap();

        assert_eq!(cartridge.header().title(), "HIROM TEST HEADER");
        assert_eq!(cartridge.header().map_mode(), 0x31);
        assert_eq!(cartridge.header().chipset(), 0x00);
        assert_eq!(cartridge.header().enhancement_chip(), EnhancementChip::None);
        assert_eq!(cartridge.header().rom_size_code(), 0x09);
        assert_eq!(cartridge.header().ram_size_code(), 0x03);
        assert_eq!(cartridge.header().reset_vector(), 0x8000);
        assert_eq!(cartridge.header().mapper_kind(), MapperKind::HiRom);
        assert_eq!(cartridge.save_ram().len(), 8 * 1024);
        assert_eq!(cartridge.read(0x008000), Some(0xEA));
        assert_eq!(cartridge.read(0xC08000), Some(0xEA));
        assert_eq!(cartridge.read(0xC10000), Some(0xA2));
    }

    #[test]
    fn lorom_sram_reads_writes_and_mirrors() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom()).unwrap();

        assert_eq!(cartridge.read(0x700123), Some(0x00));
        assert!(cartridge.write(0x700123, 0x5A));
        assert_eq!(cartridge.read(0x700123), Some(0x5A));
        assert_eq!(cartridge.read(0x702123), Some(0x5A));
        assert_eq!(cartridge.read(0xF00123), Some(0x5A));
        assert!(!cartridge.write(0x708000, 0xC3));
    }

    #[test]
    fn hirom_sram_reads_writes_and_mirrors() {
        let mut cartridge = Cartridge::from_bytes(&build_hirom()).unwrap();

        assert_eq!(cartridge.read(0x206123), Some(0x00));
        assert!(cartridge.write(0x206123, 0xA5));
        assert_eq!(cartridge.read(0x206123), Some(0xA5));
        assert_eq!(cartridge.read(0x216123), Some(0xA5));
        assert_eq!(cartridge.read(0xA06123), Some(0xA5));
        assert!(!cartridge.write(0x208000, 0xC3));
    }

    #[test]
    fn save_ram_can_be_restored_from_persisted_bytes() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom()).unwrap();
        let mut save_ram = vec![0x5A; cartridge.save_ram().len()];
        save_ram[0x0123] = 0xC3;

        cartridge.load_save_ram(&save_ram).unwrap();

        assert_eq!(cartridge.save_ram()[0x0123], 0xC3);
        assert_eq!(cartridge.read(0x700123), Some(0xC3));
        assert_eq!(cartridge.read(0x702123), Some(0xC3));
    }

    #[test]
    fn save_ram_restore_rejects_size_mismatch() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom()).unwrap();

        assert_eq!(
            cartridge.load_save_ram(&[0x5A]).unwrap_err(),
            CartridgeError::InvalidSaveRamSize {
                expected: 8 * 1024,
                actual: 1
            }
        );
    }

    #[test]
    fn rejects_unsupported_ram_size_codes() {
        let mut rom = build_lorom();
        rom[0x7FD8] = 0x09;

        assert_eq!(
            Cartridge::from_bytes(&rom).unwrap_err(),
            CartridgeError::UnsupportedRamSizeCode(0x09)
        );
    }

    #[test]
    fn rejects_non_lorom_headers() {
        let mut rom = build_lorom();
        rom[0x7FD5] = 0x21;
        assert_eq!(
            Cartridge::from_bytes(&rom).unwrap_err(),
            CartridgeError::UnsupportedMapMode(0x21)
        );
    }

    #[test]
    fn detects_requested_enhancement_chips_from_headers() {
        for (title, mapper_kind, map_mode, chipset, subtype, rom_size_code, expected) in [
            (
                "SA1 TEST HEADER",
                MapperKind::Sa1,
                0x23,
                0x34,
                None,
                0x0A,
                EnhancementChip::Sa1,
            ),
            (
                "GSU1 TEST HEADER",
                MapperKind::LoRom,
                0x20,
                0x13,
                None,
                0x0A,
                EnhancementChip::SuperFxGsu1,
            ),
            (
                "GSU2 TEST HEADER",
                MapperKind::LoRom,
                0x20,
                0x1A,
                None,
                0x0C,
                EnhancementChip::SuperFxGsu2,
            ),
            (
                "CX4 TEST HEADER",
                MapperKind::LoRom,
                0x20,
                0xF3,
                Some(0x03),
                0x0A,
                EnhancementChip::Cx4,
            ),
            (
                "CX4 HITACHI HEADER",
                MapperKind::LoRom,
                0x20,
                0xF3,
                Some(0x10),
                0x0A,
                EnhancementChip::Cx4,
            ),
            (
                "DSP1 TEST HEADER",
                MapperKind::LoRom,
                0x20,
                0x03,
                None,
                0x0A,
                EnhancementChip::Dsp1Family,
            ),
            (
                "DSP1B TEST HEADER",
                MapperKind::HiRom,
                0x21,
                0x05,
                None,
                0x0A,
                EnhancementChip::Dsp1Family,
            ),
        ] {
            let rom = match mapper_kind {
                MapperKind::LoRom | MapperKind::Sa1 => {
                    build_lorom_with_header(title, map_mode, chipset, subtype, rom_size_code)
                }
                MapperKind::HiRom => {
                    build_hirom_with_header(title, map_mode, chipset, subtype, rom_size_code)
                }
            };
            let cartridge = Cartridge::from_bytes(&rom).unwrap();

            assert_eq!(cartridge.header().mapper_kind(), mapper_kind);
            assert_eq!(cartridge.header().chipset(), chipset);
            assert_eq!(cartridge.header().expansion_chip_subtype(), subtype);
            assert_eq!(cartridge.header().enhancement_chip(), expected);
        }
    }

    #[test]
    fn rejects_sa1_map_mode_without_sa1_chipset() {
        let rom = build_lorom_with_header("BAD SA1 HEADER", 0x23, 0x00, None, 0x0A);

        assert_eq!(
            Cartridge::from_bytes(&rom).unwrap_err(),
            CartridgeError::UnsupportedMapMode(0x23)
        );
    }

    #[test]
    fn rejects_sa1_map_mode_with_unverified_sa1_family_chipset() {
        let rom = build_lorom_with_header("BAD SA1 CHIPSET", 0x23, 0x33, None, 0x0A);

        assert_eq!(
            Cartridge::from_bytes(&rom).unwrap_err(),
            CartridgeError::UnsupportedMapMode(0x23)
        );
    }

    #[test]
    fn does_not_report_sa1_without_sa1_map_mode() {
        let cartridge = Cartridge::from_bytes(&build_lorom_with_header(
            "PLAIN LOROM",
            0x20,
            0x34,
            None,
            0x0A,
        ))
        .unwrap();

        assert_eq!(cartridge.header().enhancement_chip(), EnhancementChip::None);
    }

    #[test]
    fn sa1_register_and_iram_windows_are_accessible_without_hiding_rom() {
        let mut cartridge =
            Cartridge::from_bytes(&build_lorom_with_header("SA1 MMIO", 0x23, 0x34, None, 0x0A))
                .unwrap();

        assert_eq!(cartridge.read(0x002200), Some(0x00));
        assert!(cartridge.write(0x002200, 0x5A));
        assert_eq!(cartridge.read(0x002200), Some(0x5A));
        assert_eq!(cartridge.read(0x802200), Some(0x5A));

        assert_eq!(cartridge.read(0x003000), Some(0x00));
        assert!(cartridge.write(0x003000, 0xC3));
        assert_eq!(cartridge.read(0x003000), Some(0xC3));

        assert_eq!(cartridge.read(0xC08000), Some(0xA2));
    }

    #[test]
    fn sa1_bwram_maps_direct_and_system_windows() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom_with_header(
            "SA1 BWRAM",
            0x23,
            0x34,
            None,
            0x0A,
        ))
        .unwrap();

        assert_eq!(cartridge.read(0x006000), Some(0x00));
        assert!(cartridge.write(0x002226, 0x80));
        assert!(cartridge.write(0x006000, 0x5A));
        assert_eq!(cartridge.read(0x006000), Some(0x5A));
        assert_eq!(cartridge.read(0x806000), Some(0x5A));
        assert_eq!(cartridge.read(0x400000), Some(0x5A));

        assert!(cartridge.write(0x407FFF, 0xC3));
        assert_eq!(cartridge.read(0x007FFF), Some(0xC3));
        assert_eq!(cartridge.read(0x008000), Some(0xEA));
    }

    #[test]
    fn sa1_bwram_write_protection_requires_enable_or_unprotected_range() {
        let mut cartridge = Cartridge::from_bytes(&build_sa1_rom(0x10000, 0x04)).unwrap();

        assert_eq!(cartridge.read(0x002226), Some(0x00));
        assert_eq!(cartridge.read(0x002227), Some(0x00));
        assert_eq!(cartridge.read(0x002228), Some(0x0F));
        assert!(cartridge.write(0x006000, 0xAA));
        assert_eq!(cartridge.read(0x006000), Some(0x00));

        assert!(cartridge.write(0x002228, 0x00));
        assert!(cartridge.write(0x006000, 0x11));
        assert_eq!(cartridge.read(0x006000), Some(0x00));
        assert!(cartridge.write(0x006100, 0x22));
        assert_eq!(cartridge.read(0x006100), Some(0x22));

        assert!(cartridge.write(0x002226, 0x80));
        assert!(cartridge.write(0x006000, 0x33));
        assert_eq!(cartridge.read(0x006000), Some(0x33));

        assert!(cartridge.write(0x002226, 0x00));
        assert!(cartridge.write(0x002227, 0x80));
        assert!(cartridge.write(0x006000, 0x44));
        assert_eq!(cartridge.read(0x006000), Some(0x44));
    }

    #[test]
    fn sa1_super_mmc_physical_banks_use_runtime_selectors() {
        let mut rom = build_sa1_rom(0x400000, 0x03);
        rom[0x000000] = 0x11;
        rom[0x100000] = 0x22;
        rom[0x200000] = 0x33;
        rom[0x300000] = 0x44;
        let mut cartridge = Cartridge::from_bytes(&rom).unwrap();

        assert_eq!(cartridge.read(0x002220), Some(0x00));
        assert_eq!(cartridge.read(0x002221), Some(0x01));
        assert_eq!(cartridge.read(0x002222), Some(0x02));
        assert_eq!(cartridge.read(0x002223), Some(0x03));
        assert_eq!(cartridge.read(0x002228), Some(0x0F));
        assert_eq!(cartridge.read(0xC00000), Some(0x11));
        assert_eq!(cartridge.read(0xD00000), Some(0x22));
        assert_eq!(cartridge.read(0xE00000), Some(0x33));
        assert_eq!(cartridge.read(0xF00000), Some(0x44));

        assert!(cartridge.write(0x002220, 0x03));
        assert!(cartridge.write(0x002221, 0x00));
        assert!(cartridge.write(0x002222, 0x01));
        assert!(cartridge.write(0x002223, 0x02));

        assert_eq!(cartridge.read(0xC00000), Some(0x44));
        assert_eq!(cartridge.read(0xD00000), Some(0x11));
        assert_eq!(cartridge.read(0xE00000), Some(0x22));
        assert_eq!(cartridge.read(0xF00000), Some(0x33));
    }

    #[test]
    fn sa1_super_mmc_lorom_mirrors_require_xmode() {
        let mut rom = build_sa1_rom(0x400000, 0x03);
        rom[0x000000] = 0x11;
        rom[0x100000] = 0x22;
        rom[0x300000] = 0x44;
        let mut cartridge = Cartridge::from_bytes(&rom).unwrap();

        assert_eq!(cartridge.read(0x008000), Some(0x11));
        assert_eq!(cartridge.read(0x208000), Some(0x22));
        assert_eq!(cartridge.read(0x808000), Some(0x11));

        assert!(cartridge.write(0x002220, 0x03));
        assert_eq!(cartridge.read(0x008000), Some(0x11));

        assert!(cartridge.write(0x002220, 0x83));
        assert_eq!(cartridge.read(0x008000), Some(0x44));
        assert_eq!(cartridge.read(0x808000), Some(0x44));

        assert!(cartridge.write(0x002221, 0x80));
        assert_eq!(cartridge.read(0x208000), Some(0x11));

        assert!(cartridge.write(0x002221, 0x01));
        assert_eq!(cartridge.read(0x208000), Some(0x22));
    }

    #[test]
    fn sa1_bmaps_shifts_system_bwram_window() {
        let mut cartridge = Cartridge::from_bytes(&build_sa1_rom(0x10000, 0x04)).unwrap();

        assert_eq!(cartridge.save_ram().len(), 16 * 1024);
        assert!(cartridge.write(0x002226, 0x80));
        assert!(cartridge.write(0x006000, 0x11));
        assert_eq!(cartridge.read(0x400000), Some(0x11));

        assert!(cartridge.write(0x002224, 0x01));
        assert_eq!(cartridge.read(0x002224), Some(0x01));
        assert_eq!(cartridge.read(0x006000), Some(0x00));
        assert!(cartridge.write(0x006000, 0x22));
        assert_eq!(cartridge.read(0x806000), Some(0x22));
        assert_eq!(cartridge.read(0x402000), Some(0x22));
        assert_eq!(cartridge.read(0x400000), Some(0x11));

        assert!(cartridge.write(0x002224, 0x00));
        assert_eq!(cartridge.read(0x006000), Some(0x11));
    }

    #[test]
    fn sa1_variable_length_data_reads_rom_and_auto_increments() {
        let mut rom = build_sa1_rom(0x400000, 0x03);
        rom[0x000000] = 0x12;
        rom[0x000001] = 0x34;
        rom[0x000002] = 0x56;
        rom[0x200000] = 0x77;
        let mut cartridge = Cartridge::from_bytes(&rom).unwrap();

        write_u24(&mut cartridge, 0x002259, 0xC00000);
        assert!(cartridge.write(0x002258, 0x84));
        assert_eq!(cartridge.read(0x00230C), Some(0x12));
        assert_eq!(cartridge.read_mut(0x00230D), Some(0x34));
        assert_eq!(cartridge.read_mut(0x00230C), Some(0x41));
        assert_eq!(cartridge.read_mut(0x00230D), Some(0x63));
        assert_eq!(cartridge.read_mut(0x00230C), Some(0x34));

        write_u24(&mut cartridge, 0x002259, 0x808000);
        assert_eq!(cartridge.read(0x00230C), Some(0x77));
    }

    #[test]
    fn sa1_variable_length_data_reads_bwram_and_iram() {
        let mut cartridge = Cartridge::from_bytes(&build_sa1_rom(0x10000, 0x04)).unwrap();

        assert!(cartridge.write(0x002226, 0x80));
        assert!(cartridge.write(0x400000, 0xAB));
        assert!(cartridge.write(0x003000, 0xCD));

        write_u24(&mut cartridge, 0x002259, 0x400000);
        assert_eq!(cartridge.read(0x00230C), Some(0xAB));

        write_u24(&mut cartridge, 0x002259, 0x003000);
        assert_eq!(cartridge.read(0x00230C), Some(0xCD));
    }

    #[test]
    fn sa1_normal_dma_copies_rom_to_iram() {
        let mut rom = build_sa1_rom(0x400000, 0x03);
        rom[0x000000] = 0xA1;
        rom[0x000001] = 0xB2;
        rom[0x000002] = 0xC3;
        let mut cartridge = Cartridge::from_bytes(&rom).unwrap();

        write_u24(&mut cartridge, 0x002232, 0xC00000);
        assert!(cartridge.write(0x002235, 0x00));
        write_word(&mut cartridge, 0x002238, 3);
        assert!(cartridge.write(0x002230, 0x80));
        assert!(cartridge.write(0x002236, 0x03));

        assert_eq!(cartridge.read(0x003300), Some(0xA1));
        assert_eq!(cartridge.read(0x003301), Some(0xB2));
        assert_eq!(cartridge.read(0x003302), Some(0xC3));
    }

    #[test]
    fn sa1_normal_dma_copies_iram_to_bwram() {
        let mut cartridge = Cartridge::from_bytes(&build_sa1_rom(0x10000, 0x04)).unwrap();

        assert!(cartridge.write(0x003010, 0x44));
        assert!(cartridge.write(0x003011, 0x55));
        write_u24(&mut cartridge, 0x002232, 0x000010);
        assert!(cartridge.write(0x002235, 0x00));
        assert!(cartridge.write(0x002236, 0x00));
        write_word(&mut cartridge, 0x002238, 2);
        assert!(cartridge.write(0x002230, 0x86));
        assert!(cartridge.write(0x002237, 0x00));

        assert_eq!(cartridge.read(0x400000), Some(0x44));
        assert_eq!(cartridge.read(0x400001), Some(0x55));
    }

    #[test]
    fn sa1_character_conversion_type2_writes_planar_rows_to_iram() {
        let mut cartridge = Cartridge::from_bytes(&build_sa1_rom(0x10000, 0x03)).unwrap();

        assert!(cartridge.write(0x002235, 0x00));
        assert!(cartridge.write(0x002236, 0x03));
        assert!(cartridge.write(0x002230, 0xA0));
        assert!(cartridge.write(0x002231, 0x02));

        for (index, value) in [0x01, 0x02, 0x01, 0x02, 0x01, 0x02, 0x01, 0x02]
            .into_iter()
            .enumerate()
        {
            assert!(cartridge.write(0x002240 + index as u32, value));
        }
        assert_eq!(cartridge.read(0x003300), Some(0xAA));
        assert_eq!(cartridge.read(0x003301), Some(0x55));

        for (index, value) in [0x03, 0x00, 0x03, 0x00, 0x03, 0x00, 0x03, 0x00]
            .into_iter()
            .enumerate()
        {
            assert!(cartridge.write(0x002248 + index as u32, value));
        }
        assert_eq!(cartridge.read(0x003302), Some(0xAA));
        assert_eq!(cartridge.read(0x003303), Some(0xAA));
    }

    #[test]
    fn sa1_arithmetic_multiplies_signed_operands() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom_with_header(
            "SA1 ARITH MUL",
            0x23,
            0x34,
            None,
            0x0A,
        ))
        .unwrap();

        assert!(cartridge.write(0x002250, 0x00));
        write_word(&mut cartridge, 0x002251, (-1_i16) as u16);
        write_word(&mut cartridge, 0x002253, 1);
        assert_eq!(read_u40(&mut cartridge, 0x002306), 0x0000_FFFF_FFFF);

        write_word(&mut cartridge, 0x002251, 5);
        write_word(&mut cartridge, 0x002253, 3);
        assert_eq!(read_u40(&mut cartridge, 0x002306), 15);

        write_word(&mut cartridge, 0x002253, 4);
        assert_eq!(read_u40(&mut cartridge, 0x002306), 20);

        assert!(cartridge.write(0x002254, 0));
        assert_eq!(read_u40(&mut cartridge, 0x002306), 0);
    }

    #[test]
    fn sa1_arithmetic_divides_signed_by_unsigned() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom_with_header(
            "SA1 ARITH DIV",
            0x23,
            0x34,
            None,
            0x0A,
        ))
        .unwrap();

        assert!(cartridge.write(0x002250, 0x01));
        write_word(&mut cartridge, 0x002251, 10);
        write_word(&mut cartridge, 0x002253, 3);
        assert_eq!(read_word(&mut cartridge, 0x002306), 3);
        assert_eq!(read_word(&mut cartridge, 0x002308), 1);

        write_word(&mut cartridge, 0x002251, (-7_i16) as u16);
        write_word(&mut cartridge, 0x002253, 3);
        assert_eq!(read_word(&mut cartridge, 0x002306) as i16, -3);
        assert_eq!(read_word(&mut cartridge, 0x002308), 2);

        assert!(cartridge.write(0x002254, 0));
        assert_eq!(read_u40(&mut cartridge, 0x002306), 0);

        write_word(&mut cartridge, 0x002251, 99);
        write_word(&mut cartridge, 0x002253, 0);
        assert_eq!(read_u40(&mut cartridge, 0x002306), 0);
    }

    #[test]
    fn sa1_arithmetic_accumulates_signed_products() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom_with_header(
            "SA1 ARITH SUM",
            0x23,
            0x34,
            None,
            0x0A,
        ))
        .unwrap();

        assert!(cartridge.write(0x002250, 0x02));
        write_word(&mut cartridge, 0x002251, 2);
        write_word(&mut cartridge, 0x002253, 3);
        write_word(&mut cartridge, 0x002251, 4);
        write_word(&mut cartridge, 0x002253, 5);
        assert_eq!(read_u40(&mut cartridge, 0x002306), 26);
        assert_eq!(cartridge.read(0x00230B), Some(0x00));

        assert!(cartridge.write(0x002250, 0x00));
        assert_eq!(read_u40(&mut cartridge, 0x002306), 26);
        assert!(cartridge.write(0x002250, 0x02));
        assert_eq!(read_u40(&mut cartridge, 0x002306), 0);

        write_word(&mut cartridge, 0x002251, (-1_i16) as u16);
        write_word(&mut cartridge, 0x002253, 1);
        assert_eq!(read_u40(&mut cartridge, 0x002306), 0x00FF_FFFF_FFFF);
        assert_eq!(cartridge.read(0x00230B), Some(0x80));
    }

    #[test]
    fn super_fx_register_window_is_accessible() {
        let mut cartridge =
            Cartridge::from_bytes(&build_lorom_with_header("GSU MMIO", 0x20, 0x13, None, 0x0A))
                .unwrap();

        assert_eq!(cartridge.read(0x003000), Some(0x00));
        assert!(cartridge.write(0x003000, 0x24));
        assert_eq!(cartridge.read(0x003000), Some(0x24));
        assert_eq!(cartridge.read(0x803000), Some(0x24));
        assert_eq!(cartridge.read(0x00303B), Some(0x04));
        assert_eq!(cartridge.read(0x008000), Some(0xEA));
    }

    const GSU_PIXEL_TEST_PROGRAM: [u8; 28] = [
        0x02, 0xA0, 0x05, 0x4E, 0xA4, 0x00, 0xA5, 0x08, 0x22, 0xB4, 0xA1, 0x00, 0xAC, 0x08, 0x2D,
        0xBF, 0x4C, 0x3C, 0x01, 0xD4, 0xB4, 0x3F, 0x65, 0x08, 0xEF, 0x01, 0x00, 0x01,
    ];
    const GSU_PIXEL_TEST_TILE_4BPP: [u8; 32] = [
        0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF,
        0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00,
        0xFF, 0x00,
    ];
    const GSU_DEMO_PROGRAM: [u8; 103] = [
        0x02, 0xF1, 0x00, 0x0C, 0xF8, 0x02, 0x00, 0xF0, 0xAA, 0x66, 0x31, 0x21, 0x58, 0xF0, 0x55,
        0xCC, 0x31, 0x21, 0x58, 0xF0, 0xAA, 0x99, 0x31, 0x21, 0x58, 0xF0, 0x55, 0x33, 0x31, 0x21,
        0x58, 0xF0, 0xAA, 0x66, 0x31, 0x21, 0x58, 0xF0, 0x55, 0xCC, 0x31, 0x21, 0x58, 0xF0, 0xAA,
        0x99, 0x31, 0x21, 0x58, 0xF0, 0x55, 0x33, 0x31, 0x21, 0x58, 0xF0, 0x1E, 0x01, 0x31, 0x21,
        0x58, 0xF0, 0x3C, 0x03, 0x31, 0x21, 0x58, 0xF0, 0x78, 0x07, 0x31, 0x21, 0x58, 0xF0, 0xF0,
        0x0F, 0x31, 0x21, 0x58, 0xF0, 0xE1, 0x1F, 0x31, 0x21, 0x58, 0xF0, 0xC3, 0x3F, 0x31, 0x21,
        0x58, 0xF0, 0x87, 0x7F, 0x31, 0x21, 0x58, 0xF0, 0x0F, 0xFF, 0x31, 0x00, 0x01,
    ];
    const GSU_DEMO_TILE_4BPP: [u8; 32] = [
        0xAA, 0x66, 0x55, 0xCC, 0xAA, 0x99, 0x55, 0x33, 0xAA, 0x66, 0x55, 0xCC, 0xAA, 0x99, 0x55,
        0x33, 0x1E, 0x01, 0x3C, 0x03, 0x78, 0x07, 0xF0, 0x0F, 0xE1, 0x1F, 0xC3, 0x3F, 0x87, 0x7F,
        0x0F, 0xFF,
    ];
    const GSU_RAM_LOAD_STORE_PROGRAM: [u8; 30] = [
        0xF1, 0x00, 0x01, 0xF0, 0xEF, 0xBE, 0x31, 0x3D, 0x41, 0xF2, 0x10, 0x01, 0xB0, 0x3D, 0x32,
        0x41, 0xF3, 0x20, 0x01, 0xB0, 0x33, 0xFD, 0x30, 0x01, 0xB0, 0x3D, 0x3D, 0x00, 0x01, 0x01,
    ];
    const GSU_ALU_BRANCH_PROGRAM: &[u8] = &[
        0xA0, 0x0F, 0x13, 0xB0, 0x3E, 0x51, 0xB3, 0x03, 0xF1, 0x00, 0x02, 0xB0, 0x31, 0xB3, 0x3E,
        0x77, 0x09, 0x08, 0xF0, 0xAD, 0xDE, 0xF1, 0x10, 0x02, 0xB0, 0x31, 0xF4, 0x34, 0x12, 0xB4,
        0xC0, 0xF1, 0x04, 0x02, 0xB0, 0x31, 0xB4, 0x4D, 0xF1, 0x06, 0x02, 0xB0, 0x31, 0xF0, 0x0F,
        0x00, 0xF5, 0xF0, 0x00, 0xB0, 0xC5, 0xF1, 0x08, 0x02, 0xB0, 0x31, 0xA0, 0x06, 0xA5, 0x07,
        0xB0, 0x85, 0xF1, 0x0A, 0x02, 0xB0, 0x31, 0xF0, 0xF0, 0x00, 0xF5, 0x3C, 0x00, 0xB0, 0x75,
        0xF1, 0x0C, 0x02, 0xB0, 0x31, 0xFD, 0xDC, 0x00, 0x9D, 0xF0, 0xEF, 0xBE, 0xF1, 0x12, 0x02,
        0xB0, 0x31, 0x00, 0x01, 0x01,
    ];
    const GSU_ALU_VARIANTS_PROGRAM: &[u8] = &[
        0xF0, 0x10, 0x00, 0xF2, 0x03, 0x00, 0xB0, 0x62, 0xF1, 0x00, 0x02, 0x31, 0xF0, 0x10, 0x00,
        0xB0, 0x3E, 0x65, 0xF1, 0x02, 0x02, 0x31, 0xF0, 0xF0, 0x00, 0xB0, 0x3E, 0xCF, 0xF1, 0x04,
        0x02, 0x31, 0xF0, 0xFF, 0x00, 0xB0, 0x3F, 0xCF, 0xF1, 0x06, 0x02, 0x31, 0xF0, 0xF0, 0x00,
        0xF2, 0x0F, 0x0F, 0xB0, 0x3D, 0xC2, 0xF1, 0x08, 0x02, 0x31, 0xF0, 0x07, 0x00, 0xB0, 0x3E,
        0x86, 0xF1, 0x0A, 0x02, 0x31, 0xF4, 0x01, 0x00, 0xE4, 0xB4, 0x3E, 0x50, 0xF1, 0x0C, 0x02,
        0x31, 0xF0, 0xF0, 0x00, 0xB0, 0x4F, 0xF1, 0x0E, 0x02, 0x31, 0xF0, 0xAB, 0x12, 0xB0, 0x9E,
        0xF1, 0x10, 0x02, 0x31, 0xF0, 0x80, 0x00, 0xB0, 0x95, 0xF1, 0x12, 0x02, 0x31, 0xF0, 0x80,
        0xFF, 0xB0, 0x96, 0xF1, 0x14, 0x02, 0x31, 0x00, 0x01,
    ];
    const GSU_SPRITE_SCALER_PROGRAM: &[u8] = include_bytes!(
        "../../../roms/snes-coprocessor-tests/hirom-gsu-test/build/sprite_scaler.bin"
    );
    const GSU_BITMASK_LUT: [u8; 8] = [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01];
    const CX4_PACKED_NIBBLE_PATTERN: [u8; 4] = [0x10, 0x32, 0x54, 0x76];
    const CX4_PATTERN_BITPLANES: [u8; 32] = [
        0x55, 0x33, 0x55, 0x33, 0x55, 0x33, 0x55, 0x33, 0x55, 0x33, 0x55, 0x33, 0x55, 0x33, 0x55,
        0x33, 0x0F, 0x00, 0x0F, 0x00, 0x0F, 0x00, 0x0F, 0x00, 0x0F, 0x00, 0x0F, 0x00, 0x0F, 0x00,
        0x0F, 0x00,
    ];

    #[test]
    fn super_fx_game_ram_maps_full_direct_banks_and_starts_programs() {
        let mut cartridge = Cartridge::from_bytes(&build_hirom_with_header(
            "HIROM GSU MMIO",
            0x31,
            0x15,
            None,
            0x0C,
        ))
        .unwrap();

        assert_eq!(
            cartridge.header().enhancement_chip(),
            EnhancementChip::SuperFxGsu2
        );
        assert_eq!(cartridge.save_ram().len(), 8 * 1024);
        assert_eq!(cartridge.read(0x00303B), Some(0x04));
        assert!(cartridge.write(0x700000, 0x34));
        assert!(cartridge.write(0x700001, 0x12));
        assert_eq!(cartridge.read(0x700000), Some(0x34));
        assert_eq!(cartridge.read(0x700001), Some(0x12));
        assert!(cartridge.write(0x702000, 0xA5));
        assert_eq!(cartridge.read(0x700000), Some(0xA5));

        for (offset, value) in GSU_PIXEL_TEST_PROGRAM.iter().copied().enumerate() {
            assert!(cartridge.write(0x700200 + offset as u32, value));
        }
        assert!(cartridge.write(0x003038, 0x03));
        assert!(cartridge.write(0x003030, 0x20));
        assert!(cartridge.write(0x00301E, 0x00));
        assert!(cartridge.write(0x00301F, 0x02));
        assert_eq!(cartridge.read(0x003030).unwrap() & 0x20, 0x00);
        for (offset, expected) in GSU_PIXEL_TEST_TILE_4BPP.iter().copied().enumerate() {
            assert_eq!(cartridge.read(0x700C00 + offset as u32), Some(expected));
        }

        for (offset, value) in GSU_DEMO_PROGRAM.iter().copied().enumerate() {
            assert!(cartridge.write(0x700100 + offset as u32, value));
        }
        assert!(cartridge.write(0x003030, 0x20));
        assert!(cartridge.write(0x00301E, 0x00));
        assert!(cartridge.write(0x00301F, 0x01));
        assert_eq!(cartridge.read(0x003030).unwrap() & 0x20, 0x00);
        for (offset, expected) in GSU_DEMO_TILE_4BPP.iter().copied().enumerate() {
            assert_eq!(cartridge.read(0x700C00 + offset as u32), Some(expected));
        }
        assert_eq!(cartridge.read(0xC08000), Some(0xEA));
    }

    #[test]
    fn super_fx_alt_loads_and_byte_stores_game_ram() {
        let mut cartridge = Cartridge::from_bytes(&build_hirom_with_header(
            "HIROM GSU RAM OPS",
            0x31,
            0x15,
            None,
            0x0C,
        ))
        .unwrap();

        for (offset, value) in GSU_RAM_LOAD_STORE_PROGRAM.iter().copied().enumerate() {
            assert!(cartridge.write(0x700080 + offset as u32, value));
        }
        assert!(cartridge.write(0x003030, 0x20));
        assert!(cartridge.write(0x00301E, 0x80));
        assert!(cartridge.write(0x00301F, 0x00));

        assert_eq!(cartridge.read(0x003030).unwrap() & 0x20, 0x00);
        assert_eq!(cartridge.read(0x700100), Some(0xEF));
        assert_eq!(cartridge.read(0x700101), Some(0xBE));
        assert_eq!(cartridge.read(0x700110), Some(0xEF));
        assert_eq!(cartridge.read(0x700111), Some(0x00));
        assert_eq!(cartridge.read(0x700120), Some(0xEF));
        assert_eq!(cartridge.read(0x700121), Some(0xBE));
        assert_eq!(cartridge.read(0x700130), Some(0xEF));
        assert_eq!(cartridge.read(0x700131), Some(0x00));
    }

    #[test]
    fn super_fx_alu_and_branch_ops_update_game_ram() {
        let mut cartridge = Cartridge::from_bytes(&build_hirom_with_header(
            "HIROM GSU ALU OPS",
            0x31,
            0x15,
            None,
            0x0C,
        ))
        .unwrap();

        for (offset, value) in GSU_ALU_BRANCH_PROGRAM.iter().copied().enumerate() {
            assert!(cartridge.write(0x700080 + offset as u32, value));
        }
        assert!(cartridge.write(0x003030, 0x20));
        assert!(cartridge.write(0x00301E, 0x80));
        assert!(cartridge.write(0x00301F, 0x00));

        assert_eq!(cartridge.read(0x003030).unwrap() & 0x20, 0x00);
        assert_eq!(cartridge.read(0x700200), Some(0x08));
        assert_eq!(cartridge.read(0x700201), Some(0x00));
        assert_eq!(cartridge.read(0x700210), Some(0x00));
        assert_eq!(cartridge.read(0x700211), Some(0x00));
        assert_eq!(cartridge.read(0x700204), Some(0x12));
        assert_eq!(cartridge.read(0x700205), Some(0x00));
        assert_eq!(cartridge.read(0x700206), Some(0x12));
        assert_eq!(cartridge.read(0x700207), Some(0x34));
        assert_eq!(cartridge.read(0x700208), Some(0xFF));
        assert_eq!(cartridge.read(0x700209), Some(0x00));
        assert_eq!(cartridge.read(0x70020A), Some(0x2A));
        assert_eq!(cartridge.read(0x70020B), Some(0x00));
        assert_eq!(cartridge.read(0x70020C), Some(0x30));
        assert_eq!(cartridge.read(0x70020D), Some(0x00));
        assert_eq!(cartridge.read(0x700212), Some(0x00));
        assert_eq!(cartridge.read(0x700213), Some(0x00));
    }

    #[test]
    fn super_fx_alu_variants_update_game_ram() {
        let mut cartridge = Cartridge::from_bytes(&build_hirom_with_header(
            "HIROM GSU ALU VARIANTS",
            0x31,
            0x15,
            None,
            0x0C,
        ))
        .unwrap();

        for (offset, value) in GSU_ALU_VARIANTS_PROGRAM.iter().copied().enumerate() {
            assert!(cartridge.write(0x700080 + offset as u32, value));
        }
        assert!(cartridge.write(0x003030, 0x20));
        assert!(cartridge.write(0x00301E, 0x80));
        assert!(cartridge.write(0x00301F, 0x00));

        assert_eq!(cartridge.read(0x003030).unwrap() & 0x20, 0x00);
        for (offset, expected) in [
            0x0D, 0x00, 0x0B, 0x00, 0xFF, 0x00, 0xF0, 0x00, 0xFF, 0x0F, 0x2A, 0x00, 0x00, 0x00,
            0x0F, 0xFF, 0xAB, 0x00, 0x80, 0xFF, 0xC0, 0xFF,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(cartridge.read(0x700200 + offset as u32), Some(expected));
        }
    }

    #[test]
    fn super_fx_runs_sprite_scaler_fixture() {
        let mut cartridge = Cartridge::from_bytes(&build_hirom_with_header(
            "HIROM GSU SCALER",
            0x31,
            0x15,
            None,
            0x0C,
        ))
        .unwrap();

        for (offset, value) in [8, 8, 8, 8, 0x00, 0x04, 1, 1, 0x00, 0x01, 0x00, 0x01]
            .into_iter()
            .enumerate()
        {
            assert!(cartridge.write(0x700000 + offset as u32, value));
        }
        for (offset, value) in GSU_BITMASK_LUT.iter().copied().enumerate() {
            assert!(cartridge.write(0x700060 + offset as u32, value));
        }
        for (offset, value) in GSU_DEMO_TILE_4BPP.iter().copied().enumerate() {
            assert!(cartridge.write(0x700400 + offset as u32, value));
        }
        for (offset, value) in GSU_SPRITE_SCALER_PROGRAM.iter().copied().enumerate() {
            assert!(cartridge.write(0x700100 + offset as u32, value));
        }
        assert!(cartridge.write(0x003030, 0x20));
        assert!(cartridge.write(0x00301E, 0x00));
        assert!(cartridge.write(0x00301F, 0x01));

        assert_eq!(cartridge.read(0x003030).unwrap() & 0x20, 0x00);
        for (offset, expected) in GSU_DEMO_TILE_4BPP.iter().copied().enumerate() {
            assert_eq!(cartridge.read(0x700C00 + offset as u32), Some(expected));
        }
    }

    #[test]
    fn cx4_register_window_is_accessible() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom_with_header(
            "CX4 MMIO",
            0x20,
            0xF3,
            Some(0x10),
            0x0A,
        ))
        .unwrap();

        assert_eq!(cartridge.read(0x007F40), Some(0x00));
        assert!(cartridge.write(0x007F40, 0x66));
        assert_eq!(cartridge.read(0x007F40), Some(0x66));
        assert_eq!(cartridge.read(0x807F40), Some(0x66));
        assert!(cartridge.write(0x006000, 0x42));
        assert_eq!(cartridge.read(0x006000), Some(0x42));
        assert_eq!(cartridge.read(0x806000), Some(0x42));
        assert_eq!(cartridge.read(0x007F5E), Some(0x00));
        assert_eq!(cartridge.read(0x008000), Some(0xEA));
    }

    #[test]
    fn cx4_executes_core_math_commands() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom_with_header(
            "CX4 COMMANDS",
            0x20,
            0xF3,
            Some(0x10),
            0x0A,
        ))
        .unwrap();

        write_u24(&mut cartridge, 0x007F80, 0x000123);
        write_u24(&mut cartridge, 0x007F83, 0x000004);
        assert!(cartridge.write(0x007F4F, 0x25));
        assert_eq!(read_u24(&mut cartridge, 0x007F80), 0x00048C);

        write_word(&mut cartridge, 0x007F80, 3);
        write_word(&mut cartridge, 0x007F83, 4);
        assert!(cartridge.write(0x007F4F, 0x15));
        assert_eq!(read_word(&mut cartridge, 0x007F80), 5);

        for offset in 0..0x800 {
            assert!(cartridge.write(0x006000 + offset, 1));
        }
        assert!(cartridge.write(0x007F4F, 0x40));
        assert_eq!(read_word(&mut cartridge, 0x007F80), 0x0800);

        assert!(cartridge.write(0x007F4D, 0x0E));
        assert!(cartridge.write(0x007F4F, 0x20));
        assert_eq!(cartridge.read(0x007F80), Some(0x08));

        assert!(cartridge.write(0x007F4F, 0x89));
        assert_eq!(read_u24(&mut cartridge, 0x007F80), 0x054336);
    }

    #[test]
    fn cx4_executes_geometry_commands() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom_with_header(
            "CX4 GEOMETRY",
            0x20,
            0xF3,
            Some(0x10),
            0x0A,
        ))
        .unwrap();

        write_word(&mut cartridge, 0x007F81, 10);
        write_word(&mut cartridge, 0x007F84, (-20_i16) as u16);
        write_word(&mut cartridge, 0x007F87, 7);
        assert!(cartridge.write(0x007F89, 0));
        assert!(cartridge.write(0x007F8A, 0));
        assert!(cartridge.write(0x007F8B, 0));
        write_word(&mut cartridge, 0x007F90, 0x0100);
        assert!(cartridge.write(0x007F4F, 0x2D));
        assert_eq!(read_word(&mut cartridge, 0x007F80), 10);
        assert_eq!(read_word(&mut cartridge, 0x007F83), (-20_i16) as u16);

        write_word(&mut cartridge, 0x007F80, 5);
        write_word(&mut cartridge, 0x007F83, 10);
        write_word(&mut cartridge, 0x007F86, 20);
        write_word(&mut cartridge, 0x007F89, 8);
        write_word(&mut cartridge, 0x007F8C, 0);
        write_word(&mut cartridge, 0x007F8F, 0);
        write_word(&mut cartridge, 0x007F93, 30);
        assert!(cartridge.write(0x007F4F, 0x22));
        assert_eq!(cartridge.read(0x006800), Some(15));
        assert_eq!(cartridge.read(0x006900), Some(45));
        assert_eq!(cartridge.read(0x0068E0), Some(15));
        assert_eq!(cartridge.read(0x0069E0), Some(45));

        write_word(&mut cartridge, 0x007F83, 7);
        write_word(&mut cartridge, 0x007F89, 8);
        assert!(cartridge.write(0x007F4F, 0x22));
        assert_eq!(cartridge.read(0x006800), Some(1));
        assert_eq!(cartridge.read(0x006900), Some(0));
        assert_eq!(cartridge.read(0x006801), Some(15));
        assert_eq!(cartridge.read(0x006901), Some(45));
    }

    #[test]
    fn cx4_loads_lorom_data_into_internal_ram() {
        let mut rom = build_lorom_with_header("CX4 LOAD", 0x20, 0xF3, Some(0x10), 0x0A);
        rom[0x8123] = 0xAA;
        rom[0x8124] = 0xBB;
        rom[0x8125] = 0xCC;
        let mut cartridge = Cartridge::from_bytes(&rom).unwrap();

        write_u24(&mut cartridge, 0x007F40, 0x018123);
        write_word(&mut cartridge, 0x007F43, 3);
        write_word(&mut cartridge, 0x007F45, 0x6008);
        assert!(cartridge.write(0x007F47, 0x00));

        assert_eq!(cartridge.read(0x006008), Some(0xAA));
        assert_eq!(cartridge.read(0x006009), Some(0xBB));
        assert_eq!(cartridge.read(0x00600A), Some(0xCC));
    }

    #[test]
    fn cx4_executes_immediate_register_commands() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom_with_header(
            "CX4 IMMEDIATE",
            0x20,
            0xF3,
            Some(0x10),
            0x0A,
        ))
        .unwrap();

        write_u24(&mut cartridge, 0x007F80, 0x000077);
        assert!(cartridge.write(0x007F4F, 0x5C));
        assert_eq!(read_u24(&mut cartridge, 0x007F80), 0x000030);
        assert_eq!(cartridge.read(0x006000), Some(0x00));
        assert_eq!(cartridge.read(0x006003), Some(0xFF));
        assert_eq!(cartridge.read(0x00602F), Some(0x00));

        write_u24(&mut cartridge, 0x007F80, 0x000020);
        assert!(cartridge.write(0x007F4F, 0x66));
        assert_eq!(read_u24(&mut cartridge, 0x007F80), 0x000044);
        assert_eq!(cartridge.read(0x006020), Some(0xFF));
        assert_eq!(cartridge.read(0x006027), Some(0x00));
        assert_eq!(cartridge.read(0x006043), Some(0x00));

        write_u24(&mut cartridge, 0x007F80, 0x000BFE);
        assert!(cartridge.write(0x007F4F, 0x7C));
        assert_eq!(read_u24(&mut cartridge, 0x007F80), 0x000C01);
        assert_eq!(cartridge.read(0x006BFE), Some(0xFF));
        assert_eq!(cartridge.read(0x006BFF), Some(0xFE));
    }

    #[test]
    fn cx4_disintegrates_packed_pixels_to_bitplanes() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom_with_header(
            "CX4 DISINTEGRATE",
            0x20,
            0xF3,
            Some(0x10),
            0x0A,
        ))
        .unwrap();

        write_word(&mut cartridge, 0x007F80, 4);
        write_word(&mut cartridge, 0x007F83, 4);
        write_word(&mut cartridge, 0x007F86, 0x0100);
        assert!(cartridge.write(0x007F89, 8));
        assert!(cartridge.write(0x007F8C, 8));
        write_word(&mut cartridge, 0x007F8F, 0x0100);

        write_cx4_packed_pattern(&mut cartridge);

        assert!(cartridge.write(0x007F4D, 0x0B));
        assert!(cartridge.write(0x007F4F, 0x00));

        assert_cx4_pattern_bitplanes(&mut cartridge);
    }

    #[test]
    fn cx4_scale_rotates_packed_pixels_to_bitplanes() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom_with_header(
            "CX4 SCALE ROTATE",
            0x20,
            0xF3,
            Some(0x10),
            0x0A,
        ))
        .unwrap();

        write_word(&mut cartridge, 0x007F80, 0);
        write_word(&mut cartridge, 0x007F83, 4);
        write_word(&mut cartridge, 0x007F86, 4);
        assert!(cartridge.write(0x007F89, 8));
        assert!(cartridge.write(0x007F8C, 8));
        write_word(&mut cartridge, 0x007F8F, 0x1000);
        write_word(&mut cartridge, 0x007F92, 0x1000);
        write_cx4_packed_pattern(&mut cartridge);

        assert!(cartridge.write(0x007F4D, 0x03));
        assert!(cartridge.write(0x007F4F, 0x00));
        assert_cx4_pattern_bitplanes(&mut cartridge);

        assert!(cartridge.write(0x007F4D, 0x07));
        assert!(cartridge.write(0x007F4F, 0x00));
        assert_cx4_pattern_bitplanes(&mut cartridge);
    }

    #[test]
    fn cx4_applies_bitplane_wave() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom_with_header(
            "CX4 BITPLANE WAVE",
            0x20,
            0xF3,
            Some(0x10),
            0x0A,
        ))
        .unwrap();

        for row in 0..8u32 {
            write_word(&mut cartridge, 0x006A00 + row * 2, 0xFFFF);
            write_word(&mut cartridge, 0x006A10 + row * 2, 0xFFFF);
        }
        for offset in 0..0x80 {
            assert!(cartridge.write(0x006B00 + offset, 0xF0));
        }

        assert!(cartridge.write(0x007F83, 0));
        assert!(cartridge.write(0x007F4D, 0x0C));
        assert!(cartridge.write(0x007F4F, 0x00));

        assert_eq!(read_word(&mut cartridge, 0x006000), 0xFFFF);
        assert_eq!(read_word(&mut cartridge, 0x00600E), 0xFFFF);
        assert_eq!(read_word(&mut cartridge, 0x006200), 0xFF00);
        assert_eq!(read_word(&mut cartridge, 0x00680E), 0xFF00);
        assert_eq!(read_word(&mut cartridge, 0x006010), 0xFFFF);
        assert_eq!(read_word(&mut cartridge, 0x006210), 0xFF00);
    }

    #[test]
    fn cx4_transforms_line_vertices() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom_with_header(
            "CX4 LINES",
            0x20,
            0xF3,
            Some(0x10),
            0x0A,
        ))
        .unwrap();

        write_word(&mut cartridge, 0x007F80, 1);
        assert!(cartridge.write(0x007F83, 0));
        assert!(cartridge.write(0x007F86, 0));
        assert!(cartridge.write(0x007F89, 0));
        assert!(cartridge.write(0x007F8C, 0x90));
        write_word(&mut cartridge, 0x006001, 10);
        write_word(&mut cartridge, 0x006005, 20);
        write_word(&mut cartridge, 0x006009, 0x0095);

        assert!(cartridge.write(0x007F4D, 0x05));
        assert!(cartridge.write(0x007F4F, 0x00));

        assert_eq!(read_word(&mut cartridge, 0x006001), 0x008A);
        assert_eq!(read_word(&mut cartridge, 0x006005), 0x0064);
        assert_eq!(read_word(&mut cartridge, 0x006600), 23);
        assert_eq!(read_word(&mut cartridge, 0x006602), 0x60);
        assert_eq!(read_word(&mut cartridge, 0x006605), 0x40);
        assert_eq!(read_word(&mut cartridge, 0x006608), 23);
        assert_eq!(read_word(&mut cartridge, 0x00660A), 0x60);
        assert_eq!(read_word(&mut cartridge, 0x00660D), 0x40);
    }

    #[test]
    fn cx4_draws_wireframe_from_rom() {
        let mut rom = build_lorom_with_header("CX4 WIREFRAME", 0x20, 0xF3, Some(0x10), 0x0A);
        rom[0x0100..0x0105].copy_from_slice(&[0x81, 0x10, 0x81, 0x16, 0x03]);
        rom[0x0110..0x0116].copy_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        rom[0x0116..0x011C].copy_from_slice(&[0x00, 0x08, 0x00, 0x00, 0x00, 0x00]);
        let mut cartridge = Cartridge::from_bytes(&rom).unwrap();

        write_u24(&mut cartridge, 0x007F80, 0x008100);
        assert!(cartridge.write(0x006295, 1));
        assert!(cartridge.write(0x007F90, 0x80));

        assert!(cartridge.write(0x007F4F, 0x01));
        assert_eq!(cartridge.read(0x0067E0), Some(0xF8));
        assert_eq!(cartridge.read(0x0067E1), Some(0xF8));

        assert!(cartridge.write(0x0067E0, 0));
        assert!(cartridge.write(0x0067E1, 0));
        assert!(cartridge.write(0x007F4D, 0x08));
        assert!(cartridge.write(0x007F4F, 0x00));
        assert_eq!(cartridge.read(0x0067E0), Some(0xF8));
        assert_eq!(cartridge.read(0x0067E1), Some(0xF8));
    }

    #[test]
    fn cx4_builds_oam_from_sprite_records() {
        let mut rom = build_lorom_with_header("CX4 OAM", 0x20, 0xF3, Some(0x10), 0x0A);
        rom[0x0100] = 0;
        let mut cartridge = Cartridge::from_bytes(&rom).unwrap();

        assert!(cartridge.write(0x006620, 1));
        write_word(&mut cartridge, 0x006220, 0x0012);
        write_word(&mut cartridge, 0x006222, 0x0034);
        assert!(cartridge.write(0x006224, 0x20));
        assert!(cartridge.write(0x006225, 0x40));
        assert!(cartridge.write(0x006226, 0x05));
        write_u24(&mut cartridge, 0x006227, 0x008100);

        assert!(cartridge.write(0x007F4D, 0x00));
        assert!(cartridge.write(0x007F4F, 0x00));

        assert_eq!(cartridge.read(0x006000), Some(0x12));
        assert_eq!(cartridge.read(0x006001), Some(0x34));
        assert_eq!(cartridge.read(0x006002), Some(0x40));
        assert_eq!(cartridge.read(0x006003), Some(0x25));
        assert_eq!(cartridge.read(0x006200), Some(0x02));
        assert_eq!(cartridge.read(0x0061FD), Some(0xE0));
    }

    fn write_cx4_packed_pattern(cartridge: &mut Cartridge) {
        for row in 0..8u32 {
            for (column_pair, byte) in CX4_PACKED_NIBBLE_PATTERN.into_iter().enumerate() {
                assert!(cartridge.write(0x006600 + row * 4 + column_pair as u32, byte));
            }
        }
    }

    fn assert_cx4_pattern_bitplanes(cartridge: &mut Cartridge) {
        for (offset, expected) in CX4_PATTERN_BITPLANES.into_iter().enumerate() {
            assert_eq!(cartridge.read(0x006000 + offset as u32), Some(expected));
        }
    }

    fn write_word(cartridge: &mut Cartridge, address: u32, word: u16) {
        let [low, high] = word.to_le_bytes();
        assert!(cartridge.write(address, low));
        assert!(cartridge.write(address + 1, high));
    }

    fn read_word(cartridge: &mut Cartridge, address: u32) -> u16 {
        u16::from_le_bytes([
            cartridge.read(address).unwrap(),
            cartridge.read(address + 1).unwrap(),
        ])
    }

    fn write_u24(cartridge: &mut Cartridge, address: u32, value: u32) {
        assert!(cartridge.write(address, value as u8));
        assert!(cartridge.write(address + 1, (value >> 8) as u8));
        assert!(cartridge.write(address + 2, (value >> 16) as u8));
    }

    fn read_u24(cartridge: &mut Cartridge, address: u32) -> u32 {
        u32::from(cartridge.read(address).unwrap())
            | (u32::from(cartridge.read(address + 1).unwrap()) << 8)
            | (u32::from(cartridge.read(address + 2).unwrap()) << 16)
    }

    fn read_u40(cartridge: &mut Cartridge, address: u32) -> u64 {
        (0..5).fold(0, |value, byte| {
            value | (u64::from(cartridge.read(address + byte).unwrap()) << (byte * 8))
        })
    }

    #[test]
    fn dsp1_lorom_register_window_reports_ready_status() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom_with_header(
            "DSP1 MMIO",
            0x20,
            0x03,
            None,
            0x0A,
        ))
        .unwrap();

        assert_eq!(cartridge.read(0x208000), Some(0x00));
        assert_eq!(cartridge.read(0x208001), Some(0x84));
        assert!(cartridge.write(0x208001, 0x00));
        assert_eq!(cartridge.read(0x208001), Some(0x84));
        assert!(cartridge.write(0x208000, 0x99));
        assert_eq!(cartridge.read(0x208000), Some(0x99));
        assert_eq!(cartridge.read(0xA08001), Some(0x84));
        assert_eq!(cartridge.read(0x008000), Some(0xEA));
    }

    fn write_dsp1_word(cartridge: &mut Cartridge, data_address: u32, word: u16) {
        let [low, high] = word.to_le_bytes();
        assert!(cartridge.write(data_address, low));
        assert!(cartridge.write(data_address, high));
    }

    fn read_dsp1_word(cartridge: &mut Cartridge, data_address: u32) -> u16 {
        let low = cartridge.read_mut(data_address).unwrap();
        let high = cartridge.read_mut(data_address).unwrap();
        u16::from_le_bytes([low, high])
    }

    #[test]
    fn dsp1_lorom_executes_tier1_commands() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom_with_header(
            "DSP1 COMMANDS",
            0x20,
            0x03,
            None,
            0x0A,
        ))
        .unwrap();

        assert!(cartridge.write(0x208000, 0x00));
        assert_eq!(cartridge.read(0x208001), Some(0x80));
        write_dsp1_word(&mut cartridge, 0x208000, 0x4000);
        write_dsp1_word(&mut cartridge, 0x208000, 0x4000);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x2000);
        assert_eq!(cartridge.read(0x208001), Some(0x84));

        assert!(cartridge.write(0x208000, 0x20));
        write_dsp1_word(&mut cartridge, 0x208000, 0x4000);
        write_dsp1_word(&mut cartridge, 0x208000, 0x4000);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x2001);

        assert!(cartridge.write(0x208000, 0x0F));
        write_dsp1_word(&mut cartridge, 0x208000, 0xFFFF);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x0000);

        assert!(cartridge.write(0x208000, 0x2F));
        write_dsp1_word(&mut cartridge, 0x208000, 0xFFFF);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x0100);

        assert!(cartridge.write(0x208000, 0x08));
        write_dsp1_word(&mut cartridge, 0x208000, 3);
        write_dsp1_word(&mut cartridge, 0x208000, 4);
        write_dsp1_word(&mut cartridge, 0x208000, 12);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 169);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0);

        assert!(cartridge.write(0x208000, 0x18));
        write_dsp1_word(&mut cartridge, 0x208000, 3);
        write_dsp1_word(&mut cartridge, 0x208000, 4);
        write_dsp1_word(&mut cartridge, 0x208000, 12);
        write_dsp1_word(&mut cartridge, 0x208000, 13);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0);

        assert!(cartridge.write(0x208000, 0x38));
        write_dsp1_word(&mut cartridge, 0x208000, 3);
        write_dsp1_word(&mut cartridge, 0x208000, 4);
        write_dsp1_word(&mut cartridge, 0x208000, 12);
        write_dsp1_word(&mut cartridge, 0x208000, 13);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 1);

        assert!(cartridge.write(0x208000, 0x10));
        write_dsp1_word(&mut cartridge, 0x208000, 0x4000);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x7FFF);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 1);

        assert!(cartridge.write(0x208000, 0x30));
        write_dsp1_word(&mut cartridge, 0x208000, 0x2000);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x7FFF);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 2);

        assert!(cartridge.write(0x208000, 0x10));
        write_dsp1_word(&mut cartridge, 0x208000, (-0x4000_i16) as u16);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        assert_eq!(
            read_dsp1_word(&mut cartridge, 0x208000),
            (-0x4000_i16) as u16
        );
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 2);

        assert!(cartridge.write(0x208000, 0x10));
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x7FFF);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x002F);
    }

    #[test]
    fn dsp1_lorom_executes_geometry_commands() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom_with_header(
            "DSP1 GEOMETRY",
            0x20,
            0x03,
            None,
            0x0A,
        ))
        .unwrap();

        assert!(cartridge.write(0x208000, 0x04));
        write_dsp1_word(&mut cartridge, 0x208000, 0x4000);
        write_dsp1_word(&mut cartridge, 0x208000, 0x4000);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x4000);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x0000);

        assert!(cartridge.write(0x208000, 0x0C));
        write_dsp1_word(&mut cartridge, 0x208000, 0x4000);
        write_dsp1_word(&mut cartridge, 0x208000, 10);
        write_dsp1_word(&mut cartridge, 0x208000, 20);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 20);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), (-10_i16) as u16);

        assert!(cartridge.write(0x208000, 0x1C));
        write_dsp1_word(&mut cartridge, 0x208000, 0x4000);
        write_dsp1_word(&mut cartridge, 0x208000, 0x0000);
        write_dsp1_word(&mut cartridge, 0x208000, 0x0000);
        write_dsp1_word(&mut cartridge, 0x208000, 10);
        write_dsp1_word(&mut cartridge, 0x208000, 20);
        write_dsp1_word(&mut cartridge, 0x208000, 30);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 20);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), (-10_i16) as u16);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 30);

        assert!(cartridge.write(0x208000, 0x14));
        write_dsp1_word(&mut cartridge, 0x208000, 100);
        write_dsp1_word(&mut cartridge, 0x208000, 200);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        write_dsp1_word(&mut cartridge, 0x208000, 5);
        write_dsp1_word(&mut cartridge, 0x208000, 7);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 100);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 205);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 7);

        assert!(cartridge.write(0x208000, 0x34));
        write_dsp1_word(&mut cartridge, 0x208000, 100);
        write_dsp1_word(&mut cartridge, 0x208000, 200);
        write_dsp1_word(&mut cartridge, 0x208000, 0x4000);
        write_dsp1_word(&mut cartridge, 0x208000, 10);
        write_dsp1_word(&mut cartridge, 0x208000, 20);
        write_dsp1_word(&mut cartridge, 0x208000, 30);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 80);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 210);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x4000 + 30);

        assert!(cartridge.write(0x208000, 0x28));
        write_dsp1_word(&mut cartridge, 0x208000, 3);
        write_dsp1_word(&mut cartridge, 0x208000, 4);
        write_dsp1_word(&mut cartridge, 0x208000, 12);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 13);
    }

    #[test]
    fn dsp1_lorom_executes_projection_commands() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom_with_header(
            "DSP1 PROJECT",
            0x20,
            0x03,
            None,
            0x0A,
        ))
        .unwrap();

        assert!(cartridge.write(0x208000, 0x02));
        write_dsp1_word(&mut cartridge, 0x208000, 100);
        write_dsp1_word(&mut cartridge, 0x208000, 200);
        write_dsp1_word(&mut cartridge, 0x208000, 300);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        write_dsp1_word(&mut cartridge, 0x208000, 0x4000);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x4000);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 100);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 200);

        assert!(cartridge.write(0x208000, 0x0A));
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x0100);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x0100);

        assert!(cartridge.write(0x208000, 0x1A));
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x0100);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x0100);

        assert!(cartridge.write(0x208000, 0x06));
        write_dsp1_word(&mut cartridge, 0x208000, 116);
        write_dsp1_word(&mut cartridge, 0x208000, 232);
        write_dsp1_word(&mut cartridge, 0x208000, 300);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 16);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 32);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x0100);

        assert!(cartridge.write(0x208000, 0x16));
        write_dsp1_word(&mut cartridge, 0x208000, 101);
        write_dsp1_word(&mut cartridge, 0x208000, 200);
        write_dsp1_word(&mut cartridge, 0x208000, (-20000_i16) as u16);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x4000);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x7FFF);

        assert!(cartridge.write(0x208000, 0x3E));
        write_dsp1_word(&mut cartridge, 0x208000, 16);
        write_dsp1_word(&mut cartridge, 0x208000, 32);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 116);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 232);
    }

    #[test]
    fn dsp1_lorom_executes_matrix_commands() {
        let mut cartridge = Cartridge::from_bytes(&build_lorom_with_header(
            "DSP1 MATRICES",
            0x20,
            0x03,
            None,
            0x0A,
        ))
        .unwrap();

        assert!(cartridge.write(0x208000, 0x01));
        write_dsp1_word(&mut cartridge, 0x208000, 0x4000);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        write_dsp1_word(&mut cartridge, 0x208000, 0);

        assert!(cartridge.write(0x208000, 0x0D));
        write_dsp1_word(&mut cartridge, 0x208000, 40);
        write_dsp1_word(&mut cartridge, 0x208000, (-80_i16) as u16);
        write_dsp1_word(&mut cartridge, 0x208000, 120);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 10);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), (-20_i16) as u16);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 30);

        assert!(cartridge.write(0x208000, 0x03));
        write_dsp1_word(&mut cartridge, 0x208000, 40);
        write_dsp1_word(&mut cartridge, 0x208000, (-80_i16) as u16);
        write_dsp1_word(&mut cartridge, 0x208000, 120);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 10);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), (-20_i16) as u16);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 30);

        assert!(cartridge.write(0x208000, 0x0B));
        write_dsp1_word(&mut cartridge, 0x208000, 40);
        write_dsp1_word(&mut cartridge, 0x208000, (-80_i16) as u16);
        write_dsp1_word(&mut cartridge, 0x208000, 120);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 10);

        assert!(cartridge.write(0x208000, 0x11));
        write_dsp1_word(&mut cartridge, 0x208000, 0x4000);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        assert!(cartridge.write(0x208000, 0x1D));
        write_dsp1_word(&mut cartridge, 0x208000, 64);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 16);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0);

        assert!(cartridge.write(0x208000, 0x21));
        write_dsp1_word(&mut cartridge, 0x208000, 0x4000);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        assert!(cartridge.write(0x208000, 0x2D));
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        write_dsp1_word(&mut cartridge, 0x208000, 64);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 16);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0);

        assert!(cartridge.write(0x208000, 0x01));
        write_dsp1_word(&mut cartridge, 0x208000, 0x4000);
        write_dsp1_word(&mut cartridge, 0x208000, 0x2000);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        assert!(cartridge.write(0x208000, 0x09));
        write_dsp1_word(&mut cartridge, 0x208000, 4);
        write_dsp1_word(&mut cartridge, 0x208000, 4);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 1);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0);

        assert!(cartridge.write(0x208000, 0x03));
        write_dsp1_word(&mut cartridge, 0x208000, 4);
        write_dsp1_word(&mut cartridge, 0x208000, 4);
        write_dsp1_word(&mut cartridge, 0x208000, 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 1);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0);
    }

    #[test]
    fn dsp1_lorom_executes_memory_command_aliases() {
        let new_cartridge = || {
            Cartridge::from_bytes(&build_lorom_with_header(
                "DSP1 MEMORY",
                0x20,
                0x03,
                None,
                0x0A,
            ))
            .unwrap()
        };

        let mut cartridge = new_cartridge();
        assert!(cartridge.write(0x208000, 0x07));
        write_dsp1_word(&mut cartridge, 0x208000, 0xFFFF);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x0000);

        let mut cartridge = new_cartridge();
        assert!(cartridge.write(0x208000, 0x27));
        write_dsp1_word(&mut cartridge, 0x208000, 0xFFFF);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x0100);

        let mut cartridge = new_cartridge();
        assert!(cartridge.write(0x208000, 0x17));
        write_dsp1_word(&mut cartridge, 0x208000, 0xFFFF);
        for _ in 0..1023 {
            assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x0000);
            assert_eq!(cartridge.read(0x208001), Some(0x80));
        }
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x0000);
        assert_eq!(cartridge.read(0x208001), Some(0x84));

        let mut cartridge = new_cartridge();
        assert!(cartridge.write(0x208000, 0x37));
        write_dsp1_word(&mut cartridge, 0x208000, 0xFFFF);
        assert_eq!(read_dsp1_word(&mut cartridge, 0x208000), 0x0000);
        assert_eq!(cartridge.read(0x208001), Some(0x80));
    }

    #[test]
    fn dsp1_hirom_register_window_reports_ready_status() {
        let mut cartridge = Cartridge::from_bytes(&build_hirom_with_header(
            "DSP1B MMIO",
            0x21,
            0x05,
            None,
            0x0A,
        ))
        .unwrap();

        assert_eq!(cartridge.read(0x006000), Some(0x00));
        assert_eq!(cartridge.read(0x006001), Some(0x84));
        assert!(cartridge.write(0x006000, 0x77));
        assert_eq!(cartridge.read(0x006000), Some(0x77));
        assert_eq!(cartridge.read(0x806001), Some(0x84));
        assert_eq!(cartridge.read(0xC08000), Some(0xEA));
    }
}
