#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Copy, Clone)]
pub struct LogicalSize {
    pub width: usize,
    pub height: usize,
}
