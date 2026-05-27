use crate::mapper::{HiRomMapper, LoRomMapper, Mapper, MapperKind};

const COPIER_HEADER_LEN: usize = 512;
const LOROM_HEADER_OFFSET: usize = 0x7FC0;
const LOROM_RESET_VECTOR_OFFSET: usize = 0x7FFC;
const HIROM_HEADER_OFFSET: usize = 0xFFC0;
const HIROM_RESET_VECTOR_OFFSET: usize = 0xFFFC;
const HEADER_TITLE_LEN: usize = 21;
const LOROM_MAP_MODE_MASK: u8 = 0x2F;
const LOROM_MAP_MODE_VALUE: u8 = 0x20;
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

        Ok(Self {
            rom: rom.to_vec().into_boxed_slice(),
            save_ram,
            header,
            mapper,
        })
    }

    fn parse_header(rom: &[u8], has_copier_header: bool) -> Option<(CartridgeHeader, Mapper)> {
        Self::parse_header_at(
            rom,
            has_copier_header,
            LOROM_HEADER_OFFSET,
            LOROM_RESET_VECTOR_OFFSET,
            MapperKind::LoRom,
            Mapper::LoRom(LoRomMapper),
        )
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
        if !Self::supported_map_mode(map_mode, mapper_kind) {
            return None;
        }

        let title_bytes = &rom[header_offset..header_offset + HEADER_TITLE_LEN];
        let title = String::from_utf8_lossy(title_bytes)
            .trim_end_matches(char::from(0))
            .trim_end()
            .to_owned();
        let reset_vector =
            u16::from_le_bytes([rom[reset_vector_offset], rom[reset_vector_offset + 1]]);

        Some((
            CartridgeHeader {
                title,
                map_mode,
                rom_size_code: rom[header_offset + 0x17],
                ram_size_code: rom[header_offset + 0x18],
                reset_vector,
                has_copier_header,
                mapper_kind,
            },
            mapper,
        ))
    }

    fn supported_map_mode(map_mode: u8, mapper_kind: MapperKind) -> bool {
        match mapper_kind {
            MapperKind::LoRom => map_mode & LOROM_MAP_MODE_MASK == LOROM_MAP_MODE_VALUE,
            MapperKind::HiRom => map_mode & HIROM_MAP_MODE_MASK == HIROM_MAP_MODE_VALUE,
        }
    }

    pub fn header(&self) -> &CartridgeHeader {
        &self.header
    }

    pub fn read(&self, address: u32) -> Option<u8> {
        self.mapper.read(&self.rom, &self.save_ram, address)
    }

    pub fn write(&mut self, address: u32, value: u8) -> bool {
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
    use crate::MapperKind;

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

    #[test]
    fn parses_lorom_header_and_supports_copier_header_stripping() {
        let rom = build_lorom();
        let mut with_copier_header = vec![0; 512];
        with_copier_header.extend_from_slice(&rom);

        let cartridge = Cartridge::from_bytes(&with_copier_header).unwrap();

        assert_eq!(cartridge.header().title(), "CPU TEST HEADER");
        assert_eq!(cartridge.header().map_mode(), 0x30);
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
}
