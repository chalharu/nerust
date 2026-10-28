//! Toolkit-free debugger view DTOs. Frontends render these verbatim and
//! never name shell drain types, viewmodel formatters, or core row types.
//! Formatting lives here, next to the view definitions.

use nerust_gui_shell::session::LiveImages;
use nerust_gui_viewmodel::debugger::{
    DebugImageRgba, DisplayCache, format_disasm_line, format_panels, format_register,
    scale2x_nearest,
};

/// Finished drain transport. Field types stay private to this crate's
/// consumers via inference; frontends forward the struct without naming
/// shell or viewmodel types.
#[derive(Debug)]
pub struct AppliedDrain {
    pub display: DisplayCache,
    pub reselected: Option<(u32, Option<u8>)>,
}

impl AppliedDrain {
    pub fn from_output(output: nerust_gui_shell::debug::DebugDrainOutput) -> Self {
        Self {
            display: output.display,
            reselected: output.reselected,
        }
    }
}

/// Finished live-images transport (`None` when the flight carried nothing).
#[derive(Debug)]
pub struct AppliedLive {
    pub generation: u64,
    pub images: Vec<DebugImageRgba>,
    pub running: bool,
}

impl AppliedLive {
    pub fn from_live(live: LiveImages, running: bool) -> Option<Self> {
        let batch = live?;
        Some(Self {
            generation: batch.generation,
            images: batch
                .images
                .into_iter()
                .map(|image| DebugImageRgba {
                    label: image.label,
                    width: image.width,
                    height: image.height,
                    rgba: image.rgba,
                })
                .collect(),
            running,
        })
    }
}

pub(crate) fn rows_from_display(
    display: &DisplayCache,
    status_text: String,
    pending_text: Option<String>,
) -> wakui::debugger::DebuggerRows {
    use wakui::debugger::{DebuggerRows, DisasmRow, DumpRow, RegRow};
    DebuggerRows {
        watch_text: String::new(),
        freeze_text: String::new(),
        selected_line: String::new(),
        disasm: display
            .disasm_lines
            .iter()
            .map(|line| {
                let marker = nerust_gui_viewmodel::debugger::disasm_marker(line.is_pc, false);
                DisasmRow {
                    addr: line.addr,
                    is_pc: line.is_pc,
                    text: format!("{marker}{}", format_disasm_line(line)),
                    target: line.target,
                }
            })
            .collect(),
        dump: display
            .dump_rows
            .iter()
            .map(|row| DumpRow {
                addr: row.addr,
                bytes: row.bytes[..usize::from(row.valid)].to_vec(),
            })
            .collect(),
        changed_addrs: display.diff.clone(),
        regs: display
            .regs
            .iter()
            .map(|(name, value)| RegRow {
                name,
                text: format_register(name, *value),
            })
            .collect(),
        status_text,
        paused: display.paused,
        pending_text,
    }
}

pub(crate) fn scaled_from_display(display: &DisplayCache) -> Vec<wakui::debugger::ScaledPicture> {
    display
        .images
        .iter()
        .map(|image| wakui::debugger::ScaledPicture {
            label: image.label.clone(),
            width: image.width * 2,
            height: image.height * 2,
            rgba: scale2x_nearest(&image.rgba, image.width, image.height),
        })
        .collect()
}

pub(crate) fn panels_text(display: &DisplayCache) -> String {
    format_panels(&display.panels)
}
