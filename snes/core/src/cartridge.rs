use crate::mapper::{LoRomMapper, Mapper, MapperKind};

pub(crate) const ADDRESS_MASK: u32 = 0x00FF_FFFF;
const COPIER_HEADER_LEN: usize = 512;
const LOROM_HEADER_OFFSET: usize = 0x7FC0;
const LOROM_RESET_VECTOR_OFFSET: usize = 0x7FFC;
const HEADER_TITLE_LEN: usize = 21;
const LOROM_MAP_MODE_MASK: u8 = 0x2F;
const LOROM_MAP_MODE_VALUE: u8 = 0x20;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CartridgeError {
    #[error(
        "ROM size must be an even multiple of 32 KiB, optionally plus a 512-byte copier header"
    )]
    InvalidRomSize,
    #[error("ROM is too small to contain a LoROM header")]
    MissingHeader,
    #[error("unsupported SNES map mode 0x{0:02X}")]
    UnsupportedMapMode(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CartridgeHeader {
    title: String,
    map_mode: u8,
    rom_size_code: u8,
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
    header: CartridgeHeader,
    mapper: Mapper,
}

impl Cartridge {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CartridgeError> {
        let (rom, has_copier_header) = strip_copier_header(bytes)?;
        if rom.len() <= LOROM_RESET_VECTOR_OFFSET + 1 {
            return Err(CartridgeError::MissingHeader);
        }

        let map_mode = rom[LOROM_HEADER_OFFSET + 0x15];
        if map_mode & LOROM_MAP_MODE_MASK != LOROM_MAP_MODE_VALUE {
            return Err(CartridgeError::UnsupportedMapMode(map_mode));
        }

        let title_bytes = &rom[LOROM_HEADER_OFFSET..LOROM_HEADER_OFFSET + HEADER_TITLE_LEN];
        let title = String::from_utf8_lossy(title_bytes)
            .trim_end_matches(char::from(0))
            .trim_end()
            .to_owned();
        let reset_vector = u16::from_le_bytes([
            rom[LOROM_RESET_VECTOR_OFFSET],
            rom[LOROM_RESET_VECTOR_OFFSET + 1],
        ]);

        Ok(Self {
            rom: rom.to_vec().into_boxed_slice(),
            header: CartridgeHeader {
                title,
                map_mode,
                rom_size_code: rom[LOROM_HEADER_OFFSET + 0x17],
                reset_vector,
                has_copier_header,
                mapper_kind: MapperKind::LoRom,
            },
            mapper: Mapper::LoRom(LoRomMapper),
        })
    }

    pub fn header(&self) -> &CartridgeHeader {
        &self.header
    }

    pub fn read(&self, address: u32) -> Option<u8> {
        self.mapper.read_rom(&self.rom, address)
    }

    pub fn rom_len(&self) -> usize {
        self.rom.len()
    }

    pub(crate) fn mapper_kind(&self) -> MapperKind {
        self.mapper.kind()
    }
}

fn strip_copier_header(bytes: &[u8]) -> Result<(&[u8], bool), CartridgeError> {
    match bytes.len() % 0x8000 {
        0 => Ok((bytes, false)),
        COPIER_HEADER_LEN => Ok((&bytes[COPIER_HEADER_LEN..], true)),
        _ => Err(CartridgeError::InvalidRomSize),
    }
}

#[cfg(test)]
mod tests {
    use super::{Cartridge, CartridgeError};
    use crate::MapperKind;

    const HEADER_OFFSET: usize = 0x7FC0;
    const RESET_VECTOR_OFFSET: usize = 0x7FFC;

    fn build_lorom() -> Vec<u8> {
        let mut rom = vec![0; 0x10000];
        rom[HEADER_OFFSET..HEADER_OFFSET + 21].copy_from_slice(b"CPU TEST HEADER      ");
        rom[0x7FD5] = 0x30;
        rom[0x7FD7] = 0x08;
        rom[RESET_VECTOR_OFFSET..RESET_VECTOR_OFFSET + 2]
            .copy_from_slice(&0x8000_u16.to_le_bytes());
        rom[0x0000] = 0xEA;
        rom[0x8000] = 0xA2;
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
        assert_eq!(cartridge.header().reset_vector(), 0x8000);
        assert!(cartridge.header().has_copier_header());
        assert_eq!(cartridge.header().mapper_kind(), MapperKind::LoRom);
        assert_eq!(cartridge.read(0x008000), Some(0xEA));
        assert_eq!(cartridge.read(0x018000), Some(0xA2));
        assert_eq!(cartridge.read(0x808000), Some(0xEA));
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
