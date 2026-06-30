const ADDRESS_MASK: u32 = 0x00FF_FFFF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapperKind {
    LoRom,
    HiRom,
    Sa1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mapper {
    LoRom(LoRomMapper),
    HiRom(HiRomMapper),
    Sa1(Sa1Mapper),
}

impl Mapper {
    pub(crate) fn read(&self, rom: &[u8], ram: &[u8], address: u32) -> Option<u8> {
        self.ram_index(address, ram.len())
            .map(|index| ram[index])
            .or_else(|| self.read_rom(rom, address))
    }

    pub(crate) fn write_ram(&self, ram: &mut [u8], address: u32, value: u8) -> bool {
        let Some(index) = self.ram_index(address, ram.len()) else {
            return false;
        };
        ram[index] = value;
        true
    }

    fn read_rom(&self, rom: &[u8], address: u32) -> Option<u8> {
        match self {
            Self::LoRom(mapper) => mapper.read_rom(rom, address),
            Self::HiRom(mapper) => mapper.read_rom(rom, address),
            Self::Sa1(mapper) => mapper.read_rom(rom, address),
        }
    }

    fn ram_index(&self, address: u32, ram_len: usize) -> Option<usize> {
        match self {
            Self::LoRom(_) => lorom_ram_index(address, ram_len),
            Self::HiRom(_) => hirom_ram_index(address, ram_len),
            Self::Sa1(_) => sa1_ram_index(address, ram_len),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct HiRomMapper;

impl HiRomMapper {
    pub(crate) fn read_rom(&self, rom: &[u8], address: u32) -> Option<u8> {
        hirom_rom_index(address, rom.len()).map(|index| rom[index])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Sa1Mapper;

impl Sa1Mapper {
    pub(crate) fn read_rom(&self, rom: &[u8], address: u32) -> Option<u8> {
        sa1_rom_index(address, rom.len()).map(|index| rom[index])
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

pub(crate) fn hirom_rom_index(address: u32, rom_len: usize) -> Option<usize> {
    if rom_len == 0 {
        return None;
    }

    let address = address & ADDRESS_MASK;
    let bank = ((address >> 16) & 0xFF) as u8;
    let offset = (address & 0xFFFF) as u16;

    if (0x7E..=0x7F).contains(&bank) {
        return None;
    }

    let page = usize::from(bank & 0x3F);
    let page_offset = match bank {
        0x00..=0x3F | 0x80..=0xBF => {
            if offset < 0x8000 {
                return None;
            }
            usize::from(offset)
        }
        _ => usize::from(offset),
    };
    let linear = page * 0x10000 + page_offset;
    Some(linear % rom_len)
}

pub(crate) fn sa1_rom_index(address: u32, rom_len: usize) -> Option<usize> {
    if rom_len == 0 {
        return None;
    }

    let address = address & ADDRESS_MASK;
    let bank = ((address >> 16) & 0xFF) as u8;
    let offset = (address & 0xFFFF) as u16;

    match bank {
        0xC0..=0xFF => {
            let linear = usize::from(bank - 0xC0) * 0x10000 + usize::from(offset);
            Some(linear % rom_len)
        }
        _ => lorom_rom_index(address, rom_len),
    }
}

pub(crate) fn lorom_ram_index(address: u32, ram_len: usize) -> Option<usize> {
    if ram_len == 0 {
        return None;
    }

    let address = address & ADDRESS_MASK;
    let bank = ((address >> 16) & 0xFF) as u8;
    let offset = (address & 0xFFFF) as u16;

    if !matches!(bank, 0x70..=0x7D | 0xF0..=0xFF) || offset >= 0x8000 {
        return None;
    }

    let page = usize::from(bank & 0x0F);
    let linear = page * 0x8000 + usize::from(offset);
    Some(linear % ram_len)
}

pub(crate) fn hirom_ram_index(address: u32, ram_len: usize) -> Option<usize> {
    if ram_len == 0 {
        return None;
    }

    let address = address & ADDRESS_MASK;
    let bank = ((address >> 16) & 0xFF) as u8;
    let offset = (address & 0xFFFF) as u16;

    if !matches!(bank, 0x20..=0x3F | 0xA0..=0xBF) || !(0x6000..=0x7FFF).contains(&offset) {
        return None;
    }

    let page = usize::from(bank & 0x1F);
    let linear = page * 0x2000 + usize::from(offset - 0x6000);
    Some(linear % ram_len)
}

pub(crate) fn sa1_ram_index(address: u32, ram_len: usize) -> Option<usize> {
    if ram_len == 0 {
        return None;
    }

    let address = address & ADDRESS_MASK;
    let bank = ((address >> 16) & 0xFF) as u8;
    let offset = (address & 0xFFFF) as u16;

    let linear = match bank {
        0x00..=0x3F | 0x80..=0xBF if (0x6000..=0x7FFF).contains(&offset) => {
            usize::from(offset - 0x6000)
        }
        0x40..=0x4F => usize::from(bank & 0x0F) * 0x10000 + usize::from(offset),
        _ => return None,
    };
    Some(linear % ram_len)
}

pub(crate) fn superfx_ram_index(address: u32, ram_len: usize) -> Option<usize> {
    if ram_len == 0 {
        return None;
    }

    let address = address & ADDRESS_MASK;
    let bank = ((address >> 16) & 0xFF) as u8;
    let offset = (address & 0xFFFF) as u16;

    let linear = match bank {
        0x00..=0x3F | 0x80..=0xBF if (0x6000..=0x7FFF).contains(&offset) => {
            usize::from(offset - 0x6000)
        }
        0x70..=0x71 | 0xF0..=0xF1 => usize::from(bank & 0x01) * 0x10000 + usize::from(offset),
        _ => return None,
    };
    Some(linear % ram_len)
}

#[cfg(test)]
mod tests {
    use super::{
        hirom_ram_index, hirom_rom_index, lorom_ram_index, lorom_rom_index, sa1_ram_index,
        sa1_rom_index, superfx_ram_index,
    };

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

    #[test]
    fn hirom_banks_and_mirrors_map_into_linear_rom_storage() {
        assert_eq!(hirom_rom_index(0xC00000, 0x20000), Some(0x00000));
        assert_eq!(hirom_rom_index(0xC0FFFF, 0x20000), Some(0x0FFFF));
        assert_eq!(hirom_rom_index(0xC10000, 0x20000), Some(0x10000));
        assert_eq!(hirom_rom_index(0x400000, 0x20000), Some(0x00000));
        assert_eq!(hirom_rom_index(0x408000, 0x20000), Some(0x08000));
        assert_eq!(hirom_rom_index(0x008000, 0x20000), Some(0x08000));
        assert_eq!(hirom_rom_index(0x00FFFF, 0x20000), Some(0x0FFFF));
        assert_eq!(hirom_rom_index(0x808000, 0x20000), Some(0x08000));
        assert_eq!(hirom_rom_index(0x007FFF, 0x20000), None);
        assert_eq!(hirom_rom_index(0x7E8000, 0x20000), None);
    }

    #[test]
    fn lorom_sram_banks_map_to_battery_ram_windows() {
        assert_eq!(lorom_ram_index(0x700000, 0x2000), Some(0x0000));
        assert_eq!(lorom_ram_index(0x701FFF, 0x2000), Some(0x1FFF));
        assert_eq!(lorom_ram_index(0x702000, 0x2000), Some(0x0000));
        assert_eq!(lorom_ram_index(0x710000, 0x10000), Some(0x8000));
        assert_eq!(lorom_ram_index(0xF00000, 0x2000), Some(0x0000));
        assert_eq!(lorom_ram_index(0x7E0000, 0x2000), None);
        assert_eq!(lorom_ram_index(0x708000, 0x2000), None);
    }

    #[test]
    fn hirom_sram_banks_map_to_battery_ram_windows() {
        assert_eq!(hirom_ram_index(0x206000, 0x2000), Some(0x0000));
        assert_eq!(hirom_ram_index(0x207FFF, 0x2000), Some(0x1FFF));
        assert_eq!(hirom_ram_index(0x216000, 0x10000), Some(0x2000));
        assert_eq!(hirom_ram_index(0xA06000, 0x2000), Some(0x0000));
        assert_eq!(hirom_ram_index(0x205FFF, 0x2000), None);
        assert_eq!(hirom_ram_index(0x208000, 0x2000), None);
        assert_eq!(hirom_ram_index(0x406000, 0x2000), None);
    }

    #[test]
    fn sa1_default_super_mmc_rom_banks_map_full_64k_pages() {
        assert_eq!(sa1_rom_index(0x008000, 0x20000), Some(0x00000));
        assert_eq!(sa1_rom_index(0x00FFFF, 0x20000), Some(0x07FFF));
        assert_eq!(sa1_rom_index(0xC00000, 0x20000), Some(0x00000));
        assert_eq!(sa1_rom_index(0xC08000, 0x20000), Some(0x08000));
        assert_eq!(sa1_rom_index(0xC10000, 0x20000), Some(0x10000));
    }

    #[test]
    fn sa1_bwram_banks_map_to_linear_ram_storage() {
        assert_eq!(sa1_ram_index(0x006000, 0x2000), Some(0x0000));
        assert_eq!(sa1_ram_index(0x007FFF, 0x2000), Some(0x1FFF));
        assert_eq!(sa1_ram_index(0x806000, 0x2000), Some(0x0000));
        assert_eq!(sa1_ram_index(0x400000, 0x20000), Some(0x00000));
        assert_eq!(sa1_ram_index(0x40FFFF, 0x20000), Some(0x0FFFF));
        assert_eq!(sa1_ram_index(0x410000, 0x20000), Some(0x10000));
        assert_eq!(sa1_ram_index(0x4F1234, 0x20000), Some(0x11234));
        assert_eq!(sa1_ram_index(0x005FFF, 0x20000), None);
        assert_eq!(sa1_ram_index(0x008000, 0x20000), None);
        assert_eq!(sa1_ram_index(0xC00000, 0x20000), None);
        assert_eq!(sa1_ram_index(0x700000, 0x20000), None);
    }

    #[test]
    fn superfx_game_ram_maps_direct_and_system_windows() {
        assert_eq!(superfx_ram_index(0x700000, 0x2000), Some(0x0000));
        assert_eq!(superfx_ram_index(0x701FFF, 0x2000), Some(0x1FFF));
        assert_eq!(superfx_ram_index(0x702000, 0x2000), Some(0x0000));
        assert_eq!(superfx_ram_index(0x710000, 0x2000), Some(0x0000));
        assert_eq!(superfx_ram_index(0xF00000, 0x2000), Some(0x0000));
        assert_eq!(superfx_ram_index(0x006000, 0x2000), Some(0x0000));
        assert_eq!(superfx_ram_index(0x007FFF, 0x2000), Some(0x1FFF));
        assert_eq!(superfx_ram_index(0x016000, 0x20000), Some(0x0000));
        assert_eq!(superfx_ram_index(0x3F7FFF, 0x20000), Some(0x1FFF));
        assert_eq!(superfx_ram_index(0x806000, 0x20000), Some(0x0000));
        assert_eq!(superfx_ram_index(0xBF7FFF, 0x20000), Some(0x1FFF));
        assert_eq!(superfx_ram_index(0x710000, 0x20000), Some(0x10000));
        assert_eq!(superfx_ram_index(0x005FFF, 0x2000), None);
    }
}
