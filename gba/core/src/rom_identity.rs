nerust_core_traits::declare_system_id!(pub GbaSystemId, "gba");

#[derive(Debug, Clone)]
pub struct GbaRomIdentity {
    pub title: String,
    pub game_code: String,
    pub maker_code: String,
}

impl GbaRomIdentity {
    pub fn from_rom(rom: &[u8]) -> Option<Self> {
        if rom.len() < 0xC0 {
            return None;
        }
        let title = String::from_utf8_lossy(&rom[0xA0..0xAC]).trim().to_string();
        let game_code = String::from_utf8_lossy(&rom[0xAC..0xB0]).to_string();
        let maker_code = String::from_utf8_lossy(&rom[0xB0..0xB2]).to_string();
        Some(Self {
            title,
            game_code,
            maker_code,
        })
    }
}
