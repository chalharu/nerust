use nerust_screen_logical::LogicalSize;

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Copy, Clone)]
pub struct PhysicalSize {
    pub width: f32,
    pub height: f32,
}

impl From<LogicalSize> for PhysicalSize {
    fn from(value: LogicalSize) -> PhysicalSize {
        PhysicalSize {
            width: value.width as f32,
            height: value.height as f32,
        }
    }
}
