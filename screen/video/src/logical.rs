#[derive(serde::Serialize, serde::Deserialize, Debug, Copy, Clone)]
pub struct LogicalSize {
    pub width: usize,
    pub height: usize,
}
