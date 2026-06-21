use nerust_contract_core::load_state_from_header;
use nerust_contract_core::options::CoreOptions;
use nerust_contract_core::rom::RomIdentity;
use nerust_contract_emuthread::EmuThread;

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
            Ok(payload) => payload.core_state,
            Err(_) => bytes.to_vec(),
        },
    }
}
