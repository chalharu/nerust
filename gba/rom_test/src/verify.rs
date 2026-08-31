use serde::Deserialize;

#[derive(Debug, Deserialize, Clone, Default)]
pub struct VerifySpec {
    pub memory: Option<Vec<MemoryEntry>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MemoryEntry {
    pub address: String,
    pub value: String,
}
