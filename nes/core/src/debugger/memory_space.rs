use nerust_core_traits::memory_space::MemorySpace;
use strum::{Display, EnumIter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumIter)]
pub enum NesMemorySpace {
    #[strum(serialize = "cpu")]
    Cpu,
    #[strum(serialize = "ppu")]
    Ppu,
    #[strum(serialize = "oam")]
    Oam,
    #[strum(serialize = "palette")]
    Palette,
    #[strum(serialize = "save")]
    Save,
}

impl MemorySpace for NesMemorySpace {
    fn id(&self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Ppu => "ppu",
            Self::Oam => "oam",
            Self::Palette => "palette",
            Self::Save => "save",
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Cpu => "CPU Bus",
            Self::Ppu => "PPU Bus",
            Self::Oam => "Sprite Table",
            Self::Palette => "Color Palette",
            Self::Save => "Cartridge RAM",
        }
    }

    fn address_bits(&self) -> u8 {
        match self {
            Self::Cpu => 16,
            Self::Ppu => 14,
            Self::Oam => 8,
            Self::Palette => 5,
            Self::Save => 13,
        }
    }
}
