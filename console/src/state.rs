use nerust_contract_core::options::CoreOptions;
use nerust_contract_core::rom::RomIdentity;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStateExport {
    pub state_blob: Vec<u8>,
    pub preview: Option<PreviewFrame>,
}

/// Console-owned save-state wrapper (old format, retained for backward compatibility).
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct ConsoleStatePayload {
    pub(crate) schema_version: u32,
    #[serde(with = "serde_bytes")]
    pub(crate) core_state: Vec<u8>,
    pub(crate) frame_counter: u64,
    pub(crate) paused: bool,
    #[serde(with = "serde_bytes")]
    pub(crate) controller_state: Vec<u8>,
    pub(crate) rom_identity: RomIdentity,
    pub(crate) options: CoreOptions,
    #[serde(with = "serde_bytes")]
    pub(crate) source_frame: Vec<u8>,
}
