#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Copy, Clone)]
pub(crate) struct TileInfo {
    pub(crate) low_byte: u8,
    pub(crate) high_byte: u8,
    pub(crate) palette_offset: u8,
    pub(crate) tile_addr: u16,
}

impl TileInfo {
    pub(crate) fn new() -> Self {
        Self {
            low_byte: 0,
            high_byte: 0,
            palette_offset: 0,
            tile_addr: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.low_byte = 0;
        self.high_byte = 0;
        self.palette_offset = 0;
        self.tile_addr = 0;
    }
}
