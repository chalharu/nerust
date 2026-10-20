//! Paused system-image cache for the debugger PPU viewer.
//! Pure data types only; behavior lives on the core in the parent module.

/// One live PPU image: label plus packed RGBA bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveImage {
    /// Image label.
    pub label: String,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Packed RGBA bytes.
    pub rgba: Vec<u8>,
}

impl From<(String, u32, u32, Vec<u8>)> for LiveImage {
    fn from(tuple: (String, u32, u32, Vec<u8>)) -> Self {
        Self {
            label: tuple.0,
            width: tuple.1,
            height: tuple.2,
            rgba: tuple.3,
        }
    }
}

/// Live PPU image batch: generation plus one [`LiveImage`] per image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveImageBatch {
    /// Source generation; viewers skip re-deriving known generations.
    pub generation: u64,
    /// Decoded images.
    pub images: Vec<LiveImage>,
}

/// Live PPU images for one poll: `None` when imageless or skipped.
pub type LiveImages = Option<LiveImageBatch>;
