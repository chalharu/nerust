//! Read-only session queries for frontends: metrics, titles, slots,
//! and history views. No mutation, no orchestration.
//!
//! Tick discipline: [`SessionHandle::tick_metrics`] is the single
//! metrics lock per pass. Callers fetch one [`TickMetrics`] and thread
//! it through these queries instead of combining per-call predicates.

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
        self.emu
            .core()
            .ok()
            .map(|c| c.metrics())
            .unwrap_or_default()
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
        let name = self.active_factory().map(|f| f.display_name());
        window_title(
            tick.paused,
            tick.loaded,
            name,
            self.active_slot_id(),
            self.slots().len(),
        )
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
