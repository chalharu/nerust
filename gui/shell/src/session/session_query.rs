//! Read-only session queries for frontends: metrics, titles, slots, and history views.
//! Callers fetch one `TickMetrics` and thread it through instead of combining predicates.

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
    pub fn tick_metrics(&self) -> TickMetrics {
        // Single lock-free snapshot per pass: pause/load truth rides the
        // stamped atomics, counters read the thread side directly.
        let snapshot = self
            .emu
            .require_live()
            .ok()
            .map(|c| c.tick_snapshot())
            .unwrap_or(crate::emu_core::TickSnapshot {
                fps: 0.0,
                paused: false,
                loaded: false,
                frames: 0,
            });
        TickMetrics {
            fps: snapshot.fps,
            paused: snapshot.paused,
            loaded: snapshot.loaded,
            frame_counter: snapshot.frames,
        }
    }

    pub fn window_size(&self) -> WindowSize {
        let Some(core) = self.emu.require_live().ok() else {
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
    /// One-shot title for cold paths (window creation, tests).
    /// Per-frame callers use `window_title_cached` to avoid reallocating.
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

    /// Owned rows for change-gated dialogs; the per-frame OSD path
    /// uses `notification_history_lines_cached` instead.
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

    pub fn slots_arc(&self) -> std::sync::Arc<[nerust_persistence::model::StateSlotSummary]> {
        self.media.persistence.slots_arc()
    }

    pub fn active_slot_id(&self) -> Option<u64> {
        self.media.persistence.active_slot_id()
    }
}

#[cfg(test)]
mod tests {
    use super::HistoryView;
    use crate::session::notification::{Message, NotificationLevel, UiNotification};
    use nerust_settings_core::model::language::AppLanguage;

    fn event(seq: u64, message: Message) -> UiNotification {
        UiNotification {
            level: NotificationLevel::Info,
            message,
            count: 1,
            seq,
            at: std::time::UNIX_EPOCH,
        }
    }

    #[test]
    fn history_view_rebuilds_only_on_seq_or_language_change() {
        let mut view = HistoryView::new();
        let first = [event(0, Message::paused())];
        let lines = view.lines(first.iter(), 1, AppLanguage::English);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "Paused");
        // Same seq and language: identical borrow, no reformat.
        let len = view.lines(first.iter(), 1, AppLanguage::English).len();
        assert_eq!(len, 1);
        // New event with a new seq: rebuild picks it up.
        let second = [event(0, Message::paused()), event(1, Message::resumed())];
        let lines = view.lines(second.iter(), 2, AppLanguage::English);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].text, "Resumed");
        // Same seq, new language: rebuild renders the new language.
        let lines = view.lines(second.iter(), 2, AppLanguage::Japanese);
        assert!(lines[0].text.contains("一時停止"));
    }
}

#[cfg(test)]
mod agreement_tests {
    use crate::session::session_query::*;

    #[test]
    fn cached_and_vec_histories_agree_on_one_source() {
        use crate::session::notification::{Message, NotificationLevel, UiNotification};
        use nerust_settings_core::model::language::AppLanguage;
        let events = [UiNotification {
            level: NotificationLevel::Info,
            message: Message::paused(),
            count: 1,
            seq: 0,
            at: std::time::UNIX_EPOCH,
        }];
        let direct = super::super::osd::history_lines(events.iter(), AppLanguage::English);
        let mut view = HistoryView::new();
        let cached = view.lines(events.iter(), 1, AppLanguage::English).to_vec();
        assert_eq!(direct, cached);
    }
}

#[cfg(test)]
mod query_cov_tests {
    use crate::session::session_query::*;

    #[test]
    fn coreless_queries_report_empty_defaults() {
        use std::sync::Arc;
        let caps = crate::session::test_util::test_session().capabilities;
        let registry = Arc::new(crate::registry::SystemRegistry::new(vec![]));
        let audio = Arc::new(nerust_core_traits::audio::AudioBackendRegistry::new());
        let session = super::super::SessionHandle::new_ephemeral(caps, registry, audio);
        let tick = session.tick_metrics();
        assert!(!tick.loaded);
        let size = session.window_size();
        assert_eq!((size.width, size.height), (0.0, 0.0));
        assert!(session.window_title(&tick).contains("No ROM"));
        assert!(session.slots().is_empty());
        assert_eq!(session.active_slot_id(), None);
        assert_eq!(session.notification_history_seq(), 0);
        assert_eq!(session.notification_history().count(), 0);
        assert!(
            session
                .notification_history_lines(Default::default())
                .is_empty()
        );
    }

    #[test]
    fn loaded_queries_report_core_values() {
        let session = crate::session::test_util::test_session();
        let tick = session.tick_metrics();
        let size = session.window_size();
        assert!(size.width > 0.0 && size.height > 0.0);
        let _ = session.window_title(&tick);
        let _ = session.notification_history_seq();
        let _ = session.notification_history_lines(Default::default());
    }

    #[test]
    fn window_size_defaults_are_zero() {
        assert_eq!(
            WindowSize::default(),
            WindowSize {
                width: 0.0,
                height: 0.0
            }
        );
    }
}
