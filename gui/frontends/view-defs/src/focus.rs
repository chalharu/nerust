//! View-side auxiliary kinds and focus-policy mapping. The framework's
//! [`FocusEvent`](wakui::focus::FocusEvent) carries only a `resume` flag;
//! this module resolves the concrete kind to that flag and to log tags.

pub use wakui::focus::{FocusCoordinator, FocusEffect, FocusEvent};

/// Which aux window changed focus. Interactive viewers may report a resume;
/// read-only viewers (settings, history) only cancel a pending pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuxKind {
    Debug,
    Ppu,
    History,
    Settings,
}

impl AuxKind {
    /// Windows whose arrival resumes a system-paused session.
    /// Arrival anywhere else only cancels a pending auto-pause.
    pub fn resume_on_arrival(self) -> bool {
        matches!(self, AuxKind::Debug | AuxKind::Ppu)
    }

    /// Central log tag for every aux window.
    pub fn log_tag(self) -> &'static str {
        match self {
            AuxKind::Debug => "debugger",
            AuxKind::Ppu => "PPU viewer",
            AuxKind::History => "history window",
            AuxKind::Settings => "settings",
        }
    }

    /// Human-readable title for the pending (pre-screen) window.
    pub fn title(self) -> &'static str {
        match self {
            AuxKind::Debug => "Debugger",
            AuxKind::Ppu => "PPU Viewer",
            AuxKind::History => "Notification History",
            AuxKind::Settings => "Settings",
        }
    }
}
