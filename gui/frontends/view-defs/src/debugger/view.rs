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

/// One disassembly row: pre-formatted line plus navigation addresses.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DisasmRow {
    pub addr: u32,
    pub is_pc: bool,
    pub text: String,
    pub target: Option<u32>,
}

/// One hex-dump row: address plus owned bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DumpRow {
    pub addr: u32,
    pub bytes: Vec<u8>,
}

/// One register row: pre-formatted text plus write-target name.
/// Names are static register identifiers; only the text is localized.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RegRow {
    pub name: &'static str,
    pub text: String,
}

/// Formatted debugger rows. Frontends render strings and route addresses
/// back as gestures; no formatter or row-type import needed.
#[derive(Debug, Clone, Default)]
pub struct DebuggerRows {
    pub disasm: Vec<DisasmRow>,
    pub dump: Vec<DumpRow>,
    pub changed_addrs: Vec<u32>,
    pub regs: Vec<RegRow>,
    pub status_text: String,
    pub paused: bool,
    pub pending_text: Option<String>,
}

/// One 2x-scaled PPU picture with its display size.
#[derive(Debug, Clone)]
pub struct ScaledPicture {
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub(crate) fn rows_from_display(
    display: &DisplayCache,
    status_text: String,
    pending_text: Option<String>,
) -> DebuggerRows {
    DebuggerRows {
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

pub(crate) fn scaled_from_display(display: &DisplayCache) -> Vec<ScaledPicture> {
    display
        .images
        .iter()
        .map(|image| ScaledPicture {
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

/// Pin-column marker without naming the formatter crate.
/// Mirrors the canonical rule: a pinned PC row keeps blanks because the
/// PC `>` already ships inside [`DisasmRow::text`].
pub fn pin_marker(is_pc: bool, is_pinned: bool) -> &'static str {
    if !is_pc && is_pinned { "@ " } else { "  " }
}
