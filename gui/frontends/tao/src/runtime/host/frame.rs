//! One capture per entry point: settings, tick, slots, shortcuts, plus the OSD frame.
//! Static chrome is `Arc`-cached; per-tick liveness is read off the session, never cloned.

use std::sync::Arc;

/// One capture per pass: static chrome shared by `Arc`, dynamic tick read live.
pub(crate) struct FrameCtx {
    chrome: Arc<ChromeSnapshot>,
    tick: nerust_gui_shell::session::osd::TickMetrics,
}

struct ChromeSnapshot {
    generation: u64,
    settings: Arc<nerust_gui_runtime::settings::SettingsSnapshot>,
    system: Option<Box<dyn nerust_core_traits::identity::SystemId>>,
    slots: Arc<[nerust_persistence::model::StateSlotSummary]>,
    shortcuts: Arc<[nerust_settings_core::model::input::ShortcutBinding]>,
    frame: FrameSnapshot,
}

impl FrameCtx {
    /// Pass generation for trace logging.
    #[cfg(test)]
    pub(super) fn generation(&self) -> u64 {
        self.chrome.generation
    }

    /// Settings snapshot for chrome and target-size math.
    pub(super) fn settings(&self) -> &nerust_gui_runtime::settings::SettingsSnapshot {
        &self.chrome.settings
    }

    /// Tick liveness for pause/load and title gates.
    pub(crate) fn tick(&self) -> nerust_gui_shell::session::osd::TickMetrics {
        self.tick
    }

    /// Immutable slot summaries for the menu identity gate.
    pub(super) fn slots(&self) -> &Arc<[nerust_persistence::model::StateSlotSummary]> {
        &self.chrome.slots
    }

    /// Immutable shortcut bindings for the menu identity gate.
    pub(super) fn shortcuts(&self) -> &Arc<[nerust_settings_core::model::input::ShortcutBinding]> {
        &self.chrome.shortcuts
    }

    /// OSD frame slice: notifications plus language.
    pub(super) fn frame(&self) -> &FrameSnapshot {
        &self.chrome.frame
    }
}

/// One settings snapshot per entry point, threaded by reference.
#[derive(Debug, Clone)]
pub(crate) struct FrameSnapshot(Arc<nerust_gui_runtime::settings::SettingsSnapshot>);

impl FrameSnapshot {
    pub(crate) fn notifications(
        &self,
    ) -> &nerust_settings_core::model::local::NotificationSettings {
        &self.0.local.notifications
    }

    pub(crate) fn language(&self) -> nerust_settings_core::model::language::AppLanguage {
        self.0.shared.general.language
    }
}

/// Versioned frame cache: hot passes reuse the cached `Arc`.
pub(super) struct FrameCache {
    entry: Option<Arc<ChromeSnapshot>>,
    generation: u64,
}

impl FrameCache {
    pub(super) fn new() -> Self {
        Self {
            entry: None,
            generation: 0,
        }
    }

    /// Fresh metrics per pass; only immutable chrome is cached.
    pub(super) fn frame_ctx(
        &mut self,
        session: &nerust_gui_shell::session::SessionHandle,
    ) -> FrameCtx {
        self.capture(
            session.settings_arc(),
            session.slots_arc(),
            session.active_system_id(),
            session.tick_metrics(),
        )
    }

    fn capture(
        &mut self,
        settings: Arc<nerust_gui_runtime::settings::SettingsSnapshot>,
        slots: Arc<[nerust_persistence::model::StateSlotSummary]>,
        system: Option<&dyn nerust_core_traits::identity::SystemId>,
        tick: nerust_gui_shell::session::osd::TickMetrics,
    ) -> FrameCtx {
        // Owners retain these allocations until content changes. Keeping the old
        // Arcs alive also prevents allocation-address reuse (ABA).
        let cached = self.entry.as_ref().filter(|cached| {
            Arc::ptr_eq(&cached.settings, &settings)
                && Arc::ptr_eq(&cached.slots, &slots)
                && cached.system.as_deref() == system
        });
        let chrome = if let Some(cached) = cached {
            Arc::clone(cached)
        } else {
            let shortcuts = self
                .entry
                .as_ref()
                .and_then(|cached| {
                    Arc::ptr_eq(&cached.settings, &settings).then(|| Arc::clone(&cached.shortcuts))
                })
                .unwrap_or_else(|| settings.shared.input.shortcuts.keyboard.clone().into());
            self.generation += 1;
            let fresh = Arc::new(ChromeSnapshot {
                generation: self.generation,
                frame: FrameSnapshot(Arc::clone(&settings)),
                settings,
                system: system.map(|id| id.clone_box()),
                slots,
                shortcuts,
            });
            log::trace!("frame cache miss: generation {}", fresh.generation);
            self.entry = Some(Arc::clone(&fresh));
            fresh
        };
        FrameCtx { chrome, tick }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::host::TestHost;

    #[test]
    fn frame_cache_reuses_chrome_with_fresh_tick() {
        let support = TestHost::new();
        let mut cache = FrameCache::new();
        let first = cache.frame_ctx(support.host.session());
        let tick = nerust_gui_shell::session::osd::TickMetrics {
            fps: 59.5,
            frame_counter: first.tick().frame_counter + 1,
            loaded: true,
            paused: true,
        };
        let second = cache.capture(
            Arc::clone(&first.chrome.settings),
            Arc::clone(first.slots()),
            None,
            tick,
        );
        assert!(Arc::ptr_eq(&first.chrome, &second.chrome));
        assert_eq!(second.tick().frame_counter, tick.frame_counter);
        assert_eq!(second.tick().fps, tick.fps);
        assert!(second.tick().loaded);
        assert!(second.tick().paused);
        assert!(!first.tick().loaded);
    }

    #[test]
    fn frame_cache_reuses_arc_on_hot_path() {
        let support = TestHost::new();
        let mut host = support.host;
        let first = host.frame_ctx();
        let second = host.frame_ctx();
        assert!(Arc::ptr_eq(&first.chrome, &second.chrome));
        assert_eq!(first.generation(), second.generation());
    }

    #[test]
    fn frame_cache_bumps_on_settings_change() {
        let support = TestHost::new();
        let mut host = support.host;
        let first = host.frame_ctx();
        let mut snapshot = host.session().settings_snapshot().clone();
        snapshot.shared.general.language =
            nerust_settings_core::model::language::AppLanguage::Japanese;
        host.apply_settings(snapshot)
            .expect("headless settings apply");
        let second = host.frame_ctx();
        assert!(!Arc::ptr_eq(&first.chrome, &second.chrome));
        assert!(second.generation() > first.generation());
    }

    #[test]
    fn frame_cache_tracks_notification_position_color_duration_and_area() {
        use nerust_settings_core::model::local::{
            NotificationAreaMode, NotificationColor, NotificationPosition, NotificationSettings,
        };
        let changes: [fn(&mut NotificationSettings); 4] = [
            |settings| settings.position = NotificationPosition::TopRight,
            |settings| settings.info_color = NotificationColor::Cyan,
            |settings| settings.info_duration_ms += 1,
            |settings| settings.area_mode = NotificationAreaMode::Strip,
        ];
        let support = TestHost::new();
        let mut host = support.host;
        for change in changes {
            let first = host.frame_ctx();
            let mut settings = first.settings().clone();
            change(&mut settings.local.notifications);
            host.apply_settings(settings.clone())
                .expect("headless settings apply");
            let second = host.frame_ctx();
            assert!(!Arc::ptr_eq(&first.chrome, &second.chrome));
            assert_eq!(
                second.frame().notifications(),
                &settings.local.notifications
            );
            assert!(Arc::ptr_eq(&second.chrome.settings, &second.frame().0));
        }
    }

    #[test]
    fn frame_cache_tracks_subsecond_slot_replacement_without_rescanning_shortcuts() {
        use nerust_persistence::model::StateSlotSummary;
        let support = TestHost::new();
        let mut cache = FrameCache::new();
        let settings = support.host.session().settings_arc();
        let slot = StateSlotSummary {
            schema_version: 1,
            slot_id: 1,
            path: "state.slot".into(),
            saved_at: std::time::UNIX_EPOCH + std::time::Duration::from_millis(100),
            has_thumbnail: false,
            emulator_version: "test".into(),
        };
        let first = cache.capture(
            Arc::clone(&settings),
            vec![slot.clone()].into(),
            None,
            Default::default(),
        );
        let mut changed = slot;
        changed.saved_at += std::time::Duration::from_millis(1);
        let second = cache.capture(
            settings,
            vec![changed.clone()].into(),
            None,
            Default::default(),
        );
        assert!(second.generation() > first.generation());
        assert_eq!(second.slots()[0].saved_at, changed.saved_at);
        assert!(Arc::ptr_eq(first.shortcuts(), second.shortcuts()));
        // Previously captured immutable chrome remains unchanged.
        assert_ne!(first.slots()[0].saved_at, changed.saved_at);
    }
}
