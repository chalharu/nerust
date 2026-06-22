use nerust_contract_core::load_state_from_header;
use nerust_contract_emuthread::EmuThread;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OperationError {
    #[error("emu thread channel unavailable")]
    WorkerUnavailable,
    #[error("emu thread reply channel closed")]
    NoReply,
    #[error("{0}")]
    Reply(String),
}

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

/// Generate a preview frame from the EmuThread's shared frame buffer.
/// Caller should hold no lock on the shared frame buffer.
pub fn generate_preview(emu: &EmuThread) -> Option<PreviewFrame> {
    let Ok(guard) = emu.shared_frame_buffer().lock() else {
        log::warn!("generate_preview: shared frame buffer lock failed");
        return None;
    };
    let w = guard.width();
    let h = guard.height();
    if w == 0 || h == 0 {
        return None;
    }
    let rgba = if let Some(palette) = guard.palette() {
        let indices = guard.as_ref();
        let mut rgba = Vec::with_capacity(w * h * 4);
        for &idx in indices.iter().take(w * h) {
            let color = palette[idx as usize];
            rgba.push((color >> 24) as u8);
            rgba.push((color >> 16) as u8);
            rgba.push((color >> 8) as u8);
            rgba.push(color as u8);
        }
        rgba
    } else {
        guard.as_ref().to_vec()
    };
    drop(guard);
    Some(PreviewFrame {
        width: w as u32,
        height: h as u32,
        rgba,
    })
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
