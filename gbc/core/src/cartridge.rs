/// Stub cartridge for Phase 3 compilation.
///
/// Filled in during Phase 4 (Mbc trait, ROM Only, MBC1).
#[derive(Debug, Clone, Default)]
pub struct Cartridge {
    _private: (),
}

impl Cartridge {
    pub fn read_rom(&self, _addr: u16) -> u8 {
        0xFF
    }

    pub fn read_ram(&self, _addr: u16) -> u8 {
        0xFF
    }

    pub fn write_rom(&mut self, _addr: u16, _value: u8) {}

    pub fn write_ram(&mut self, _addr: u16, _value: u8) {}
}
