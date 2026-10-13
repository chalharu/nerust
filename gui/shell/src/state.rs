use nerust_core_traits::save_state::load_state_from_header;

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

/// Pre-Phase-7b save-state format. No longer written, but existing
/// archives must remain loadable. Only `core_state` is extracted;
/// all other fields (`rom_identity`, `options`, etc.) are ignored
/// by serde's default unknown-field handling.
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
