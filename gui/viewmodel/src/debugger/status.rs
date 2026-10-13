//! Debugger outcomes as data: the session reports *what*
//! happened, frontends render wording in their own language.

use nerust_settings_core::model::language::AppLanguage;

use nerust_core_traits::debugger::{DebuggerError, SpaceId, SpaceTable};

/// One debugger outcome, data only. The session and drain report
/// *what* happened; frontends render the wording via
/// [`StatusMessage::render`] in their own language. This keeps
/// presentation (wording, language) out of the application layer:
/// the drain no longer needs a language, and both languages are
/// covered by construction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum StatusMessage {
    /// Nothing to report yet (empty display).
    #[default]
    Empty,
    Paused,
    Running,
    Resume,
    NothingToWrite,
    NoCore,
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
    /// Frontend-local wording (parse errors, selection prompts):
    /// never produced by the session, rendered verbatim.
    Local(String),
}

impl StatusMessage {
    /// Render the outcome in a language. Single wording shared by
    /// every frontend (and its tests).
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

/// A fully staged write awaiting confirm. Rendered by the frontend's
/// confirm row; the drain and any optimistic display share this one
/// shape instead of pre-formatted strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PendingWrite {
    pub addr: u32,
    pub old: Option<u8>,
    pub value: u64,
}

impl PendingWrite {
    /// Confirm-row text in a language. Single formatter shared by
    /// every frontend.
    pub fn render(&self, language: AppLanguage) -> String {
        nerust_settings_core::i18n::pending_write(language, self.addr, self.old, self.value)
    }
}

/// Resolve the display key for a [`DebuggerError`]. `None` for
/// variants without a table entry. Lives outside the kernel: error
/// display is presentation's duty, so `UnknownSpace` never
/// fabricates a key from inside domain code.
pub fn space_key(table: &SpaceTable, error: &DebuggerError) -> Option<&'static str> {
    let id: SpaceId = match error {
        DebuggerError::UnknownSpace(id) | DebuggerError::ReadOnlySpace(id) => *id,
        DebuggerError::UnmappedAddress { space, .. } => *space,
        _ => return None,
    };
    table.get(id).map(|s| s.key)
}
