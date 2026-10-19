//! Read-only session queries for frontends: metrics, titles, slots,
//! and history views. No mutation, no orchestration.

use nerust_emu_thread::ConsoleMetrics;

use crate::session::{SessionHandle, osd::TickMetrics, title::window_title};

/// Window size in logical pixels for frontend surface setup.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WindowSize {
    pub width: f32,
    pub height: f32,
}

impl SessionHandle {
    pub fn metrics(&self) -> ConsoleMetrics {
        self.emu.core_opt().map(|c| c.metrics()).unwrap_or_default()
    }

    pub fn tick_metrics(&self) -> TickMetrics {
        let metrics = self.metrics();
        TickMetrics {
            fps: metrics.emulation_fps,
            paused: metrics.paused,
            loaded: metrics.loaded,
            frame_counter: metrics.frame_counter,
        }
    }

    pub fn window_size(&self) -> WindowSize {
        let Some(core) = self.emu.core_opt() else {
            return WindowSize::default();
        };
        let profile = core.render_profile();
        WindowSize {
            width: profile.physical_size.width,
            height: profile.physical_size.height,
        }
    }

    pub fn window_title(&self) -> String {
        let metrics = self.metrics();
        let name = self.active_factory().map(|f| f.display_name());
        window_title(
            metrics.paused,
            metrics,
            name,
            self.active_slot_id(),
            self.slots().len(),
        )
    }

    pub fn loaded(&self) -> bool {
        self.tick_metrics().loaded
    }

    pub fn paused(&self) -> bool {
        self.tick_metrics().paused
    }

    pub fn can_pause(&self) -> bool {
        self.tick_metrics().can_pause()
    }

    pub fn can_resume(&self) -> bool {
        self.tick_metrics().can_resume()
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

    pub fn clear_notification_history(&mut self) {
        self.notifications.clear_history();
    }

    pub fn slots(&self) -> &[nerust_persistence::model::StateSlotSummary] {
        self.media.persistence.slots()
    }

    pub fn active_slot_id(&self) -> Option<u64> {
        self.media.persistence.active_slot_id()
    }

    pub fn save_hidden_lifecycle_state(&mut self) -> bool {
        self.emu
            .core_opt()
            .is_some_and(|core| self.media.persistence.save_hidden(core))
    }

    pub fn load_hidden_lifecycle_state(&mut self) -> bool {
        self.emu
            .core_opt()
            .is_some_and(|core| self.media.persistence.load_hidden(core))
    }

    pub fn clear_hidden_lifecycle_state(&mut self) {
        self.media.persistence.clear_hidden();
    }
}
