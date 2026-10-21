//! Shared debugger batches and outcomes; shell does not depend on viewmodel DTOs.
use nerust_core_traits::debugger::{DebugImage, DebugPanel, DisasmLine, HexRow, ImageFormat};
use nerust_settings_core::model::language::AppLanguage;

pub use crate::debug_transaction::{PendingWrite, WriteTransaction};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum StatusMessage {
    #[default]
    Empty,
    Paused,
    Running,
    Resume,
    NothingToWrite,
    NoCore,
    /// An accepted operation lost its reply; it must not be blindly retried.
    OutcomeUnknown,
    Stepped(u64),
    WroteMemory {
        addr: u32,
        value: u64,
    },
    RegisterWrote {
        name: &'static str,
        value: u64,
    },
    PauseFailed(String),
    ResumeFailed(String),
    StepFailed(String),
    WriteRefused(String),
    RegisterRefused(String),
    ThreadFailed(String),
    Local(String),
}

impl StatusMessage {
    pub fn render(&self, language: AppLanguage) -> String {
        use nerust_settings_core::i18n as ui_msg;
        match self {
            Self::Empty => String::new(),
            Self::Paused => ui_msg::text(language, ui_msg::UiText::PausedState).to_string(),
            Self::Running => ui_msg::text(language, ui_msg::UiText::RunningState).to_string(),
            Self::Resume => ui_msg::text(language, ui_msg::UiText::Resume).to_string(),
            Self::NothingToWrite => {
                ui_msg::text(language, ui_msg::UiText::NothingToWrite).to_string()
            }
            Self::NoCore => ui_msg::text(language, ui_msg::UiText::NoCore).to_string(),
            Self::OutcomeUnknown => match language {
                AppLanguage::Japanese => {
                    "操作結果を確認できません。状態を確認してから再操作してください。".into()
                }
                _ => "Operation outcome unknown. Check the current state before retrying.".into(),
            },
            Self::Stepped(count) => ui_msg::stepped(language, *count),
            Self::WroteMemory { addr, value } => ui_msg::wrote_memory(language, *addr, *value),
            Self::RegisterWrote { name, value } => ui_msg::register_wrote(language, name, *value),
            Self::PauseFailed(detail) => ui_msg::pause_failed(language, detail),
            Self::ResumeFailed(detail) => ui_msg::resume_failed(language, detail),
            Self::StepFailed(detail) => ui_msg::step_failed(language, detail),
            Self::WriteRefused(detail) => ui_msg::write_refused(language, detail),
            Self::RegisterRefused(detail) => ui_msg::register_refused(language, detail),
            Self::ThreadFailed(detail) => ui_msg::thread_failed(language, detail),
            Self::Local(text) => text.clone(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct DebugSnapshot {
    pub available: bool,
    pub regs: Vec<(&'static str, u64)>,
    pub dump_rows: Vec<HexRow>,
    pub disasm_lines: Vec<DisasmLine>,
    pub pc: Option<u32>,
    pub panels: Vec<DebugPanel>,
    pub images: Vec<(String, u32, u32, Vec<u8>)>,
    pub images_gen: u64,
    pub watch: Option<u8>,
    pub reg_names: Vec<&'static str>,
}

#[derive(Debug, Clone, Default)]
pub struct DisplayCache {
    pub regs: Vec<(&'static str, u64)>,
    pub dump_rows: Vec<HexRow>,
    pub diff: Vec<u32>,
    pub disasm_lines: Vec<DisasmLine>,
    pub panels: Vec<DebugPanel>,
    pub images: Vec<(String, u32, u32, Vec<u8>)>,
    pub images_gen: u64,
    pub watch_value: Option<u8>,
    pub paused: bool,
    pub reg_names: Vec<&'static str>,
    pub status: StatusMessage,
    pub pending: Option<PendingWrite>,
}

impl DisplayCache {
    pub fn clone_without_images(&self) -> Self {
        Self {
            regs: self.regs.clone(),
            dump_rows: self.dump_rows.clone(),
            diff: self.diff.clone(),
            disasm_lines: self.disasm_lines.clone(),
            panels: self.panels.clone(),
            images: Vec::new(),
            images_gen: self.images_gen,
            watch_value: self.watch_value,
            paused: self.paused,
            reg_names: self.reg_names.clone(),
            status: self.status.clone(),
            pending: self.pending,
        }
    }

    pub fn restore_paused_sections(&mut self, old: &Self) {
        if self.paused {
            return;
        }
        self.dump_rows.clone_from(&old.dump_rows);
        self.regs.clone_from(&old.regs);
        self.reg_names.clone_from(&old.reg_names);
        self.disasm_lines.clone_from(&old.disasm_lines);
        self.panels.clone_from(&old.panels);
        self.diff.clone_from(&old.diff);
    }
}

pub fn diff_rows(prev: &[HexRow], next: &[HexRow]) -> Vec<u32> {
    next.iter()
        .filter(|row| {
            prev.iter().any(|p| {
                p.addr == row.addr
                    && (p.valid != row.valid
                        || p.bytes[..p.valid as usize] != row.bytes[..row.valid as usize])
            })
        })
        .map(|row| row.addr)
        .collect()
}

pub fn rgba_image(image: &DebugImage) -> (String, u32, u32, Vec<u8>) {
    let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
    for &pixel in &image.pixels {
        let rgb = match image.format {
            ImageFormat::Indexed2bpp | ImageFormat::Indexed4bpp => image
                .palette
                .get(pixel as usize)
                .copied()
                .unwrap_or([0, 0, 0]),
        };
        rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 0xFF]);
    }
    (image.label_id.to_string(), image.width, image.height, rgba)
}
