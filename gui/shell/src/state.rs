use nerust_core_traits::save_state::load_state_from_header;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Pre-Phase-7b save-state format: no longer written, still loadable.
/// Only `core_state` is extracted; other fields use serde defaults.
#[derive(serde::Deserialize)]
struct ConsoleStatePayload {
    #[serde(default)]
    core_state: Vec<u8>,
}

/// Resolve a save state blob to raw core bytes.
/// Tries: SaveStateHeader → ConsoleStatePayload (old format) → raw bytes.
pub fn resolve_state_format(bytes: &[u8]) -> Vec<u8> {
    match load_state_from_header(bytes) {
        Ok(inner) => inner.to_vec(),
        Err(_) => match rmp_serde::from_slice::<ConsoleStatePayload>(bytes) {
            Ok(payload) if !payload.core_state.is_empty() => payload.core_state,
            _ => bytes.to_vec(),
        },
    }
}
