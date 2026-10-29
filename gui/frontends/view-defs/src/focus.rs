//! View-side auxiliary kinds and focus-transition policy. The framework's
//! [`FocusEvent`](wakui::focus::FocusEvent) carries only a `resume` flag;
//! this module resolves the concrete kind to that flag and to log tags,
//! and owns the burst-debounce tracker: departures arm, arrivals cancel,
//! burst end commits. Resume decisions read the suspension holds, never a
//! framework back-channel: a hold held means the protocol suspended, so an
//! arrival releases it; a hold absent means nothing to resume.

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

/// Burst-debounce tracker for focus transitions. Departures only arm;
/// arrivals cancel; the caller commits at burst end. Hold manipulation stays
/// with the owner table; this tracks only notification truth.
#[derive(Debug, Default)]
pub struct FocusTracker {
    main_active: bool,
    pending: bool,
}

impl FocusTracker {
    pub fn new() -> Self {
        Self {
            main_active: true,
            pending: false,
        }
    }

    pub fn main_active(&self) -> bool {
        self.main_active
    }

    /// Main-window transition: arrival clears, departure arms.
    pub fn main(&mut self, active: bool) {
        self.main_active = active;
        self.pending = !active;
    }

    /// Auxiliary-window transition: focused arrival clears (the caller
    /// releases the hold only for resuming kinds); departure arms.
    pub fn aux(&mut self, focused: bool) {
        self.pending = !focused;
    }

    /// Commit a still-armed departure. The caller suspends only when content
    /// is present; an unarmed or cancelled burst commits nothing.
    pub fn take_armed(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn departure_arms_and_burst_end_commits_once() {
        let mut tracker = FocusTracker::new();
        tracker.main(false);
        assert!(!tracker.main_active());
        assert!(tracker.take_armed());
        assert!(!tracker.take_armed());
    }

    #[test]
    fn arrival_cancels_pending_regardless_of_order() {
        for arrival_first in [false, true] {
            let mut tracker = FocusTracker::new();
            if arrival_first {
                tracker.main(true);
                tracker.main(false);
                assert!(tracker.take_armed());
            } else {
                tracker.main(false);
                tracker.main(true);
                assert!(!tracker.take_armed());
            }
        }
    }

    #[test]
    fn aux_arrival_cancels_main_departure() {
        let mut tracker = FocusTracker::new();
        tracker.main(false);
        tracker.aux(true);
        assert!(!tracker.take_armed());
    }

    #[test]
    fn aux_departure_arms_for_later_commit() {
        let mut tracker = FocusTracker::new();
        tracker.aux(false);
        assert!(tracker.take_armed());
        tracker.main(true);
        assert!(tracker.main_active());
        assert!(!tracker.take_armed());
    }
}
