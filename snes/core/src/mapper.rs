use crate::cartridge::ADDRESS_MASK;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapperKind {
    LoRom,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mapper {
    LoRom(LoRomMapper),
}

impl Mapper {
    pub(crate) fn kind(&self) -> MapperKind {
        match self {
            Self::LoRom(_) => MapperKind::LoRom,
        }
    }

    pub(crate) fn read_rom(&self, rom: &[u8], address: u32) -> Option<u8> {
        match self {
            Self::LoRom(mapper) => mapper.read_rom(rom, address),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct LoRomMapper;

impl LoRomMapper {
    pub(crate) fn read_rom(&self, rom: &[u8], address: u32) -> Option<u8> {
        lorom_rom_index(address, rom.len()).map(|index| rom[index])
    }
}

pub(crate) fn lorom_rom_index(address: u32, rom_len: usize) -> Option<usize> {
    if rom_len == 0 {
        return None;
    }

    let address = address & ADDRESS_MASK;
    let bank = ((address >> 16) & 0xFF) as u8;
    let offset = (address & 0xFFFF) as u16;

    if (0x7E..=0x7F).contains(&bank) {
        return None;
    }

    let page = usize::from(bank & 0x7F);
    let page_offset = match bank {
        0x00..=0x3F | 0x80..=0xBF => {
            if offset < 0x8000 {
                return None;
            }
            usize::from(offset - 0x8000)
        }
        _ => usize::from(offset & 0x7FFF),
    };
    let linear = page * 0x8000 + page_offset;
    Some(linear % rom_len)
}

#[cfg(test)]
mod tests {
    use super::lorom_rom_index;

    #[test]
    fn lorom_banks_and_mirrors_map_into_linear_rom_storage() {
        assert_eq!(lorom_rom_index(0x008000, 0x10000), Some(0x0000));
        assert_eq!(lorom_rom_index(0x00FFFF, 0x10000), Some(0x7FFF));
        assert_eq!(lorom_rom_index(0x018000, 0x10000), Some(0x8000));
        assert_eq!(lorom_rom_index(0x400000, 0x10000), Some(0x0000));
        assert_eq!(lorom_rom_index(0x408000, 0x10000), Some(0x0000));
        assert_eq!(lorom_rom_index(0x708000, 0x400000), Some(0x380000));
        assert_eq!(lorom_rom_index(0x808000, 0x10000), Some(0x0000));
        assert_eq!(lorom_rom_index(0xC18000, 0x10000), Some(0x8000));
        assert_eq!(lorom_rom_index(0x007FFF, 0x10000), None);
        assert_eq!(lorom_rom_index(0x7E8000, 0x400000), None);
    }
}
