use nerust_core_traits::save_state::load_state_from_header;
use nerust_emu_thread::EmuThread;
use nerust_render_traits::FrameBuffer;

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
        compact_rgba(&guard)?
    };
    drop(guard);
    Some(PreviewFrame {
        width: w as u32,
        height: h as u32,
        rgba,
    })
}

fn compact_rgba(frame: &FrameBuffer) -> Option<Vec<u8>> {
    let row_bytes = frame.width().checked_mul(4)?;
    let source_len = frame.stride().checked_mul(frame.height())?;
    let source = frame.as_ref().get(..source_len)?;
    if frame.stride() < row_bytes {
        return None;
    }

    let mut rgba = Vec::with_capacity(row_bytes.checked_mul(frame.height())?);
    for row in source.chunks_exact(frame.stride()) {
        rgba.extend_from_slice(&row[..row_bytes]);
    }
    Some(rgba)
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

#[cfg(test)]
mod tests {
    use nerust_render_traits::{FrameBuffer, PixelFormat};

    use super::compact_rgba;

    #[test]
    fn compact_rgba_removes_aligned_row_padding() {
        let mut frame = FrameBuffer::with_capacity(3, 2, PixelFormat::Rgba);
        frame.resize(3, 2);
        assert!(frame.stride() > 3 * 4);
        frame.as_mut().fill(0xEE);
        frame.as_mut()[..12].copy_from_slice(&[1; 12]);
        let second_row = frame.stride();
        frame.as_mut()[second_row..second_row + 12].copy_from_slice(&[2; 12]);

        assert_eq!(compact_rgba(&frame), Some([[1; 12], [2; 12]].concat()));
    }
}
