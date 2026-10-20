//! Read-only session queries for frontends: metrics, titles, slots, and history views.
//! Callers fetch one `TickMetrics` and thread it through instead of combining predicates.

use nerust_emu_thread::ConsoleMetrics;

use crate::session::{SessionHandle, osd::TickMetrics, title::TitleInput};

/// Window size in logical pixels for frontend surface setup.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WindowSize {
    pub width: f32,
    pub height: f32,
}

/// Per-frame history cache: rebuilds on history advance or language change;
/// otherwise serves the stored rows without reformatting.
#[derive(Debug, Default)]
pub struct HistoryView {
    last_seq: Option<(u64, nerust_settings_core::model::language::AppLanguage)>,
    last_lines: Vec<super::osd::HistoryLine>,
}

impl HistoryView {
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrowed history rows for `history` at `seq`/`language`, from the drained iterator.
    /// The caller passes the current `history_seq`.
    pub fn lines<'a>(
        &mut self,
        history: impl Iterator<Item = &'a super::notification::UiNotification>,
        seq: u64,
        language: nerust_settings_core::model::language::AppLanguage,
    ) -> &[super::osd::HistoryLine] {
        let key = (seq, language);
        if self.last_seq != Some(key) {
            self.last_lines = super::osd::history_lines(history, language);
            self.last_seq = Some(key);
        }
        &self.last_lines
    }
}

impl SessionHandle {
    /// Infra escape hatch: raw console metrics; new code threads one `TickMetrics` per pass.
    /// Kept as a shim: removing it breaks the out-of-crate frontend.
    pub fn metrics(&self) -> ConsoleMetrics {
        self.emu
            .core()
            .ok()
            .map(|c| c.metrics())
            .unwrap_or_default()
    }

    pub fn tick_metrics(&self) -> TickMetrics {
        // Single lock-free snapshot per pass: pause/load truth rides the
        // stamped atomics, counters read the thread side directly.
        let (fps, paused, loaded, frame_counter) = self
            .emu
            .core()
            .ok()
            .map(|c| c.tick_snapshot())
            .unwrap_or((0.0, false, false, 0));
        TickMetrics {
            fps,
            paused,
            loaded,
            frame_counter,
        }
    }

    pub fn window_size(&self) -> WindowSize {
        let Some(core) = self.emu.core().ok() else {
            return WindowSize::default();
        };
        let profile = core.render_profile();
        WindowSize {
            width: profile.physical_size.width,
            height: profile.physical_size.height,
        }
    }

    /// Window title from one already-fetched tick: no metrics lock
    /// here, so per-frame title refreshes share the pass tick.
    pub fn window_title(&self, tick: &TickMetrics) -> String {
        let name = self.active_system_display_name();
        super::title::title(TitleInput {
            paused: tick.paused,
            loaded: tick.loaded,
            system_name: name,
            active_slot: self.active_slot_id(),
            slot_count: self.slots().len(),
        })
    }

    pub fn notification_history(
        &self,
    ) -> impl Iterator<Item = &super::notification::UiNotification> {
        self.notifications.history()
    }

    pub fn notification_history_lines(
        &self,
        language: nerust_settings_core::model::language::AppLanguage,
    ) -> Vec<super::osd::HistoryLine> {
        super::osd::history_lines(self.notifications.history(), language)
    }

    pub fn notification_history_seq(&self) -> u64 {
        self.notifications.history_seq()
    }

    pub fn slots(&self) -> &[nerust_persistence::model::StateSlotSummary] {
        self.media.persistence.slots()
    }

    pub fn active_slot_id(&self) -> Option<u64> {
        self.media.persistence.active_slot_id()
    }
}

#[cfg(test)]
mod tests {
    use super::HistoryView;
    use crate::session::notification::{NotificationKind, NotificationLevel, UiNotification};
    use nerust_settings_core::model::language::AppLanguage;

    fn event(seq: u64, kind: NotificationKind) -> UiNotification {
        UiNotification {
            level: NotificationLevel::Info,
            kind,
            count: 1,
            seq,
            at: std::time::UNIX_EPOCH,
        }
    }

    #[test]
    fn history_view_rebuilds_only_on_seq_or_language_change() {
        let mut view = HistoryView::new();
        let first = [event(0, NotificationKind::Paused)];
        let lines = view.lines(first.iter(), 1, AppLanguage::English);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Paused");
        // Same seq and language: identical borrow, no reformat.
        let len = view.lines(first.iter(), 1, AppLanguage::English).len();
        assert_eq!(len, 1);
        // New event with a new seq: rebuild picks it up.
        let second = [
            event(0, NotificationKind::Paused),
            event(1, NotificationKind::Resumed),
        ];
        let lines = view.lines(second.iter(), 2, AppLanguage::English);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].text, "Resumed");
        // Same seq, new language: rebuild renders the new language.
        let lines = view.lines(second.iter(), 2, AppLanguage::Japanese);
        assert!(lines[0].text.contains("一時停止"));
    }
}
