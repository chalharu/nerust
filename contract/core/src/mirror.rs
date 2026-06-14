#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
pub enum MirrorMode {
    Horizontal,
    Vertical,
    Single0,
    Single1,
    Four,
    Custom([u8; 4]),
}

impl MirrorMode {
    pub fn try_from(mode: u8) -> Result<MirrorMode, &'static str> {
        match mode {
            0 => Ok(MirrorMode::Horizontal),
            1 => Ok(MirrorMode::Vertical),
            2 => Ok(MirrorMode::Single0),
            3 => Ok(MirrorMode::Single1),
            4 => Ok(MirrorMode::Four),
            _ => Err("parse error"),
        }
    }
}
