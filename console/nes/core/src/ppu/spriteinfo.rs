#[derive(serde::Serialize, serde::Deserialize, Debug, Copy, Clone, Default)]
pub(crate) struct SpriteInfo {
    pub(crate) low_byte: u8,
    pub(crate) high_byte: u8,
    pub(crate) palette_offset: u8,
    pub(crate) tile_addr: u16,
    pub(crate) horizontal_mirror: bool,
    pub(crate) priority: bool,
    pub(crate) position: u8,
}

impl SpriteInfo {
    pub(crate) fn new() -> Self {
        Self {
            low_byte: 0,
            high_byte: 0,
            palette_offset: 0,
            tile_addr: 0,
            horizontal_mirror: false,
            priority: false,
            position: 0,
        }
    }
}
