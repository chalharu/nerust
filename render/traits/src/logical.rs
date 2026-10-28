#[derive(serde::Serialize, serde::Deserialize, Debug, Copy, Clone, PartialEq, Eq)]
pub struct LogicalSize {
    pub width: usize,
    pub height: usize,
}
