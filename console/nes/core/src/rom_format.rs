#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum RomFormat {
    INes,
    Nes20,
}

impl RomFormat {
    pub const fn label(self) -> &'static str {
        match self {
            Self::INes => "iNES",
            Self::Nes20 => "NES 2.0",
        }
    }
}
